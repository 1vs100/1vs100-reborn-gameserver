//! Sum counters: show and persistent stats.

use std::{
    sync::Mutex,
    time::{Duration, SystemTime},
};

use crate::{store::Store, sync::lock, xsor};

use crate::sbg::{
    conn::*,
    key::{Key, ParsedKey},
    message::{Action, Reply},
};

/// Lobby player-count GET wait: every console's post for the bucket lands first.
const LOBBY_SUM_SETTLE: Duration = Duration::from_millis(1500);

pub(in crate::sbg) fn sum(
    key: &ParsedKey,
    action: &Action,
    execute_at: Option<SystemTime>,
    store: &Mutex<Store>,
) -> Reply {
    let clock = lock(store).clock();
    if *action == Action::SumGet
        && let Some(at) = execute_at
    {
        // GETs can precede the SUM; wait for the counter until execute time.
        let now = clock.now();
        let deadline = now + hold_for(Some(at), now.into());
        while lock(store).sum_get(key).is_none() && clock.now() < deadline {
            clock.sleep(Duration::from_millis(100));
        }
    } else if *action == Action::SumGet && matches!(key.kind, Key::LobbyPlayerCount { .. }) {
        // Answer after the bucket closes; at once the first poster reads 1, the second 2.
        clock.sleep(LOBBY_SUM_SETTLE);
    }
    let mut store = lock(store);
    match *action {
        Action::SumAdd(delta) if store.sum_add(&key.raw, delta) => Reply::ok_empty(),
        Action::SumAdd(_) => Reply::not_found(),
        _ => match store.sum_get(key) {
            Some(total) => Reply::document(xsor::sum_value(total)),
            None => Reply::not_found(),
        },
    }
}

/// Global counters (`v3.Stats...Sum...`): an add or a read of the total. Unlike the session sums
/// these live in persistent storage so they survive restarts.
pub(in crate::sbg) fn stats_sum(key: &str, action: &Action, store: &Mutex<Store>) -> Reply {
    let storage = lock(store).storage();
    if let Action::SumAdd(delta) = *action {
        return match storage.counter_add(key, delta) {
            Ok(_) => Reply::ok_empty(),
            Err(error) => {
                tracing::warn!(%error, key, "stats counter update failed");
                Reply::not_found()
            }
        };
    }
    match storage.counter(key) {
        Ok(Some(total)) => Reply::document(xsor::sum_value(total)),
        Ok(None) => Reply::not_found(),
        Err(error) => {
            tracing::warn!(%error, key, "stats counter read failed");
            Reply::not_found()
        }
    }
}
