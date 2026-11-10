//! The port of the title's `ServerStateManager` (`support::title_ssm`) against the recorded
//! sessions, open loop: fed every recorded GameState reply at its recorded time, the model must
//! ask for the recorded GameStates (same execute time to the millisecond, sent within
//! `SEND_TOLERANCE`) and its `SetState` must give every recorded post its start and execute time.
//! Then the model can stand in for consoles in closed-loop tests against the server
//! (`tests/title_closed_loop.rs`).

mod support;

use std::collections::{BTreeMap, HashSet};

use onevs100_gameserver::{ids::ConnId, sbg::record::Kind};
use support::title_ssm::{self, State, TitleSsm, ms};

/// Network delay and the console's clock offset: a frame reaches the server up to this long
/// after the console meant to send it.
const SEND_TOLERANCE: i64 = 2_000;

/// How far the session start of a restarted state manager is looked for around the previous one.
const RESTART_WINDOW: i64 = 10_000;

#[test]
fn the_model_asks_and_posts_what_the_consoles_did() {
    // How many ElectHosts the console raised before the first LiveHostIntro: two where the first
    // round opens twice.
    let recordings = support::recorded_sessions()
        .into_iter()
        .map(|recording| (recording, 1))
        .chain(
            support::first_round_opens_twice_recordings()
                .into_iter()
                .map(|recording| (recording, 2)),
        );
    for ((path, recording), opening_elections) in recordings {
        let name = path.display().to_string();
        let protocol = recording.settings.protocol;
        // Per console (connection; a recording can hold several), every title state manager that
        // is still consistent with what the console did. Usually one.
        let mut models: BTreeMap<ConnId, Vec<TitleSsm>> = BTreeMap::new();
        let mut gets = HashSet::new();
        let (mut checked_gets, mut checked_sets) = (0, 0);
        for event in &recording.events {
            let now = ms(support::at(&recording, event.at_ms));
            for (conn, candidates) in models.iter_mut() {
                let mut unexpected = None;
                candidates.retain_mut(|model| {
                    let quiet = advance(model, now);
                    unexpected = unexpected.or(quiet.err());
                    quiet.is_ok()
                });
                if let (true, Some((wake, execute_at))) = (candidates.is_empty(), unexpected) {
                    panic!(
                        "{name}: console on connection {conn}: the model sent a GET at {wake} (execute {execute_at}) the console did not"
                    );
                }
            }
            match event.kind {
                Kind::In => {
                    let Some(request) = support::state_request(protocol, &event.frame) else {
                        continue;
                    };
                    let execute_at = ms(request
                        .execute_at
                        .expect("state requests carry an execute time"));
                    match request.posted {
                        None => {
                            let candidates = models.entry(event.conn).or_default();
                            let expected = std::mem::take(candidates);
                            let raised_nothing = expected.iter().all(|m| m.raised.is_empty());
                            let previous_start = expected.first().map(TitleSsm::session_start);
                            let asked: Vec<_> =
                                expected.iter().map(TitleSsm::pending_get).collect();
                            let mut kept: Vec<_> = expected
                                .into_iter()
                                .filter(|model| expects(model, execute_at, now))
                                .collect();
                            if kept.is_empty() && raised_nothing {
                                // The first GET, or a new state manager before the console raised
                                // anything (the lobby restarts it).
                                kept = started(execute_at, now, previous_start);
                            }
                            assert!(
                                !kept.is_empty(),
                                "{name}: GET {} at {now} (execute {execute_at}) on connection {}: no title state manager asks it; the model asks {asked:?}",
                                request.cookie,
                                event.conn
                            );
                            for model in &mut kept {
                                model.sent();
                            }
                            *candidates = kept;
                            gets.insert((event.conn, request.cookie));
                            checked_gets += 1;
                        }
                        Some(posted) => {
                            let candidates =
                                models.get_mut(&event.conn).expect("a post before any GET");
                            let posted = State::from_doc(&posted).unwrap();
                            let currents = format!(
                                "{:?}",
                                candidates
                                    .iter()
                                    .map(|model| model.current_during(now - SEND_TOLERANCE, now))
                                    .collect::<Vec<_>>()
                            );
                            candidates.retain(|model| {
                                let currents = model.current_during(now - SEND_TOLERANCE, now);
                                chains_onto(&currents, &posted, execute_at, now)
                            });
                            assert!(
                                !candidates.is_empty(),
                                "{name}: {posted:?} (execute {execute_at}) posted at {now}, current then: {currents}"
                            );
                            checked_sets += 1;
                        }
                    }
                }
                Kind::Out => {
                    let Some((cookie, state)) = support::replied_state(protocol, &event.frame)
                    else {
                        continue;
                    };
                    if gets.remove(&(event.conn, cookie)) {
                        let state = state.as_ref().and_then(State::from_doc);
                        for model in models.get_mut(&event.conn).unwrap() {
                            model.reply(now, state.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        assert!(!models.is_empty(), "{name}: no GameState traffic");
        // The check covered a whole first round: every console went on to round 2.
        assert!(
            checked_gets > 0 && checked_sets > 0,
            "{name}: {checked_gets} GETs, {checked_sets} posts"
        );
        for (conn, candidates) in &models {
            // A restart leaves several session starts until the console's next GETs tell them
            // apart; by the end of a round one is left.
            assert_eq!(
                candidates.len(),
                1,
                "{name}: console on connection {conn}: session starts {:?}",
                candidates
                    .iter()
                    .map(TitleSsm::session_start)
                    .collect::<Vec<_>>()
            );
            let model = &candidates[0];
            assert!(
                model.raised.iter().any(|state| state.name == "RoundStart"),
                "{name}: console on connection {conn} never reached RoundStart: {:?}",
                model.raised
            );
            assert_eq!(
                model.opening_elections(),
                opening_elections,
                "{name}: console on connection {conn}: {:?}",
                model.raised
            );
        }
    }
}

/// Run `model` up to `now`; `Err((wake, execute))` when it sent a GET the console had not sent
/// by then.
fn advance(model: &mut TitleSsm, now: i64) -> Result<(), (i64, i64)> {
    while let Some(wake) = model
        .next_wake()
        .filter(|wake| *wake <= now - SEND_TOLERANCE)
    {
        model.step(wake);
        if let Some(get) = model.get_due(wake) {
            return Err((wake, get.execute_at));
        }
    }
    model.step(now);
    Ok(())
}

/// Whether `model`'s pending GET is the console's: the same execute time, sent in time.
fn expects(model: &TitleSsm, execute_at: i64, now: i64) -> bool {
    model.pending_get().is_some_and(|get| {
        get.execute_at == execute_at && (now - get.send_at).abs() <= SEND_TOLERANCE
    })
}

/// The state managers whose first GET (`Init`) is the console's (see `decision_times`). In sync, the GET executes GetExecutionDelta before the session starts.
/// After a restart the session start may have moved, and on the JIP branch (ssm:246) the GET
/// only fixes `start + ceil(now - start)`: every start that fits, near the previous one.
fn started(execute_at: i64, now: i64, previous_start: Option<i64>) -> Vec<TitleSsm> {
    let in_sync = execute_at + title_ssm::GET_EXECUTION_DELTA;
    let session_starts = match previous_start {
        None => vec![in_sync],
        Some(previous) => {
            let jip = execute_at - title_ssm::GET_REQUEST_DELTA;
            let mut starts: Vec<_> = (-RESTART_WINDOW / 1000..=RESTART_WINDOW / 1000)
                .map(|seconds| previous + seconds * 1000)
                .map(|near| jip + (near - jip).div_euclid(1000) * 1000)
                .chain([in_sync])
                .filter(|start| (start - previous).abs() <= RESTART_WINDOW)
                .collect();
            starts.sort_unstable();
            starts.dedup();
            starts
        }
    };
    session_starts
        .into_iter()
        .filter_map(|session_start| {
            decision_times(session_start, now)
                .map(|at| TitleSsm::start(at, session_start))
                .find(|model| expects(model, execute_at, now))
        })
        .collect()
}

/// When `Init` may have run, for a frame that arrived at `now`: `beginGetState` depends on whole
/// seconds since the session start (ssm:241), so the window's second boundaries and its ends
/// cover every outcome.
fn decision_times(session_start: i64, now: i64) -> impl Iterator<Item = i64> {
    let from = now - SEND_TOLERANCE;
    let first_boundary = session_start + (from - session_start).div_euclid(1000) * 1000 + 1000;
    std::iter::once(from)
        .chain((first_boundary..now).step_by(1000))
        .chain(std::iter::once(now))
}

/// Whether `posted` is what `SetState` makes on one of `currents`. It ran on the One's game
/// thread up to the network delay before the frame arrived; the state it chained onto was
/// current then.
fn chains_onto(currents: &[&State], posted: &State, execute_at: i64, now: i64) -> bool {
    currents.iter().any(|current| {
        title_ssm::set_state(
            current,
            now - SEND_TOLERANCE,
            &posted.name,
            posted.end - posted.start,
            0,
        )
        .is_some_and(|set| {
            (set.state.start, set.state.end, set.execute_at)
                == (posted.start, posted.end, execute_at)
        })
    })
}
