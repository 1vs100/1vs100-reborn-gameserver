//! The execute times decoded from both builds' requests follow the title's `ServerStateManager`
//! (`serverstatemanager.lua`, `beginGetState`): a GameState GET
//! asked while a state is current executes `GetExecutionDelta` (3.175 s) before that state ends.
//! Checked on every recorded session.

mod support;

use std::collections::HashMap;

use onevs100_gameserver::{gamestate::GameStateDoc, ids::ConnId, sbg::record::Kind};

const GET_EXECUTION_DELTA: std::time::Duration = std::time::Duration::from_millis(3_175);

#[test]
fn state_gets_execute_just_before_the_current_state_ends() {
    let mut checked = 0;
    let recordings = support::recorded_sessions()
        .into_iter()
        .chain(support::first_round_opens_twice_recordings());
    for (path, recording) in recordings {
        let protocol = recording.settings.protocol;
        // Per connection: GET cookies waiting for a reply, and the state the console holds now.
        let mut pending = HashMap::<(ConnId, u16), ()>::new();
        let mut current = HashMap::<ConnId, GameStateDoc>::new();
        for event in &recording.events {
            match event.kind {
                Kind::In => {
                    let Some(request) = support::state_request(protocol, &event.frame) else {
                        continue;
                    };
                    if request.posted.is_some() {
                        continue;
                    }
                    if let Some(state) = current.get(&event.conn) {
                        assert_eq!(
                            request.execute_at.map(|at| at.to_xsor()),
                            Some((state.end().unwrap() - GET_EXECUTION_DELTA).to_xsor()),
                            "{}: GET {} after {}",
                            path.display(),
                            request.cookie,
                            state.name(),
                        );
                        checked += 1;
                    }
                    pending.insert((event.conn, request.cookie), ());
                }
                Kind::Out => {
                    let Some((cookie, state)) = support::replied_state(protocol, &event.frame)
                    else {
                        continue;
                    };
                    if pending.remove(&(event.conn, cookie)).is_none() {
                        continue;
                    }
                    match state {
                        Some(state) => current.insert(event.conn, state),
                        // NOT_FOUND: the console invents an ElectHost the server never sees.
                        None => current.remove(&event.conn),
                    };
                }
                _ => {}
            }
        }
    }
    assert!(checked >= 60, "only {checked} GETs checked");
}
