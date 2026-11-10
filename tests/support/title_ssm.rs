//! The title's `ServerStateManager`, ported for tests: how a console asks for GameStates and
//! raises them. Written line by line from the v2.0.1149.0 retail script
//! (`serverstatemanager.lua`, cited as `ssm:<line>`); 1317 runs
//! the same code. The Lua runs the loop on a thread with blocking waits; this port is a state
//! machine driven by the caller's clock: `step` does whatever is due at `now`, `next_wake` says
//! when that is next, `get_due` hands out a GET when the console sends it and `reply` delivers its
//! answer.
//!
//! Times are Unix milliseconds. The title computes them in seconds as floats; every quantity that
//! reaches the wire is a whole number of milliseconds.

#![allow(dead_code)] // each test binary uses its own part

use onevs100_gameserver::{gamestate::GameStateDoc, time::Utc};

/// `TimeConstants.GetExecutionDelta` (timeconstants.lua:7).
pub const GET_EXECUTION_DELTA: i64 = 3_175;
/// `TimeConstants.GetRequestDelta` (timeconstants.lua:8).
pub const GET_REQUEST_DELTA: i64 = 2_825;
/// `TimeConstants.SetExecutionDelta` (timeconstants.lua:12).
pub const SET_EXECUTION_DELTA: i64 = 3_675;
/// `PTTimingsTable.ServerStateTimeLength.ElectHost` (pttimings.lua:154): 24 + 11 + 10 + 6 s.
pub const ELECT_HOST: i64 = 51_000;
/// `GameData.ServerStates.ElectHostOffset` (data.lua:160).
pub const ELECT_HOST_SEND_GET_OFFSET: i64 = 4_000;
/// A request is sent at most this long before its execute time (ssm:269).
const MAX_EARLY: i64 = 105_000;

pub const STALE_STATE: &str = "ElectHost";
const PRE_GAME_STATE: &str = "STATE_PREGAME";

/// `math.ceil(a / b)` for a positive `b`.
fn ceil_div(a: i64, b: i64) -> i64 {
    -(-a).div_euclid(b)
}

pub fn ms(time: Utc) -> i64 {
    (time.as_secs_f64() * 1000.0).round() as i64
}

/// `ServerState` (ssm:17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub name: String,
    pub start: i64,
    pub end: i64,
    pub send_get: Option<i64>,
    /// `data.QuestionNumber` of a state the server served; `None` for those the model makes.
    pub question: Option<u32>,
}

impl State {
    /// What `endGetState` makes of a reply carrying a GameState.
    pub fn from_doc(doc: &GameStateDoc) -> Option<State> {
        Some(State {
            name: doc.name().into_owned(),
            start: ms(doc.start()?),
            end: ms(doc.end()?),
            send_get: doc.send_get().map(ms),
            question: doc.reported_question(),
        })
    }
}

/// A GET the console sends (`beginGetState`, ssm:236).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Get {
    pub send_at: i64,
    pub execute_at: i64,
}

/// A SET of the next state (`SetState`, ssm:125).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Set {
    pub state: State,
    pub execute_at: i64,
}

/// The GET in flight (`get`, ssm:229).
#[derive(Debug)]
struct PendingGet {
    get: Get,
    /// Whether it has been sent.
    sent: bool,
    /// Its answer, once in.
    answer: Option<GetAnswer>,
}

/// What the GET in flight was answered with.
#[derive(Debug)]
enum GetAnswer {
    /// NOT_FOUND.
    NotFound,
    State(State),
}

#[derive(Debug)]
enum Stage {
    /// `Init`: waiting for the answer to the first GET (ssm:105).
    Init,
    /// `run`: waiting until the current state starts, to raise it (ssm:218-224).
    Raise { at: i64 },
    /// `run`: waiting for the answer to the GET for the next state (ssm:229).
    Answer,
}

#[derive(Debug)]
pub struct TitleSsm {
    session_start: i64,
    current: State,
    old_name: String,
    last_execute: i64,
    last_get_successful: bool,
    stage: Stage,
    /// The GET in flight.
    get: Option<PendingGet>,
    /// Every state raised (`onStateChanged`), in order.
    pub raised: Vec<State>,
    /// When each state became the current one, in order.
    history: Vec<(i64, State)>,
    /// How often the console joined a state in progress (`onJoinInProgress`).
    pub joined_in_progress: usize,
}

impl TitleSsm {
    /// `ServerStateManager:new` (ssm:36) and `Init` (ssm:97) at `now`, for a session whose first
    /// ElectHost starts at `session_start` (`roleCutoffTime`, serverstatecallbacks.lua:470).
    pub fn start(now: i64, session_start: i64) -> TitleSsm {
        let mut ssm = TitleSsm {
            session_start,
            current: State {
                name: PRE_GAME_STATE.into(),
                start: now,
                end: session_start,
                send_get: None,
                question: None,
            },
            old_name: String::new(),
            last_execute: 0,
            last_get_successful: false,
            stage: Stage::Init,
            get: None,
            raised: Vec::new(),
            history: Vec::new(),
            joined_in_progress: 0,
        };
        ssm.history.push((now, ssm.current.clone()));
        ssm.begin_get_state(now);
        ssm
    }

    /// When the session's first ElectHost starts, as this state manager was created with.
    pub fn session_start(&self) -> i64 {
        self.session_start
    }

    pub fn current(&self) -> &State {
        &self.current
    }

    /// The next time `step` or `get_due` has something to do.
    pub fn next_wake(&self) -> Option<i64> {
        let send = self
            .get
            .as_ref()
            .and_then(|pending| (!pending.sent).then_some(pending.get.send_at));
        let raise = match self.stage {
            Stage::Raise { at } => Some(at),
            _ => None,
        };
        send.into_iter().chain(raise).min()
    }

    /// The GET in flight, unless it has been sent already.
    pub fn pending_get(&self) -> Option<Get> {
        self.get
            .as_ref()
            .and_then(|pending| (!pending.sent).then_some(pending.get))
    }

    /// The GET whose send time has come; it is sent once.
    pub fn get_due(&mut self, now: i64) -> Option<Get> {
        let pending = self.get.as_mut()?;
        if pending.sent || pending.get.send_at > now {
            return None;
        }
        pending.sent = true;
        Some(pending.get)
    }

    /// Mark the GET in flight as sent (the caller saw it go out).
    pub fn sent(&mut self) -> Option<Get> {
        let pending = self.get.as_mut()?;
        pending.sent = true;
        Some(pending.get)
    }

    /// The answer to the GET in flight: a GameState, or `None` for NOT_FOUND.
    pub fn reply(&mut self, now: i64, state: Option<State>) {
        if let Some(pending) = self.get.as_mut() {
            pending.answer = Some(state.map_or(GetAnswer::NotFound, GetAnswer::State));
        }
        self.step(now);
    }

    /// Do everything that is due at `now`.
    pub fn step(&mut self, now: i64) {
        loop {
            match self.stage {
                Stage::Init | Stage::Answer => {
                    let Some(answer) = self.get.as_mut().and_then(|pending| pending.answer.take())
                    else {
                        return;
                    };
                    self.get = None;
                    let table = self.end_get_state(answer);
                    self.update_state(table);
                    self.history.push((now, self.current.clone()));
                    self.run_iteration(now);
                }
                Stage::Raise { at } if at <= now => {
                    // ssm:221-224
                    self.raised.push(self.current.clone());
                    self.stage = Stage::Answer;
                }
                Stage::Raise { .. } => return,
            }
        }
    }

    /// The top of the `run` loop (ssm:198-226), up to the wait for the next answer.
    fn run_iteration(&mut self, now: i64) {
        self.begin_get_state(now);
        self.stage = Stage::Answer;
        if self.last_get_successful {
            let missed_start = self.current.start < now;
            let joined_late = self.old_name == PRE_GAME_STATE && now > self.session_start;
            if missed_start || joined_late {
                self.joined_in_progress += 1;
            }
            if self.current.start >= now {
                self.stage = Stage::Raise {
                    at: self.current.start,
                };
            }
        }
    }

    /// `beginGetState` (ssm:236-290) without an explicit execute time.
    fn begin_get_state(&mut self, now: i64) {
        let since_start = now - self.session_start;
        let default_execute =
            ceil_div(since_start, 1000) * 1000 + self.session_start + GET_REQUEST_DELTA;
        self.last_execute = if default_execute > self.current.end - GET_EXECUTION_DELTA {
            default_execute // too late to sync with the next state: JIP (ssm:246-249)
        } else {
            self.current.end - GET_EXECUTION_DELTA // ssm:252
        };
        let mut send_at = self.current.send_get.unwrap_or(self.current.start); // ssm:259-264
        if self.last_execute - send_at > MAX_EARLY {
            send_at = self.last_execute - MAX_EARLY; // ssm:268-271
        }
        let get = Get {
            send_at: send_at.max(now), // ssm:273-275: sent at once when that time has passed
            execute_at: self.last_execute,
        };
        self.get = Some(PendingGet {
            get,
            sent: false,
            answer: None,
        });
    }

    /// `endGetState` (ssm:293-327): a NOT_FOUND becomes the session's first ElectHost.
    fn end_get_state(&self, answer: GetAnswer) -> State {
        match answer {
            GetAnswer::State(state) => state,
            GetAnswer::NotFound => State {
                name: STALE_STATE.into(),
                start: self.session_start,
                end: self.session_start + ELECT_HOST,
                send_get: Some(self.session_start + ELECT_HOST_SEND_GET_OFFSET),
                question: None,
            },
        }
    }

    /// `updateState` (ssm:329-368).
    fn update_state(&mut self, table: State) {
        self.old_name = std::mem::take(&mut self.current.name);
        self.last_get_successful = true;
        self.current = if self.last_execute >= table.end - GET_EXECUTION_DELTA {
            // Stale: the console runs an ElectHost of its own (ssm:344-362).
            let start = if self.last_execute < table.end {
                table.end
            } else {
                let since = self.last_execute - table.end;
                let mut start = table.end + ceil_div(since, ELECT_HOST) * ELECT_HOST;
                if start - self.last_execute > GET_EXECUTION_DELTA {
                    start -= ELECT_HOST;
                }
                start
            };
            State {
                name: STALE_STATE.into(),
                start,
                end: start + ELECT_HOST,
                send_get: None,
                question: None,
            }
        } else {
            table
        };
    }

    /// Every state that was the current one at some time in `from..=to`.
    pub fn current_during(&self, from: i64, to: i64) -> Vec<&State> {
        let mut states = Vec::new();
        for (i, (since, state)) in self.history.iter().enumerate() {
            let until = self.history.get(i + 1).map_or(i64::MAX, |(next, _)| *next);
            if *since <= to && until >= from {
                states.push(state);
            }
        }
        states
    }

    /// `SetState` (ssm:125-193) for the current state.
    pub fn set_state(
        &self,
        now: i64,
        name: &str,
        duration: i64,
        send_get_offset: i64,
    ) -> Option<Set> {
        set_state(&self.current, now, name, duration, send_get_offset)
    }
}

/// `SetState` (ssm:125-193): the One posting the state after `current`, `None` when the title
/// refuses (execute time in the past). The new state is computed when it is called (ssm:172);
/// the frame leaves afterwards.
pub fn set_state(
    current: &State,
    now: i64,
    name: &str,
    duration: i64,
    send_get_offset: i64,
) -> Option<Set> {
    {
        let execute_at = current.end - SET_EXECUTION_DELTA;
        if execute_at < now {
            return None;
        }
        Some(Set {
            state: State {
                name: name.into(),
                start: current.end,
                end: current.end + duration,
                send_get: Some(current.end + send_get_offset),
                question: None,
            },
            execute_at,
        })
    }
}

impl TitleSsm {
    /// How many ElectHosts were raised before the first other state: 1 is the title's design.
    pub fn opening_elections(&self) -> usize {
        self.raised
            .iter()
            .take_while(|state| state.name == STALE_STATE)
            .count()
    }
}
