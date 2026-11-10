//! Every SBG frame in and out: the recent ones in memory for the admin API, and all of them in a
//! recording file while one is open. Connections only queue their frames, so neither a slow disk
//! nor an admin reading the frames holds up a game.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    fs::{File, OpenOptions},
    io::{self, LineWriter, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::JoinHandle,
};

use super::{
    record::{self, Event, Kind},
    replay,
};
use crate::{
    app::App,
    ids::{ConnId, Xuid},
    matchmaking::Members,
    storage::StorageError,
    sync::lock,
    time::Utc,
};

/// Events waiting for the writer; past this a connection drops its event rather than wait.
const QUEUE_EVENTS: usize = 16_384;
/// Frame bytes waiting for the writer; past this a connection drops its event rather than wait.
const QUEUE_BYTES: usize = 64 * 1024 * 1024;
/// Bytes of recent events kept in memory, frames and bookkeeping.
const RECENT_BYTES: usize = 32 * 1024 * 1024;
/// What one kept event costs besides its frame: the event, the frame's allocation, the slot.
const EVENT_OVERHEAD_BYTES: usize = 2 * std::mem::size_of::<Captured>() + 32;

/// Captured events in the order the writer took them; later events have larger numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CaptureSeq(pub u64);

/// Events in the order connections stamped them, which the writer may take in another order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Stamp(pub u64);

/// One frame in or out, or a connection opening or closing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Captured {
    pub seq: CaptureSeq,
    pub stamp: Stamp,
    /// On the wall clock the logs use.
    pub at: Utc,
    pub conn: ConnId,
    pub peer: Option<IpAddr>,
    /// The console's first player when the writer took the event.
    pub xuid: Option<Xuid>,
    pub kind: Kind,
    /// The raw frame for `In`/`Out`; empty for `Open`/`Close`.
    pub frame: Arc<[u8]>,
    /// Pairs a request with its reply on one connection.
    pub cookie: Option<u16>,
    /// The key a request names; on a reply, the key of its request.
    pub key: Option<Arc<str>>,
    /// Events dropped just before this one, because the writer fell behind.
    pub dropped_before: u64,
}

/// An event on its way to the writer.
struct Queued {
    dropped_before: u64,
    stamp: Stamp,
    at: Utc,
    conn: ConnId,
    peer: Option<IpAddr>,
    kind: Kind,
    frame: Arc<[u8]>,
}

/// What the capture holds now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureStatus {
    pub recent_events: usize,
    pub recent_bytes: usize,
    pub recent_capacity_bytes: usize,
    /// When the oldest kept event happened; `None` when none is kept.
    pub oldest: Option<Utc>,
    /// Events dropped since the server started, because the writer fell behind.
    pub dropped: u64,
    pub recording: Option<RecordingStatus>,
    /// The last recording that stopped on a write error.
    pub failure: Option<RecordingFailure>,
}

/// The recording file being written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingStatus {
    pub path: PathBuf,
    pub started: Utc,
    pub events: u64,
}

/// A recording the writer stopped, and why, for a person to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingFailure {
    pub path: PathBuf,
    pub error: String,
}

/// Why a recording did not start.
#[derive(Debug)]
pub enum StartError {
    /// A recording is open already; stop it first.
    Recording { path: PathBuf },
    /// The storage snapshot for the header could not be read.
    Storage(StorageError),
    /// The storage snapshot does not serialize.
    Snapshot {
        path: PathBuf,
        error: serde_json::Error,
    },
    /// The file could not be created or written.
    Io { path: PathBuf, error: io::Error },
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Recording { path } => write!(f, "already recording to {}", path.display()),
            Self::Storage(error) => write!(f, "storage snapshot: {error}"),
            Self::Snapshot { path, error } => {
                write!(
                    f,
                    "SBG recording {}: storage snapshot: {error}",
                    path.display()
                )
            }
            Self::Io { path, error } => write!(f, "SBG recording {}: {error}", path.display()),
        }
    }
}

impl std::error::Error for StartError {}

#[derive(Default)]
struct Recent {
    events: VecDeque<Captured>,
    bytes: usize,
    next_seq: CaptureSeq,
    dropped: u64,
    /// Requests not answered yet, for the key of their reply.
    asked: HashMap<(ConnId, u16), Arc<str>>,
}

impl Recent {
    /// Keep `captured`, and drop the oldest past `capacity` bytes.
    fn push(&mut self, captured: Captured, capacity: usize) {
        self.bytes += cost(&captured);
        self.events.push_back(captured);
        while self.bytes > capacity
            && let Some(oldest) = self.events.pop_front()
        {
            self.bytes -= cost(&oldest);
        }
    }
}

fn cost(captured: &Captured) -> usize {
    captured.frame.len() + EVENT_OVERHEAD_BYTES
}

struct FileRecording {
    out: LineWriter<File>,
    path: PathBuf,
    started: Utc,
    /// Events stamped before this are before the recording.
    from: Stamp,
    /// The first event not taken yet; earlier ones are not written again.
    next: CaptureSeq,
    events: u64,
    /// Connections the file has opened.
    open: HashSet<ConnId>,
    /// Requests in the file not answered yet.
    asked: HashSet<(ConnId, u16)>,
}

impl FileRecording {
    /// Write `captured` as a replay reads it. A connection opened before the recording gets its
    /// `open` with its first event, and a reply goes in only after its request.
    fn write(&mut self, captured: &Captured) -> io::Result<()> {
        if captured.seq < self.next {
            return Ok(());
        }
        self.next = CaptureSeq(captured.seq.0 + 1);
        if captured.stamp < self.from {
            return Ok(());
        }
        if captured.dropped_before > 0 {
            writeln!(self.out, "# dropped {}", captured.dropped_before)?;
        }
        let pair = captured.cookie.map(|cookie| (captured.conn, cookie));
        let written = match captured.kind {
            Kind::Open => self.open.insert(captured.conn),
            Kind::Close => {
                self.asked.retain(|(conn, _)| *conn != captured.conn);
                self.open.remove(&captured.conn)
            }
            Kind::In => {
                if self.open.insert(captured.conn) {
                    self.line(captured, Kind::Open, &[])?;
                }
                if let Some(pair) = pair {
                    self.asked.insert(pair);
                }
                true
            }
            Kind::Out => pair.is_some_and(|pair| self.asked.remove(&pair)),
        };
        if written {
            self.line(captured, captured.kind, &captured.frame)?;
        }
        Ok(())
    }

    fn line(&mut self, captured: &Captured, kind: Kind, frame: &[u8]) -> io::Result<()> {
        // A clock stepped back past the start records the event at the start.
        let at_ms = u64::try_from(captured.at.unix_ms() - self.started.unix_ms()).unwrap_or(0);
        let event = Event {
            at_ms,
            conn: captured.conn,
            peer: captured.peer,
            xuid: captured.xuid,
            kind,
            frame: frame.to_vec(),
        };
        writeln!(self.out, "{}", record::format_event(&event))?;
        self.events += 1;
        Ok(())
    }

    fn status(&self) -> RecordingStatus {
        RecordingStatus {
            path: self.path.clone(),
            started: self.started,
            events: self.events,
        }
    }
}

#[derive(Default)]
struct Shared {
    recent: Mutex<Recent>,
    recording: Mutex<Option<FileRecording>>,
    failure: Mutex<Option<RecordingFailure>>,
}

pub struct Capture {
    queue: Option<SyncSender<Queued>>,
    next_stamp: AtomicU64,
    /// Events dropped since the last one queued.
    dropped: Arc<AtomicU64>,
    queued_bytes: Arc<AtomicUsize>,
    shared: Arc<Shared>,
    writer: Option<JoinHandle<()>>,
}

impl Capture {
    /// `members` names the first player of each event's console.
    pub fn new(members: Arc<Mutex<Members>>) -> Self {
        let (queue, events) = mpsc::sync_channel(QUEUE_EVENTS);
        let dropped = Arc::new(AtomicU64::new(0));
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let shared = Arc::new(Shared::default());
        let writer = std::thread::Builder::new()
            .name("sbg-capture".into())
            .spawn({
                let queued_bytes = Arc::clone(&queued_bytes);
                let shared = Arc::clone(&shared);
                move || write_events(&events, &shared, &queued_bytes, &members)
            })
            // Without the thread nothing is captured; the game is served all the same.
            .inspect_err(|error| tracing::warn!(%error, "SBG capture not started"))
            .ok();
        Self {
            queue: writer.is_some().then_some(queue),
            next_stamp: AtomicU64::new(0),
            dropped,
            queued_bytes,
            shared,
            writer,
        }
    }

    /// The stamp the next event gets; a recording from here on holds it and every later one.
    pub fn next_stamp(&self) -> Stamp {
        Stamp(self.next_stamp.load(Ordering::SeqCst))
    }

    /// Queue one event, timed now. A full queue drops it, and the file and the status say how
    /// many were lost: capturing must not hold up a game.
    pub fn record(&self, conn: ConnId, peer: Option<IpAddr>, kind: Kind, frame: &[u8]) {
        let Some(queue) = &self.queue else {
            return;
        };
        if self.queued_bytes.load(Ordering::Relaxed) + frame.len() > QUEUE_BYTES {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let queued = Queued {
            dropped_before: self.dropped.swap(0, Ordering::Relaxed),
            stamp: Stamp(self.next_stamp.fetch_add(1, Ordering::SeqCst)),
            at: crate::time::now(),
            conn,
            peer,
            kind,
            frame: frame.into(),
        };
        self.queued_bytes.fetch_add(frame.len(), Ordering::Relaxed);
        if let Err(TrySendError::Full(queued) | TrySendError::Disconnected(queued)) =
            queue.try_send(queued)
        {
            self.queued_bytes.fetch_sub(frame.len(), Ordering::Relaxed);
            self.dropped
                .fetch_add(queued.dropped_before + 1, Ordering::Relaxed);
        }
    }

    /// The newest `limit` kept events that `keep` accepts, oldest first. Read in place, so only
    /// what is returned is copied.
    pub fn newest(&self, limit: usize, keep: impl Fn(&Captured) -> bool) -> Vec<Captured> {
        let recent = lock(&self.shared.recent);
        let mut kept: Vec<Captured> = recent
            .events
            .iter()
            .rev()
            .filter(|event| keep(event))
            .take(limit)
            .cloned()
            .collect();
        kept.reverse();
        kept
    }

    pub fn status(&self) -> CaptureStatus {
        let recent = lock(&self.shared.recent);
        let recent_events = recent.events.len();
        let recent_bytes = recent.bytes;
        let oldest = recent.events.front().map(|event| event.at);
        let dropped = recent.dropped + self.dropped.load(Ordering::Relaxed);
        drop(recent);
        CaptureStatus {
            recent_events,
            recent_bytes,
            recent_capacity_bytes: RECENT_BYTES,
            oldest,
            dropped,
            recording: lock(&self.shared.recording)
                .as_ref()
                .map(FileRecording::status),
            failure: lock(&self.shared.failure).clone(),
        }
    }

    /// Start recording to the file at `path` with `header`. The kept events stamped from
    /// `header.from` on go in first, so a frame the writer took while the file opened is not
    /// lost; later ones follow as the writer takes them.
    pub fn start(
        &self,
        path: &Path,
        existing: ExistingFile,
        header: Header,
    ) -> Result<RecordingStatus, StartError> {
        if let Some(file) = lock(&self.shared.recording).as_ref() {
            return Err(StartError::Recording {
                path: file.path.clone(),
            });
        }
        let io_failed = |error| StartError::Io {
            path: path.to_owned(),
            error,
        };
        let mut options = OpenOptions::new();
        options.write(true);
        match existing {
            ExistingFile::Refuse => options.create_new(true),
            ExistingFile::Replace => options.create(true).truncate(true),
        };
        let out = options.open(path).map_err(io_failed)?;
        // The header can be megabytes of storage: written before the writer is made to wait.
        let started = self.open(path, out, header);
        if started.is_err() {
            let _ = std::fs::remove_file(path);
        }
        started
    }

    fn open(&self, path: &Path, out: File, header: Header) -> Result<RecordingStatus, StartError> {
        let io_failed = |error| StartError::Io {
            path: path.to_owned(),
            error,
        };
        let mut out = LineWriter::new(out);
        write!(out, "{}", record::header(&header.settings, header.started)).map_err(io_failed)?;
        writeln!(out, "# service_host {}", header.service_host).map_err(io_failed)?;
        let storage =
            serde_json::to_string(&header.storage).map_err(|error| StartError::Snapshot {
                path: path.to_owned(),
                error,
            })?;
        writeln!(out, "# storage {storage}").map_err(io_failed)?;
        let mut file = FileRecording {
            out,
            path: path.to_owned(),
            started: header.started,
            from: header.from,
            next: CaptureSeq::default(),
            events: 0,
            open: HashSet::new(),
            asked: HashSet::new(),
        };
        let mut recording = lock(&self.shared.recording);
        if let Some(other) = recording.as_ref() {
            return Err(StartError::Recording {
                path: other.path.clone(),
            });
        }
        // The writer pushes to `recent` before it takes `recording`, so with `recording` held
        // every event it took is in `recent`, and every later one waits for the file.
        {
            let recent = lock(&self.shared.recent);
            for event in &recent.events {
                file.write(event).map_err(io_failed)?;
            }
            file.next = recent.next_seq;
        }
        let status = file.status();
        *recording = Some(file);
        drop(recording);
        tracing::warn!(path = %path.display(), "recording every SBG frame");
        Ok(status)
    }

    /// Close the recording file, if one is open. Events still queued are not in it.
    pub fn stop(&self) -> Option<RecordingStatus> {
        let mut file = lock(&self.shared.recording).take()?;
        // Drops not yet carried by an event are counted in the file and stay in the status.
        let pending = self.dropped.swap(0, Ordering::Relaxed);
        lock(&self.shared.recent).dropped += pending;
        let closed = if pending > 0 {
            writeln!(file.out, "# dropped {pending}")
        } else {
            Ok(())
        }
        .and_then(|()| file.out.flush());
        if let Err(error) = closed {
            tracing::warn!(%error, path = %file.path.display(), "SBG recording close failed");
        }
        tracing::info!(path = %file.path.display(), events = file.events, "SBG recording stopped");
        Some(file.status())
    }
}

impl Drop for Capture {
    /// Write what is queued before the file closes. A server that exits without dropping its
    /// capture loses only what is queued.
    fn drop(&mut self) {
        drop(self.queue.take());
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        self.stop();
    }
}

/// What a recording's header says: what the server runs with, and its data at the start.
pub struct Header {
    pub settings: record::Settings,
    pub started: Utc,
    /// The first event of the recording; taken before the storage snapshot.
    pub from: Stamp,
    pub service_host: String,
    pub storage: crate::storage::Dump,
}

impl Header {
    /// The header for a recording of `app` starting now. A request in flight while the snapshot
    /// is read can be both in the snapshot and in the recording, so a replay of a recording
    /// started under traffic can differ; so can one whose consoles joined matchmaking before it.
    pub fn of(app: &App) -> Result<Self, StorageError> {
        let from = app.capture.next_stamp();
        let now = crate::time::now();
        let schedule = {
            let mut schedule = lock(&app.schedule);
            // A replay serves the recorded shows and their ids, it never plans its own: the
            // coming shows go into storage before the snapshot.
            schedule.shows(now);
            schedule.is_enabled()
        };
        let (options, storage) = {
            let store = lock(&app.store);
            (store.options(), store.storage())
        };
        Ok(Self {
            settings: record::Settings {
                protocol: app.sbg.protocol,
                options,
                server_flow: app.sbg.server_flow,
                schedule,
                served: crate::xsor::ServedConfig::LIVE,
            },
            started: now,
            from,
            service_host: app.sbg.service_host.clone(),
            storage: storage.export()?,
        })
    }
}

/// What a recording does with a file already at its path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExistingFile {
    /// Fail: a recording started at runtime never overwrites one.
    Refuse,
    /// Truncate it: the recording a server starts with replaces the last run's.
    Replace,
}

/// Start recording `app`'s SBG traffic to the file at `path`.
pub fn start_recording(
    app: &App,
    path: &Path,
    existing: ExistingFile,
) -> Result<RecordingStatus, StartError> {
    let header = Header::of(app).map_err(StartError::Storage)?;
    app.capture.start(path, existing, header)
}

/// Take queued events until every sender is gone: name their console and key, keep each, and
/// write it while recording.
fn write_events(
    events: &Receiver<Queued>,
    shared: &Shared,
    queued_bytes: &AtomicUsize,
    members: &Mutex<Members>,
) {
    for queued in events {
        queued_bytes.fetch_sub(queued.frame.len(), Ordering::Relaxed);
        let xuid = queued
            .peer
            .and_then(|peer| lock(members).xuid_for_console(peer));
        let cookie = matches!(queued.kind, Kind::In | Kind::Out)
            .then(|| replay::frame_cookie(&queued.frame))
            .flatten();
        let asked_key = match queued.kind {
            Kind::In => replay::request_key(&queued.frame).map(Arc::from),
            _ => None,
        };
        let captured = {
            let mut recent = lock(&shared.recent);
            let key = match (queued.kind, cookie) {
                (Kind::In, Some(cookie)) => asked_key.inspect(|key| {
                    recent.asked.insert((queued.conn, cookie), Arc::clone(key));
                }),
                (Kind::In, None) => asked_key,
                (Kind::Out, Some(cookie)) => recent.asked.remove(&(queued.conn, cookie)),
                (Kind::Close, _) => {
                    recent.asked.retain(|(conn, _), _| *conn != queued.conn);
                    None
                }
                _ => None,
            };
            let captured = Captured {
                seq: recent.next_seq,
                stamp: queued.stamp,
                at: queued.at,
                conn: queued.conn,
                peer: queued.peer,
                xuid,
                kind: queued.kind,
                frame: queued.frame,
                cookie,
                key,
                dropped_before: queued.dropped_before,
            };
            recent.next_seq = CaptureSeq(recent.next_seq.0 + 1);
            recent.dropped += queued.dropped_before;
            recent.push(captured.clone(), RECENT_BYTES);
            captured
        };
        let mut recording = lock(&shared.recording);
        if let Some(file) = recording.as_mut()
            && let Err(error) = file.write(&captured)
        {
            tracing::warn!(%error, path = %file.path.display(), "SBG recording write failed: recording stopped");
            *lock(&shared.failure) = Some(RecordingFailure {
                path: file.path.clone(),
                error: error.to_string(),
            });
            *recording = None;
        }
    }
}

/// One connection's view of the capture: `open` when created, `close` when dropped, so every way
/// out of the connection loop is captured.
pub struct ConnCapture {
    app: Arc<App>,
    conn: ConnId,
    peer: Option<IpAddr>,
}

impl ConnCapture {
    pub fn open(app: Arc<App>, conn: ConnId, peer: Option<IpAddr>) -> Self {
        let this = Self { app, conn, peer };
        this.event(Kind::Open, &[]);
        this
    }

    pub fn frame_in(&self, frame: &[u8]) {
        self.event(Kind::In, frame);
    }

    pub fn frame_out(&self, frame: &[u8]) {
        self.event(Kind::Out, frame);
    }

    fn event(&self, kind: Kind, frame: &[u8]) {
        self.app.capture.record(self.conn, self.peer, kind, frame);
    }
}

impl Drop for ConnCapture {
    fn drop(&mut self) {
        self.event(Kind::Close, &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(capture: &Capture, events: usize) {
        let begun = std::time::Instant::now();
        while capture.status().recent_events < events {
            assert!(begun.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn header(capture: &Capture, started: Utc) -> Header {
        Header {
            settings: record::Settings {
                protocol: crate::wire::Protocol::V1149,
                options: crate::store::SimulationOptions::default(),
                server_flow: true,
                schedule: false,
                served: crate::xsor::ServedConfig::LIVE,
            },
            started,
            from: capture.next_stamp(),
            service_host: "192.0.2.10".into(),
            storage: crate::storage::Storage::export(&crate::storage::MemoryStorage::default())
                .unwrap(),
        }
    }

    /// A 1149 request frame with `cookie`, as the replay fixtures hold them.
    fn request(cookie: u8) -> Vec<u8> {
        let mut frame = vec![0xff, 0, 0, 0, 0, 0x11, 0, cookie];
        frame.extend_from_slice(&[0, 3, 0xba, 0xc5, 0x0a, 0x44, 0x03, 0x5b, 0x18]);
        frame
    }

    fn reply(cookie: u8) -> Vec<u8> {
        let mut frame = vec![3, 0x83, 0, 0, 0, 0x1a, 0, cookie];
        frame.extend_from_slice(&[0; 18]);
        frame
    }

    #[test]
    fn a_recording_opens_its_connections_and_keeps_only_replies_to_its_requests() {
        let dir = std::env::temp_dir().join(format!("sbg-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("1.sbg");
        let capture = Capture::new(Arc::default());
        let peer: IpAddr = "192.0.2.21".parse().unwrap();
        capture.record(ConnId(1), Some(peer), Kind::Open, &[]);
        // Asked before the recording: its reply is not in the file.
        capture.record(ConnId(1), Some(peer), Kind::In, &request(1));
        wait_for(&capture, 2);
        let started = crate::time::now();
        let head = header(&capture, started);
        // Taken by the writer before the file opens: written from the kept events.
        capture.record(ConnId(1), Some(peer), Kind::In, &request(2));
        wait_for(&capture, 3);
        capture.start(&path, ExistingFile::Refuse, head).unwrap();
        assert!(matches!(
            capture.start(
                &dir.join("2.sbg"),
                ExistingFile::Refuse,
                header(&capture, started)
            ),
            Err(StartError::Recording { .. })
        ));
        assert!(!dir.join("2.sbg").exists());
        capture.record(ConnId(1), Some(peer), Kind::Out, &reply(1));
        capture.record(ConnId(1), Some(peer), Kind::Out, &reply(2));
        wait_for(&capture, 5);
        let stopped = capture.stop().unwrap();
        assert_eq!(stopped.events, 3);
        assert_eq!(capture.stop(), None);
        capture.record(ConnId(1), Some(peer), Kind::Close, &[]);
        let key_of_reply = capture.newest(1, |event| event.kind == Kind::Out);
        assert_eq!(key_of_reply[0].cookie, Some(2));
        drop(capture);

        let recording = record::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(recording.started, started);
        assert_eq!(recording.service_host.as_deref(), Some("192.0.2.10"));
        let kinds: Vec<_> = recording.events.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [Kind::Open, Kind::In, Kind::Out]);
        assert_eq!(recording.events[1].frame, request(2));
        assert_eq!(recording.events[2].frame, reply(2));
        assert_eq!(recording.dropped, 0);
    }

    #[test]
    fn a_startup_recording_replaces_the_last_runs_file_and_a_runtime_one_refuses_it() {
        let path = std::env::temp_dir().join(format!("sbg-existing-{}.sbg", std::process::id()));
        std::fs::write(&path, "last run").unwrap();
        let capture = Capture::new(Arc::default());
        let refused = capture.start(
            &path,
            ExistingFile::Refuse,
            header(&capture, crate::time::now()),
        );
        assert!(matches!(refused, Err(StartError::Io { .. })), "{refused:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "last run");
        capture
            .start(
                &path,
                ExistingFile::Replace,
                header(&capture, crate::time::now()),
            )
            .unwrap();
        capture.stop().unwrap();
        let replaced = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(record::parse(&replaced).is_ok(), "{replaced}");
    }

    #[test]
    fn the_oldest_events_go_past_the_capacity() {
        let mut recent = Recent::default();
        for seq in 0..4 {
            recent.push(
                Captured {
                    seq: CaptureSeq(seq),
                    stamp: Stamp(seq),
                    at: Utc::from_unix_ms(0),
                    conn: ConnId(1),
                    peer: None,
                    xuid: None,
                    kind: Kind::In,
                    frame: Arc::from(&[0u8; 36][..]),
                    cookie: None,
                    key: None,
                    dropped_before: 0,
                },
                2 * (36 + EVENT_OVERHEAD_BYTES) + 1,
            );
        }
        let kept: Vec<u64> = recent.events.iter().map(|event| event.seq.0).collect();
        assert_eq!(kept, [2, 3]);
        assert_eq!(recent.bytes, 2 * (36 + EVENT_OVERHEAD_BYTES));
    }

    #[test]
    fn events_dropped_before_one_are_counted_where_they_were_lost() {
        let path = std::env::temp_dir().join(format!("sbg-dropped-{}.sbg", std::process::id()));
        let capture = Capture::new(Arc::default());
        let settings = header(&capture, Utc::from_unix_ms(1_000)).settings;
        let head = record::header(&settings, Utc::from_unix_ms(1_000));
        std::fs::write(&path, &head).unwrap();
        let mut file = FileRecording {
            out: LineWriter::new(OpenOptions::new().append(true).open(&path).unwrap()),
            path: path.clone(),
            started: Utc::from_unix_ms(1_000),
            from: Stamp(1),
            next: CaptureSeq(1),
            events: 0,
            open: HashSet::new(),
            asked: HashSet::new(),
        };
        let event = |seq, at, kind, dropped_before| Captured {
            seq: CaptureSeq(seq),
            stamp: Stamp(seq),
            at: Utc::from_unix_ms(at),
            conn: ConnId(3),
            peer: None,
            xuid: None,
            kind,
            frame: Arc::from(&[][..]),
            cookie: None,
            key: None,
            dropped_before,
        };
        // Taken already; then two the file holds, one timed before the start.
        file.write(&event(0, 1_200, Kind::Open, 0)).unwrap();
        file.write(&event(1, 1_005, Kind::Open, 0)).unwrap();
        file.write(&event(2, 900, Kind::Close, 4)).unwrap();
        drop(file);
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        insta::assert_snapshot!(text.strip_prefix(head.as_str()).unwrap(), @r"
        5 3 - - open
        # dropped 4
        0 3 - - close
        ");
        assert_eq!(record::parse(&text).unwrap().dropped, 4);
    }
}
