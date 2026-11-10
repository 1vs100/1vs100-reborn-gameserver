//! An SBG connection: the listener, the connection loop with protocol pinning and the replies
//! answered off the connection's thread, and who is calling (`Caller`, `Ctx`).

use std::{
    io::{self, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, SystemTime},
};

use crate::{
    app::App,
    ids::ConnId,
    matchmaking::Members,
    schedule::Schedule,
    store::Store,
    wire::{self, Protocol},
};

use crate::sbg::{
    NEXT_CONN, Settings,
    capture::ConnCapture,
    dispatch::dispatch,
    host::{HostAuthority, LiveConnection},
    message,
    message::{Action, DecodeError, Message, Reply},
};

const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Caller: console address and SBG connection; sockets without a peer are not served.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Caller {
    pub peer: std::net::IpAddr,
    pub conn: ConnId,
}

/// Request context: settings, store, host role, roster, schedule.
#[derive(Clone, Copy)]
pub(crate) struct Ctx<'a> {
    pub settings: &'a Settings,
    pub store: &'a Mutex<Store>,
    pub host: &'a Mutex<HostAuthority>,
    /// Players each console joined with.
    pub members: &'a Mutex<Members>,
    /// Show schedule.
    pub schedule: &'a Mutex<Schedule>,
}

/// The server's requests are served against its `App`.
pub(crate) fn ctx(app: &App) -> Ctx<'_> {
    Ctx {
        settings: &app.sbg,
        store: &app.store,
        host: &app.host,
        members: &app.members,
        schedule: &app.schedule,
    }
}

/// `None` for frames that get no answer (unsupported kinds); malformed requests become a 404.
pub(in crate::sbg) fn decode(frame: &wire::Frame) -> Option<Result<Message, ()>> {
    match message::decode(frame) {
        Ok(message) => Some(Ok(message)),
        Err(DecodeError::Malformed) => Some(Err(())),
        Err(DecodeError::Unsupported(kind)) => {
            tracing::warn!("SBG ignored unsupported frame kind {kind:#04x}");
            None
        }
    }
}

pub(in crate::sbg) fn respond(
    caller: Caller,
    ctx: Ctx,
    cookie: u16,
    message: Result<Message, ()>,
) -> Option<Vec<u8>> {
    let protocol = ctx.settings.protocol;
    let reply = match message {
        Ok(Message::Register) => {
            tracing::debug!(cookie, "SBG register");
            Reply::Register
        }
        Ok(Message::Unregister { controller, xuid }) => {
            tracing::info!(cookie, controller, xuid = %format!("{:016x}", xuid.0), "SBG unregister");
            // Only 1149 sends it (`message::decode`); the server keeps no per-player registration.
            return Some(wire::v1149::unregister_reply(cookie));
        }
        Ok(Message::Heartbeat) => {
            tracing::trace!("SBG heartbeat");
            // 1317 gets no answer; 1149 drops the connection after 60 s without traffic.
            return (protocol == Protocol::V1149).then(|| wire::v1149::heartbeat_ack(cookie));
        }
        Ok(Message::Request {
            key,
            action,
            execute_at,
        }) => {
            let name = action.name();
            let reply = dispatch(caller, ctx, cookie, &key, action, execute_at);
            if reply.is_not_found() {
                // Every kind of request answered NOT_FOUND, not only store reads: a TopN or Sum
                // nobody posted to is as visible here as a missing key.
                tracing::warn!(cookie, key, action = name, "SBG request answered NOT_FOUND");
            }
            reply
        }
        Err(()) => {
            tracing::warn!(cookie, "SBG malformed request answered NOT_FOUND");
            Reply::not_found()
        }
    };
    Some(reply.encode(protocol, cookie))
}

/// SBG over one store; far-future execute times are clock problems, not plans.
const MAX_EXECUTE_HOLD: Duration = Duration::from_secs(30);

/// Wait until `execute_at`, capped; zero for none or past.
pub(in crate::sbg) fn hold_for(execute_at: Option<SystemTime>, now: SystemTime) -> Duration {
    execute_at
        .and_then(|at| at.duration_since(now).ok())
        .map_or(Duration::ZERO, |wait| wait.min(MAX_EXECUTE_HOLD))
}

/// SBG for one title build; other-build frames end their connection.
pub fn serve(listener: TcpListener, app: Arc<App>) -> io::Result<()> {
    serve_with_read_timeout(listener, app, READ_IDLE_TIMEOUT)
}

/// True when the connection opens with HTTP: the ad client, pointed at TCP 1000 by both builds.
fn opens_with_http(stream: &TcpStream) -> io::Result<Option<bool>> {
    let mut first = [0; 1];
    match stream.peek(&mut first)? {
        0 => Ok(None),
        _ => Ok(Some(first[0].is_ascii_uppercase())),
    }
}

pub(in crate::sbg) fn serve_with_read_timeout(
    listener: TcpListener,
    app: Arc<App>,
    read_timeout: Duration,
) -> io::Result<()> {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let app = Arc::clone(&app);
                let peer = stream.peer_addr().ok().map(|a| a.ip());
                std::thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, app, read_timeout) {
                        tracing::warn!(%error, ?peer, "SBG connection failed");
                    }
                });
            }
            Err(error) => tracing::warn!(%error, "SBG accept failed"),
        }
    }
    Ok(())
}

enum Next {
    Frame(wire::Frame),
    ClosedByPeer,
    IdleTimeout,
}

fn next_frame(stream: &mut TcpStream) -> Result<Next, wire::ReadError> {
    match wire::read_one(stream) {
        Ok(frame) => Ok(Next::Frame(frame)),
        Err(wire::ReadError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
            Ok(Next::ClosedByPeer)
        }
        Err(wire::ReadError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(Next::IdleTimeout)
        }
        Err(error) => Err(error),
    }
}

fn handle_connection(stream: TcpStream, app: Arc<App>, read_timeout: Duration) -> io::Result<()> {
    let mut stream = stream;
    stream.set_read_timeout(Some(read_timeout))?;
    match opens_with_http(&stream) {
        Ok(Some(true)) => {
            tracing::debug!("SBG port: HTTP request, answered as ads");
            return crate::http::serve_ads_connection(&stream, &app.sbg.service_host);
        }
        Ok(Some(false)) => {}
        Ok(None) => return Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    }
    let writer = Arc::new(Mutex::new(stream.try_clone()?));
    let peer = match stream.peer_addr() {
        Ok(addr) => addr.ip(),
        Err(error) => {
            // Without an address the console cannot be told apart from the others.
            tracing::warn!(%error, "SBG connection without a peer address closed");
            return Ok(());
        }
    };
    // Open from connect: a reconnecting host keeps its role until its first frame arrives.
    let _live = LiveConnection::open(&app.host, peer);
    let first = match next_frame(&mut stream) {
        Ok(Next::Frame(frame)) => frame,
        // Port scanners: the first bytes after connect are not an SBG frame.
        Ok(Next::ClosedByPeer | Next::IdleTimeout) | Err(wire::ReadError::Header(_)) => {
            tracing::debug!(%peer, "SBG port: first bytes are not an SBG frame, closed");
            return Ok(());
        }
        Err(wire::ReadError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
            ) =>
        {
            tracing::debug!(%peer, %error, "SBG port: reset before a full SBG frame");
            return Ok(());
        }
        Err(wire::ReadError::Io(error)) => return Err(error),
    };
    let caller = Caller {
        peer,
        conn: ConnId(NEXT_CONN.fetch_add(1, Ordering::Relaxed)),
    };
    // Error level, so the connection's lines carry its fields under any filter that enables
    // this module at all.
    let span = tracing::error_span!("sbg", conn = caller.conn.0, peer = %caller.peer);
    let _entered = span.enter();
    tracing::info!("SBG connection opened");
    let capture = Arc::new(ConnCapture::open(
        Arc::clone(&app),
        caller.conn,
        Some(caller.peer),
    ));
    // Failures are logged here, inside the span, so they name the connection.
    let mut pending = Some(first);
    let served = (|| -> io::Result<()> {
        loop {
            let frame = match pending.take() {
                Some(frame) => frame,
                None => match next_frame(&mut stream)? {
                    Next::Frame(frame) => frame,
                    Next::ClosedByPeer => {
                        tracing::info!("SBG connection closed by the console");
                        return Ok(());
                    }
                    Next::IdleTimeout => {
                        tracing::info!(
                            idle_secs = read_timeout.as_secs(),
                            "SBG connection closed: idle timeout"
                        );
                        return Ok(());
                    }
                },
            };
            app.note_sbg_activity();
            capture.frame_in(&frame.to_bytes());
            let cookie = frame.cookie;
            let protocol = app.sbg.protocol;
            if frame.protocol() != protocol {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("SBG {:?} frame on a {protocol:?} server", frame.protocol()),
                ));
            }
            let Some(message) = decode(&frame) else {
                continue;
            };
            if answered_off_thread(&message) {
                // A plain GET may be held (see `game_state_get`, `store_get_with_wait`).
                // Answer it from its own thread so the SETs the title sends meanwhile
                // (the write that releases the hold) are still read from this stream.
                let app = Arc::clone(&app);
                let writer = Arc::clone(&writer);
                let capture = Arc::clone(&capture);
                let span = span.clone();
                std::thread::spawn(move || {
                    let _entered = span.enter();
                    if let Some(response) = respond(caller, ctx(&app), cookie, message) {
                        capture.frame_out(&response);
                        let mut writer = crate::sync::lock(&writer);
                        if let Err(error) = writer.write_all(&response) {
                            tracing::warn!(%error, "SBG deferred reply failed");
                        }
                    }
                });
            } else if let Some(response) = respond(caller, ctx(&app), cookie, message) {
                capture.frame_out(&response);
                let mut writer = crate::sync::lock(&writer);
                writer.write_all(&response)?;
            }
        }
    })();
    if let Err(error) = served {
        tracing::warn!(%error, "SBG connection failed");
    }
    Ok(())
}

/// Requests answered from their own thread: plain GETs and scheduled counter reads.
pub(in crate::sbg) fn answered_off_thread(message: &Result<Message, ()>) -> bool {
    match message {
        Ok(Message::Request {
            action, execute_at, ..
        }) => action.is_deferrable() || (*action == Action::SumGet && execute_at.is_some()),
        _ => false,
    }
}
