//! Server state, built once by `server::run` and shared by every service.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    time::Instant,
};

use crate::{
    matchmaking::Members,
    sbg::{self, host::HostAuthority},
    schedule::Schedule,
    store::Store,
};

pub struct App {
    pub started: Instant,
    /// What SBG serves with: the build, the server flow, the service host, dumps.
    pub sbg: sbg::Settings,
    /// When the shows are; shared with the Grid and with the roster (which session a joining
    /// console goes into).
    pub schedule: Arc<Mutex<Schedule>>,
    /// The show's values, sums, TopN and elections, and the persistent player data.
    pub store: Mutex<Store>,
    /// The matchmaking roster. The matchmaking service holds a handle of its own, as it needs
    /// nothing else.
    pub members: Arc<Mutex<Members>>,
    /// Which console drives GameState progression; server-wide, a new show keeps it.
    pub host: Mutex<HostAuthority>,
    /// Every SBG frame: the recent ones, and a recording file while one is open.
    pub capture: sbg::capture::Capture,
    /// Unix seconds of the last frame read on any SBG connection; 0 = never (the health check).
    last_sbg_activity: AtomicI64,
}

impl App {
    /// The roster is wired to `schedule`: a joining console goes into its current session.
    pub fn new(
        sbg: sbg::Settings,
        schedule: Arc<Mutex<Schedule>>,
        store: Store,
        mut members: Members,
    ) -> Self {
        let shows = Arc::clone(&schedule);
        members.set_current_session(move || {
            crate::sync::lock(&shows).current_session_id(crate::time::now().into())
        });
        let members = Arc::new(Mutex::new(members));
        Self {
            started: Instant::now(),
            sbg,
            schedule,
            store: Mutex::new(store),
            capture: sbg::capture::Capture::new(Arc::clone(&members)),
            members,
            host: Mutex::default(),
            last_sbg_activity: AtomicI64::new(0),
        }
    }

    pub fn note_sbg_activity(&self) {
        self.last_sbg_activity
            .store(crate::time::now().unix_secs(), Ordering::Relaxed);
    }

    /// When an SBG connection last read a frame, if ever.
    pub fn last_sbg_activity_unix_secs(&self) -> Option<i64> {
        match self.last_sbg_activity.load(Ordering::Relaxed) {
            0 => None,
            secs => Some(secs),
        }
    }
}
