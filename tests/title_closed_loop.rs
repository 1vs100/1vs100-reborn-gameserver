//! Model consoles (`support::title_ssm`, the title's `ServerStateManager`) playing against the real
//! SBG server (`sbg::sim`) on a virtual clock, closed loop: every GET is answered by the server and
//! the console's next request depends on the answer. The One posts its states the way the title
//! does (`SetState`), built from recorded GameStates (`fixtures/gamestates.txt`).
//!
//! These are the acceptance tests of the GameState handover: the server's answer to a console's
//! second GET keeps the first round from opening twice.

mod support;

use std::{
    io::Write,
    net::IpAddr,
    sync::{Arc, Mutex},
};

use onevs100_gameserver::{
    gamestate::GameStateDoc,
    ids::{ConnId, Xuid},
    sbg::sim::Sim,
    storage::MemoryStorage,
    store::SimulationOptions,
    time::Utc,
    wire::{Protocol, v1317},
};
use support::title_ssm::{State, TitleSsm, ms};

/// Each test runs a game on the process-wide SBG state; one at a time.
static GAME: Mutex<()> = Mutex::new(());

const START: i64 = 1_792_618_000_000;
/// The first ElectHost starts this long after the consoles start their state managers (about what
/// the recordings show).
const LEAD: i64 = 90_000;
const SESSION_START: i64 = START + LEAD;
const STATE_KEY: &str =
    "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.GameState";

/// What the One posts after raising a state. Read off the recordings; the delays only have to
/// leave the post before the state's end.
struct PostPlan {
    name: &'static str,
    length: i64,
    /// After the raise.
    delay: i64,
}

fn next_post(raised: &str) -> Option<PostPlan> {
    Some(match raised {
        "ElectHost" => PostPlan {
            name: "LiveHostIntro",
            length: 22_000,
            delay: 25_000,
        },
        "LiveHostIntro" => PostPlan {
            name: "Introductions",
            length: 38_000,
            delay: 10_000,
        },
        "Introductions" => PostPlan {
            name: "QA",
            length: 24_000,
            delay: 12_000,
        },
        "QA" => PostPlan {
            name: "OnesAnswer",
            length: 16_000,
            delay: 14_000,
        },
        _ => return None,
    })
}

/// A post the One still has to send.
#[derive(Clone, Copy)]
struct Post {
    /// When to send it.
    at: i64,
    name: &'static str,
    length: i64,
    /// The question the state carries.
    question: u32,
}

struct Console {
    id: ConnId,
    xuid: Xuid,
    ssm: TitleSsm,
    cookie: u16,
    /// The GET waiting for its answer.
    get: Option<u16>,
    /// States already looked at for a post to make.
    seen_raised: usize,
    /// The One's posts still to send.
    posts: Vec<Post>,
    /// ElectHosts raised: the round the One's posts carry.
    rounds: u32,
    left: bool,
}

impl Console {
    fn next_cookie(&mut self) -> u16 {
        self.cookie += 1;
        self.cookie
    }
}

struct Game {
    sim: Sim,
    consoles: Vec<Console>,
    /// Which console is the One (its posts are the only ones sent).
    one: usize,
}

impl Game {
    fn new() -> Game {
        let options = SimulationOptions {
            synthetic_mob: true,
            synthetic_crowd: 60,
        };
        let mut settings = onevs100_gameserver::sbg::Settings::new(Protocol::V1317);
        settings.server_flow = true;
        settings.service_host = "10.0.0.106".into();
        let sim = Sim::new(
            settings,
            options,
            Arc::new(MemoryStorage::default()),
            Utc::from_unix_ms(START),
        )
        .expect("the empty storage loads");
        Game {
            sim,
            consoles: Vec::new(),
            one: 0,
        }
    }

    fn now(&self) -> i64 {
        ms(self.sim.now())
    }

    /// A console starts its state manager now.
    fn join(&mut self, xuid: Xuid) -> usize {
        let index = self.consoles.len();
        let peer: IpAddr = format!("10.0.0.{}", 130 + index).parse().unwrap();
        let id = ConnId(index as u64 + 1);
        self.sim.note_console_xuid(peer, xuid);
        self.sim.open(id, peer);
        let now = self.now();
        self.consoles.push(Console {
            id,
            xuid,
            ssm: TitleSsm::start(now, SESSION_START),
            cookie: 0,
            get: None,
            seen_raised: 0,
            posts: Vec::new(),
            rounds: 0,
            left: false,
        });
        self.settle();
        index
    }

    /// The console quits the title: its connection closes and it sends nothing more.
    fn leave(&mut self, index: usize) {
        self.consoles[index].left = true;
        let id = self.consoles[index].id;
        self.sim.close(id);
        self.settle();
    }

    /// Play until `until` (Unix ms).
    fn run_until(&mut self, until: i64) {
        loop {
            let wakes = self
                .consoles
                .iter()
                .filter(|console| !console.left)
                .flat_map(|console| {
                    console
                        .ssm
                        .next_wake()
                        .into_iter()
                        .chain(console.posts.iter().map(|post| post.at))
                });
            let next = wakes
                .chain(self.sim.next_wake().map(ms))
                .filter(|wake| *wake <= until)
                .min();
            let Some(next) = next else {
                self.sim.advance_to(Utc::from_unix_ms(until));
                self.settle();
                return;
            };
            self.sim.advance_to(Utc::from_unix_ms(next.max(self.now())));
            self.settle();
        }
    }

    /// Let everything due now happen: answers reach the consoles, consoles raise states, send
    /// their GETs and the One its posts, until nothing more happens at this instant.
    fn settle(&mut self) {
        loop {
            let mut acted = false;
            let now = self.now();
            for (id, frame) in self.sim.replies() {
                let Some(console) = self.consoles.iter_mut().find(|c| c.id == id && !c.left) else {
                    continue;
                };
                let Some((cookie, state)) = support::replied_state(Protocol::V1317, &frame) else {
                    continue;
                };
                if console.get == Some(cookie) {
                    console.get = None;
                    console
                        .ssm
                        .reply(now, state.as_ref().and_then(State::from_doc));
                    acted = true;
                }
            }
            let one = self.one;
            for (index, console) in self.consoles.iter_mut().enumerate() {
                if console.left {
                    continue;
                }
                console.ssm.step(now);
                for raised in &console.ssm.raised[console.seen_raised..] {
                    if raised.name == "ElectHost" {
                        console.rounds += 1;
                    }
                    if index == one
                        && let Some(plan) = next_post(&raised.name)
                    {
                        // QA asks the next question, OnesAnswer answers the one asked.
                        let question = match plan.name {
                            "QA" => raised.question.unwrap_or(0) + 1,
                            "OnesAnswer" => raised.question.unwrap_or(1),
                            _ => 0,
                        };
                        console.posts.push(Post {
                            at: raised.start + plan.delay,
                            name: plan.name,
                            length: plan.length,
                            question,
                        });
                    }
                }
                console.seen_raised = console.ssm.raised.len();
                if let Some(get) = console.ssm.get_due(now) {
                    let cookie = console.next_cookie();
                    console.get = Some(cookie);
                    let frame = request(cookie, get.execute_at, None);
                    self.sim.receive(console.id, &frame).unwrap();
                    acted = true;
                }
                let due: Vec<_> = console
                    .posts
                    .iter()
                    .copied()
                    .filter(|post| post.at <= now)
                    .collect();
                console.posts.retain(|post| post.at > now);
                for post in due {
                    let Some(set) = console.ssm.set_state(now, post.name, post.length, 0) else {
                        continue; // the title refuses a post whose execute time has passed
                    };
                    let doc = posted_doc(&set.state, console.rounds, console.xuid, post.question);
                    let cookie = console.next_cookie();
                    let frame = request(cookie, set.execute_at, Some(&doc));
                    self.sim.receive(console.id, &frame).unwrap();
                    acted = true;
                }
            }
            if !acted {
                return;
            }
            self.sim.advance_to(self.sim.now());
        }
    }
}

/// A 1317 request on the state key: a GET, or a SET of `doc` (compressed, as the title sends it).
fn request(cookie: u16, execute_at: i64, doc: Option<&[u8]>) -> Vec<u8> {
    const UNIX_EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;
    let execute_time_utc = u64::try_from(execute_at).unwrap() * 10_000 + UNIX_EPOCH_AS_FILETIME;
    let value = doc.map(|doc| {
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        zlib.write_all(doc).unwrap();
        zlib.finish().unwrap()
    });
    v1317::NamedRequest {
        key: STATE_KEY.as_bytes(),
        tag: b"",
        compressed: value.is_some(),
        value: value.as_deref().unwrap_or_default(),
        execute_time_utc,
    }
    .to_frame(cookie)
    .unwrap()
}

/// A recorded 1317 GameState called `state.name`, re-timed to `state` and carrying `round`, the
/// One's XUID and `question`.
fn posted_doc(state: &State, round: u32, ones_xuid: Xuid, question: u32) -> Vec<u8> {
    let template = include_str!("../fixtures/gamestates.txt")
        .lines()
        .filter(|line| line.starts_with("<XsorObject>"))
        .filter_map(|line| GameStateDoc::parse(line.as_bytes()))
        .find(|doc| doc.name() == state.name)
        .unwrap_or_else(|| panic!("no recorded {}", state.name));
    let mut doc = template;
    let time = |at: i64| Utc::from_unix_ms(at).to_xsor();
    doc.set("startTimeUtc", time(state.start));
    doc.set("endTimeUtc", time(state.end));
    doc.set("sendGetUtc", time(state.send_get.unwrap_or(state.start)));
    doc.set_field("stateLength", (state.end - state.start) / 1000);
    doc.set_field("RoundNumber", round);
    // Decimal: the title writes and reads its XUID fields as numbers.
    doc.set_field("OnesXuid", ones_xuid.0);
    doc.set_field("QuestionNumber", question);
    doc.to_xml()
}

fn names(states: &[State]) -> Vec<&str> {
    states.iter().map(|state| state.name.as_str()).collect()
}

/// One console, the One, through the first question.
fn one_console() -> Game {
    let mut game = Game::new();
    game.join(Xuid(16_140_907_667_073_616_695));
    game.run_until(SESSION_START + 51_000 + 22_000 + 38_000 + 24_000 + 5_000);
    game
}

#[test]
fn the_first_round_opens_once() {
    let _game = onevs100_gameserver::sync::lock(&GAME);
    let game = one_console();
    let ssm = &game.consoles[0].ssm;
    assert_eq!(
        names(&ssm.raised)[..4],
        ["ElectHost", "LiveHostIntro", "Introductions", "QA"],
        "{:?}",
        ssm.raised
    );
    assert_eq!(
        ssm.raised[1].start,
        SESSION_START + 51_000,
        "LiveHostIntro follows the first ElectHost"
    );
}

#[test]
fn two_consoles_raise_the_same_states_at_the_same_times() {
    let _game = onevs100_gameserver::sync::lock(&GAME);
    let mut game = Game::new();
    game.join(Xuid(16_140_907_667_073_616_695));
    game.run_until(START + 3_000);
    game.join(Xuid(16_140_907_667_073_616_696));
    game.run_until(SESSION_START + 51_000 + 22_000 + 38_000 + 24_000 + 5_000);
    let (a, b) = (&game.consoles[0].ssm, &game.consoles[1].ssm);
    assert_eq!(
        names(&a.raised)[..4],
        ["ElectHost", "LiveHostIntro", "Introductions", "QA"]
    );
    assert_eq!(a.raised, b.raised);
}

#[test]
fn a_console_that_joins_during_a_question_follows_from_the_next_state() {
    let _game = onevs100_gameserver::sync::lock(&GAME);
    let mut game = Game::new();
    game.join(Xuid(16_140_907_667_073_616_695));
    let qa_start = SESSION_START + 51_000 + 22_000 + 38_000;
    game.run_until(qa_start + 5_000);
    let late = game.join(Xuid(16_140_907_667_073_616_696));
    game.run_until(qa_start + 24_000 + 5_000);
    let (a, b) = (&game.consoles[0].ssm, &game.consoles[late].ssm);
    assert!(b.joined_in_progress >= 1, "{:?}", b.raised);
    assert!(!names(&b.raised).contains(&"ElectHost"), "{:?}", b.raised);
    assert_eq!(
        b.raised.first(),
        a.raised.last(),
        "both raise the state after QA"
    );
}

#[test]
fn when_the_one_leaves_the_other_console_elects_once() {
    let _game = onevs100_gameserver::sync::lock(&GAME);
    let mut game = Game::new();
    game.join(Xuid(16_140_907_667_073_616_695));
    game.run_until(START + 3_000);
    let other = game.join(Xuid(16_140_907_667_073_616_696));
    let qa_start = SESSION_START + 51_000 + 22_000 + 38_000;
    game.run_until(qa_start + 5_000);
    game.leave(0);
    game.one = other;
    let qa_end = qa_start + 24_000;
    game.run_until(qa_end + 51_000 + 22_000 + 5_000);
    let ssm = &game.consoles[other].ssm;
    let after_qa: Vec<_> = ssm
        .raised
        .iter()
        .filter(|state| state.start >= qa_end)
        .collect();
    assert_eq!(
        after_qa
            .iter()
            .map(|state| state.name.as_str())
            .take(2)
            .collect::<Vec<_>>(),
        ["ElectHost", "LiveHostIntro"],
        "{:?}",
        ssm.raised
    );
    let elections = after_qa.iter().filter(|state| state.name == "ElectHost");
    assert_eq!(elections.count(), 1, "{:?}", ssm.raised);
    assert_eq!(
        after_qa[0].start, qa_end,
        "the re-election starts when the abandoned QA ends"
    );
}
