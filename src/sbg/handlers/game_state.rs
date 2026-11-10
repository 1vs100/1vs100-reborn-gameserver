//! GameState requests: the GET held until its execute time, the One's console, the dumps.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{
    gamestate::{GameStateDoc, hold},
    ids::Xuid,
    store::Store,
    sync::lock,
    time::Utc,
};

use crate::sbg::{Settings, conn::*, host::HostAuthority};

/// True when this SET carries the XUID of a player the posting console joined matchmaking with
/// (that console is The One of the state it posts).
///
/// A mid-round claim is only believed when it matches the One of the state in progress; at a round
/// boundary (stored state `RoundStart`/`ElectHost`, or nothing yet) a new One may be crowned.
/// Otherwise a console that (re)joins late and elects itself would overwrite a running round.
/// `players`: the posting console's, from the roster.
pub(in crate::sbg) fn is_ones_console(
    players: &[Xuid],
    value: &[u8],
    store: &Store,
    key: &str,
) -> bool {
    // The One can be any local player of the console, not only the one it is known by.
    let Some(xuid) = GameStateDoc::parse(value)
        .and_then(|doc| doc.ones_xuid().map(|ones| ones.into_owned()))
        .filter(|ones| ones.parse().is_ok_and(|ones| players.contains(&Xuid(ones))))
    else {
        return false;
    };
    let Some(stored) = store.get(key).ok().and_then(GameStateDoc::parse) else {
        return true;
    };
    matches!(
        stored.state(),
        Some(
            crate::gamestate::GameStateName::RoundStart
                | crate::gamestate::GameStateName::ElectHost
        )
    ) || stored.ones_xuid().is_none_or(|ones| ones == xuid)
}

/// Write a posted GameState document for offline comparison (`Settings::dump_dir`).
pub(in crate::sbg) fn dump_game_state(settings: &Settings, value: &[u8], tag: &str, now: Utc) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let Some(dir) = &settings.dump_dir else {
        return;
    };
    let name = describe_window(value, now);
    let state = name.split_whitespace().next().unwrap_or("unknown");
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    if let Err(error) = std::fs::create_dir_all(dir) {
        tracing::warn!(%error, dir = %dir.display(), "cannot create the GameState dump directory");
        return;
    }
    let path = dir.join(format!("{n:03}-{state}-{tag}.xml"));
    if let Err(error) = std::fs::write(&path, value) {
        tracing::warn!(%error, path = %path.display(), "cannot write the GameState dump");
    }
}

/// GET a GameState key the way the title's `ServerStateManager` expects ([`hold`]): answered at
/// its execute time with the state stored then. When the stored state would look stale to the
/// title, or there is none, the next state has not been posted yet; that is waited for up to
/// [`hold::wait_until`], then the answer is what is stored. A stale answer makes the title run an ElectHost, its recovery from a lost One. A GET
/// without an execute time is answered at once.
pub(in crate::sbg) fn game_state_get(
    caller: Caller,
    host: &Mutex<HostAuthority>,
    key: &str,
    execute_at: Option<Utc>,
    store: &Mutex<Store>,
) -> Result<Vec<u8>, crate::store::GetError> {
    const POLL_INTERVAL: Duration = Duration::from_millis(100);

    let clock = lock(store).clock();
    let now = clock.now();
    let execute_at = hold::execute_time(execute_at, now);
    if let Some(at) = execute_at {
        let wait = at.saturating_duration_since(now);
        if !wait.is_zero() {
            tracing::trace!(
                key,
                wait_ms = wait.as_millis() as u64,
                "GameState GET held until its execute time"
            );
            clock.sleep(wait);
        }
    }
    // The first console to ask claims the host role.
    lock(host).claim_if_vacant(caller.peer);
    loop {
        let current = lock(store).get(key).map(<[u8]>::to_vec);
        let Some(at) = execute_at else {
            return current;
        };
        let stale = hold::stale(current.as_deref().ok(), at);
        let remaining = hold::wait_until(at).saturating_duration_since(clock.now());
        if !stale || remaining.is_zero() {
            if stale {
                tracing::info!(
                    key,
                    "GameState GET answered without a next state: the title will run an ElectHost"
                );
            }
            return current;
        }
        clock.sleep(POLL_INTERVAL.min(remaining));
    }
}

/// `Name [start..end]` in seconds relative to now, for correlating GETs with each console's log.
pub(in crate::sbg) fn describe_window(value: &[u8], now: Utc) -> String {
    let now = now.as_secs_f64();
    let window = GameStateDoc::parse(value).and_then(|doc| {
        let (start, end) = (doc.start()?, doc.end()?);
        Some(format!(
            "{} [{:+.0}s..{:+.0}s]",
            doc.name(),
            start.as_secs_f64() - now,
            end.as_secs_f64() - now
        ))
    });
    window.unwrap_or_else(|| "unparsed".to_owned())
}
