//! A matchmaking connection: the socket loop, and the sweeper that ends silent consoles.

use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{ids::ConnId, sync::lock};

use super::{
    Push, Writer,
    codec::{MAX_INBOUND_PAYLOAD, REJECT, VERSION, encode, read_frame},
    deliver,
    handlers::{Client, ClientState, Refusal, Registry, Reject, handle_frame},
    roster::{ConsoleKey, Members},
};

pub(super) const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

const SWEEP_INTERVAL: Duration = Duration::from_secs(15);

/// No heartbeat (every 15 s) for this long: the console is gone and its grace starts.
pub(super) const STALE_THRESHOLD: Duration = Duration::from_secs(45);

/// A connection's socket: frames go out whole, one at a time.
struct SocketWriter(Mutex<TcpStream>);

impl Writer for SocketWriter {
    fn write_frame(&self, frame: &[u8]) -> io::Result<()> {
        lock(&self.0).write_all(frame)
    }
}

/// Matchmaking on `listener`: the roster in `members`, every joining player recorded in
/// `registry`.
pub fn serve(
    listener: TcpListener,
    members: Arc<Mutex<Members>>,
    registry: Registry,
) -> io::Result<()> {
    serve_with_read_timeout(listener, members, registry, READ_IDLE_TIMEOUT)
}

pub(super) fn serve_with_read_timeout(
    listener: TcpListener,
    members: Arc<Mutex<Members>>,
    registry: Registry,
    read_timeout: Duration,
) -> io::Result<()> {
    spawn_sweeper(Arc::clone(&members));
    tracing::info!(version = VERSION, "matchmaking listener started");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let members = Arc::clone(&members);
                let registry = registry.clone();
                std::thread::spawn(move || {
                    match handle_connection(stream, members, registry, read_timeout) {
                        Ok(()) => {}
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                            ) =>
                        {
                            tracing::debug!(%error, "matchmaking client dropped");
                        }
                        Err(error) => tracing::warn!(%error, "matchmaking connection failed"),
                    }
                });
            }
            Err(error) => tracing::warn!(%error, "matchmaking accept failed"),
        }
    }
    Ok(())
}

fn handle_connection(
    mut stream: TcpStream,
    members: Arc<Mutex<Members>>,
    registry: Registry,
    read_timeout: Duration,
) -> io::Result<()> {
    stream.set_read_timeout(Some(read_timeout))?;
    let peer = stream.peer_addr().ok().map(|addr| addr.ip());
    let writer = Arc::new(SocketWriter(Mutex::new(stream.try_clone()?)));
    let mut client = Client::connected(writer, registry);
    let result = serve_client(&mut stream, &members, &mut client, peer);
    if matches!(result, Ok(Closing::Rejected)) {
        close_after_reject(&mut stream);
    }
    let why = if result.is_ok() {
        "connection closed"
    } else {
        "connection failed"
    };
    let state = std::mem::replace(&mut client.state, ClientState::Connected);
    if let ClientState::Joined { console, .. } = state {
        let pushes = lock(&members).disconnect(&console, client.owner, why, Instant::now());
        deliver(pushes);
    }
    result.map(|_| ())
}

/// How a connection ended without an I/O error.
enum Closing {
    ByThePeer,
    Rejected,
}

/// Close after a REJECT without a reset: closing with the client's unread bytes still queued
/// (the rest of a frame of another version) makes the kernel send a reset, which can destroy the
/// REJECT before the client reads it. Stop writing, then drain what the client sent, briefly.
fn close_after_reject(stream: &mut TcpStream) {
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut drained = 0;
    let mut buf = [0; 1024];
    while drained < MAX_INBOUND_PAYLOAD {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(read) => drained += read,
        }
    }
}

fn serve_client(
    stream: &mut TcpStream,
    members: &Mutex<Members>,
    client: &mut Client,
    peer: Option<std::net::IpAddr>,
) -> io::Result<Closing> {
    loop {
        let Some(frame) = read_frame(stream)? else {
            return Ok(Closing::ByThePeer);
        };
        let kind = frame.kind;
        let reply = if frame.version == VERSION {
            handle_frame(frame, members, client)
        } else {
            tracing::warn!(
                ?peer,
                received = frame.version,
                version = VERSION,
                "matchmaking frame of another protocol version"
            );
            Err(Refusal::Reject(Reject::Version))
        };
        match reply {
            Ok(Some(reply)) => client.writer.write_frame(&reply)?,
            Ok(None) => {}
            Err(Refusal::Reject(reason)) => {
                tracing::warn!(?peer, ?reason, "matchmaking REJECT");
                client
                    .writer
                    .write_frame(&encode(REJECT, &[reason as u8]))?;
                return Ok(Closing::Rejected);
            }
            Err(Refusal::Malformed { reject, error }) => {
                tracing::warn!(?peer, kind, %error, "matchmaking frame dropped");
                if let Some(reason) = reject {
                    client
                        .writer
                        .write_frame(&encode(REJECT, &[reason as u8]))?;
                    return Ok(Closing::Rejected);
                }
                return Err(io::Error::new(io::ErrorKind::InvalidData, error));
            }
        }
        if let Some(ip) = peer
            && let ClientState::Joined { players, .. } = &client.state
        {
            lock(members).note_console_players(ip, players);
        }
    }
}

fn spawn_sweeper(members: Arc<Mutex<Members>>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(SWEEP_INTERVAL);
            let pushes = sweep(&mut lock(&members), Instant::now(), STALE_THRESHOLD);
            deliver(pushes);
        }
    });
}

/// Consoles silent for `stale_threshold` start their grace; reservations past it end.
pub(super) fn sweep(members: &mut Members, now: Instant, stale_threshold: Duration) -> Vec<Push> {
    let stale: Vec<(ConsoleKey, ConnId)> = members
        .consoles
        .iter()
        .filter(|(_, console)| now.duration_since(console.last_heartbeat) > stale_threshold)
        .map(|(key, console)| (key.clone(), console.owner))
        .collect();
    let mut pushes = Vec::new();
    for (key, owner) in stale {
        pushes.extend(members.disconnect(&key, owner, "no heartbeat", now));
    }
    let expired: Vec<ConsoleKey> = members
        .reservations
        .iter()
        .filter(|(_, reservation)| reservation.expires <= now)
        .map(|(key, _)| key.clone())
        .collect();
    for key in expired {
        let reservation = members.reservations.remove(&key).expect("listed");
        tracing::info!(session = %key.session, xuid = key.xuid.0, group = reservation.group.get(), players = reservation.players, "matchmaking reservation expired");
        crate::timeline::record(
            "matchmaking",
            format!(
                "{:016x}'s place in group {} expired",
                key.xuid.0, reservation.group
            ),
            Some(&key.session),
        );
        pushes.extend(members.announce(&key.session, reservation.group, None));
    }
    pushes
}
