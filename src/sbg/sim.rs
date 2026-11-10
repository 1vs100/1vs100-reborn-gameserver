//! SBG server on a virtual clock for replays and tests.
//! Same decode/respond path as live; [`Sim::advance_to`] settles each connection first.
//! One `Sim` per process: process-wide SBG state is shared.

use std::{
    collections::HashMap,
    fmt,
    net::IpAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

use super::{
    Caller, LiveConnection, Message, NEXT_CONN, Settings, answered_off_thread, ctx, decode, respond,
};
use crate::{
    app::App,
    ids::{ConnId, Xuid},
    matchmaking::Members,
    schedule::{Schedule, ScheduleConfig},
    storage::{Storage, StorageError},
    store::{SimulationOptions, Store},
    time::{Busy, Clock, Utc, VirtualClock},
    wire,
};

/// Why a simulation could not be set up or driven.
#[derive(Debug)]
pub enum SimError {
    /// The storage the simulation serves from has no readable show schedule.
    Schedule(StorageError),
    /// The storage the simulation serves from has no readable question bank.
    QuestionBank(StorageError),
    /// A frame arrived for a connection that is not open.
    NotOpen { conn: ConnId },
    /// A frame does not decode.
    Undecodable { conn: ConnId },
    /// A request was received and never answered; no wake-up was left to answer it.
    NeverAnswered,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schedule(error) => write!(f, "the schedule in the storage: {error}"),
            Self::QuestionBank(error) => write!(f, "the question bank in the storage: {error}"),
            Self::NotOpen { conn } => write!(f, "conn {conn}: frame outside open/close"),
            Self::Undecodable { conn } => write!(f, "conn {conn}: undecodable frame"),
            Self::NeverAnswered => f.write_str("a request was never answered"),
        }
    }
}

impl std::error::Error for SimError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Schedule(error) | Self::QuestionBank(error) => Some(error),
            Self::NotOpen { .. } | Self::Undecodable { .. } | Self::NeverAnswered => None,
        }
    }
}

pub struct Sim {
    served: Served,
    conns: HashMap<ConnId, Connection>,
    reply_tx: mpsc::Sender<(ConnId, Vec<u8>)>,
    replies: mpsc::Receiver<(ConnId, Vec<u8>)>,
}

impl Sim {
    /// A fresh server at `start`, served with `settings`. A programme in `storage` (a recording's
    /// snapshot) is served with its saved shows, as live.
    pub fn new(
        settings: Settings,
        options: SimulationOptions,
        storage: Arc<dyn Storage>,
        start: Utc,
    ) -> Result<Sim, SimError> {
        let clock = Arc::new(VirtualClock::starting_at(start));
        let schedule = Schedule::load(ScheduleConfig::default(), Arc::clone(&storage))
            .map_err(SimError::Schedule)?;
        let questions = crate::questions::QuestionBank::in_effect(
            storage.question_bank().map_err(SimError::QuestionBank)?,
        );
        let store = Store::bootstrap_with(options)
            .with_storage(storage)
            .with_clock(Arc::clone(&clock) as Arc<dyn Clock>)
            .with_questions(Arc::new(questions));
        let app = Arc::new(App::new(
            settings,
            Arc::new(std::sync::Mutex::new(schedule)),
            store,
            Members::default(),
        ));
        let (reply_tx, replies) = mpsc::channel();
        Ok(Sim {
            served: Served {
                clock,
                app,
                unanswered: Arc::new(AtomicUsize::new(0)),
            },
            conns: HashMap::new(),
            reply_tx,
            replies,
        })
    }

    /// Serve `config` in the title's Config document (what a recording was served).
    pub fn with_served_config(self, config: crate::xsor::ServedConfig) -> Sim {
        {
            let mut store = crate::sync::lock(&self.served.app.store);
            let fresh = std::mem::take(&mut *store);
            *store = fresh.with_served_config(config);
        }
        self
    }

    pub fn now(&self) -> Utc {
        self.served.clock.now()
    }

    /// The console at `peer` is signed in as `xuid` (what matchmaking learns on a live server).
    pub fn note_console_xuid(&self, peer: IpAddr, xuid: Xuid) {
        crate::sync::lock(&self.served.app.members).note_console_players(peer, &[xuid]);
    }

    /// A console opens SBG connection `id`.
    pub fn open(&mut self, id: ConnId, peer: IpAddr) {
        let caller = Caller {
            peer,
            conn: ConnId(NEXT_CONN.fetch_add(1, Ordering::Relaxed)),
        };
        let replies = self.reply_tx.clone();
        let on_reply = move |reply| {
            let _ = replies.send((id, reply));
        };
        let connection = Connection::open(caller, self.served.clone(), on_reply);
        self.conns.insert(id, connection);
    }

    pub fn close(&mut self, id: ConnId) {
        if let Some(conn) = self.conns.remove(&id) {
            conn.close(&self.served.clock);
        }
    }

    /// Connection `id` receives `frame` now. A frame of the other title build ends the connection,
    /// as live; one that does not decode is not answered.
    pub fn receive(&mut self, id: ConnId, frame: &[u8]) -> Result<(), SimError> {
        let Some(conn) = self.conns.get(&id) else {
            return Err(SimError::NotOpen { conn: id });
        };
        let Ok(Some((frame, _))) = wire::decode(frame) else {
            return Err(SimError::Undecodable { conn: id });
        };
        if frame.protocol() != self.served.app.sbg.protocol {
            self.close(id);
            return Ok(());
        }
        let Some(message) = decode(&frame) else {
            return Ok(());
        };
        self.served.unanswered.fetch_add(1, Ordering::SeqCst);
        conn.request(&self.served.clock, frame.cookie, message);
        Ok(())
    }

    /// Move the clock to `to`.
    pub fn advance_to(&mut self, to: Utc) {
        self.served.clock.advance_to(to);
    }

    /// When a held request or a waiting connection wakes next.
    pub fn next_wake(&self) -> Option<Utc> {
        self.served.clock.next_wake()
    }

    /// Requests received and not answered yet.
    pub fn unanswered(&self) -> usize {
        self.served.unanswered.load(Ordering::SeqCst)
    }

    /// Run until every request received so far is answered.
    pub fn settle(&mut self) -> Result<(), SimError> {
        while self.unanswered() > 0 {
            let wake = self.next_wake().ok_or(SimError::NeverAnswered)?;
            self.advance_to(wake);
        }
        Ok(())
    }

    /// The replies sent since the last call, as (connection, frame), in the order they were sent.
    pub fn replies(&self) -> Vec<(ConnId, Vec<u8>)> {
        self.replies.try_iter().collect()
    }
}

/// What every connection of one `Sim` shares.
#[derive(Clone)]
struct Served {
    clock: Arc<VirtualClock>,
    app: Arc<App>,
    /// Requests handed to a connection and not yet answered.
    unanswered: Arc<AtomicUsize>,
}

enum Job {
    Request(u16, Result<Message, ()>),
    Close,
}

/// One connection on its own thread, as live; every job holds a `Busy` guard.
struct Connection {
    jobs: mpsc::Sender<(Job, Busy)>,
}

impl Connection {
    fn open(
        caller: Caller,
        served: Served,
        on_reply: impl Fn(Vec<u8>) + Clone + Send + 'static,
    ) -> Self {
        let (jobs, received) = mpsc::channel::<(Job, Busy)>();
        let opening = served.clock.busy();
        thread::spawn(move || {
            let _live = LiveConnection::open(&served.app.host, caller.peer);
            drop(opening);
            for (job, _busy) in received {
                let Job::Request(cookie, message) = job else {
                    break;
                };
                let off_thread = answered_off_thread(&message);
                let answer = {
                    let (served, on_reply) = (served.clone(), on_reply.clone());
                    move || {
                        if let Some(reply) = respond(caller, ctx(&served.app), cookie, message) {
                            on_reply(reply);
                        }
                        served.unanswered.fetch_sub(1, Ordering::SeqCst);
                    }
                };
                if off_thread {
                    let busy = served.clock.busy();
                    thread::spawn(move || {
                        let _busy = busy;
                        answer();
                    });
                } else {
                    answer();
                }
            }
        });
        Self { jobs }
    }

    fn request(&self, clock: &Arc<VirtualClock>, cookie: u16, message: Result<Message, ()>) {
        let _ = self
            .jobs
            .send((Job::Request(cookie, message), clock.busy()));
    }

    fn close(self, clock: &Arc<VirtualClock>) {
        let _ = self.jobs.send((Job::Close, clock.busy()));
    }
}
