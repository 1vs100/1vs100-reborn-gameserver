//! Plain store requests: GET (setup documents, the bank, launch data) and SET (GameStates
//! through the flow check), the console's error reports.

use std::{sync::Mutex, time::Duration};

use crate::{
    questions::ShowSeed, storage::ErrorReport, store::Store, sync::lock, time::Utc, wire::Protocol,
    xsor,
};

use crate::sbg::{
    conn::{Caller, Ctx},
    handlers::{describe_window, dump_game_state, game_state_get, is_ones_console},
    key::{Key, ParsedKey},
    message::Reply,
    user_error,
};

/// v2.0.1149.0 `v1.Setup.MGS.1vs100.Pangolin.Config` ([`Key::PangolinConfig`]), fetched natively
/// right after register (`sub_82239E20`). `sub_82239A68` wants status 200 and a value inflating to
/// exactly 8 bytes: a big-endian f64 sampling fraction. Pangolin (UDP engine to the SBG host, port
/// 8088) starts when `rand()/32767 < value`; otherwise the client idles in state 2.
/// 0: Pangolin stays off, the backend serves nothing on 8088.
const PANGOLIN_SAMPLE_FRACTION: f64 = 0.0;

/// `...Sessions.<UNSCHEDULED_NEXT_SESSION>.MergedLaunchData`, asked for by 1149's "Next" entry.
fn is_unscheduled_next_launch_data(key: &ParsedKey) -> bool {
    matches!(&key.kind, Key::MergedLaunchData { session }
        if session.as_str().eq_ignore_ascii_case(crate::schedule::UNSCHEDULED_NEXT_SESSION))
}

/// The show a question list is chosen for: the id the console sent, or the key's own id when it
/// already carries the canonical one.
fn show_of(asked: Option<&crate::schedule::ShowRef>, key: &ParsedKey) -> Option<ShowSeed> {
    let id = match asked {
        Some(asked) => asked.id(),
        None => key.show_id()?.id,
    };
    Some(ShowSeed::of(id))
}

pub(in crate::sbg) fn store_get(
    caller: Caller,
    ctx: Ctx,
    cookie: u16,
    parsed: &ParsedKey,
    asked: Option<&crate::schedule::ShowRef>,
    execute_at: Option<Utc>,
) -> Reply {
    let Ctx {
        settings,
        store,
        schedule,
        ..
    } = ctx;
    let key = parsed.raw.as_str();
    let requested_session = match asked {
        Some(crate::schedule::ShowRef::Session(id)) => Some(id.as_str()),
        _ => None,
    };
    if parsed.kind.is_question_list() {
        // Only the debug short form (1317 `DEFAULT_QMS_QBANK`) names no show; it gets no bank list.
        if let Some(show) = show_of(asked, parsed) {
            // A special asks only its category's questions (the show by the session or episode id).
            let category = asked.and_then(|asked| {
                let now = lock(store).now();
                lock(schedule).category_of(now, asked)
            });
            if let Some(value) = lock(store).question_list_get(parsed, category.as_deref(), show) {
                return Reply::document(value);
            }
        }
    } else if let Some(value) = lock(store).question_get(parsed) {
        return Reply::document(value);
    }
    if parsed.kind == Key::Config {
        // Generated per request: the round cut-off depends on whether shows are scheduled.
        let cutoff = {
            let schedule = lock(schedule);
            schedule.is_enabled().then(|| schedule.pt_min_end())
        };
        return Reply::document(xsor::config_fixture(lock(store).served_config(), cutoff));
    }
    if parsed.kind == Key::PangolinConfig {
        return Reply::document(PANGOLIN_SAMPLE_FRACTION.to_be_bytes().to_vec());
    }
    // v2.0.1149.0 `VerifyGridLaunchData` wants Duration/GameType as numbers, 1317.0 as strings.
    let numeric = settings.protocol == Protocol::V1149;
    if requested_session.is_none() && is_unscheduled_next_launch_data(parsed) {
        let next = Some(crate::schedule::UNSCHEDULED_NEXT_SESSION);
        let now = lock(store).now().into();
        let showing = lock(schedule).showing_for_session(now, next);
        showing.log_handed_out("launch data", now, Some(caller.peer));
        return Reply::document(xsor::merged_launch_data_fixture(
            &showing,
            numeric,
            &settings.service_host,
        ));
    }
    if matches!(&parsed.kind, Key::MergedLaunchData { session } if session.is_canonical()) {
        let (stored, now) = {
            let store = lock(store);
            (store.get(key).ok().map(<[u8]>::to_vec), store.now())
        };
        let value = stored.unwrap_or_else(|| {
            let showing = lock(schedule).showing_for_session(now.into(), requested_session);
            showing.log_handed_out("launch data", now.into(), Some(caller.peer));
            xsor::merged_launch_data_fixture(&showing, numeric, &settings.service_host)
        });
        return Reply::document(value);
    }
    let game_state = matches!(parsed.kind, Key::GameState { .. });
    let value = if game_state {
        game_state_get(caller, ctx.host, key, execute_at, store)
    } else {
        store_get_with_wait(key, store)
    };
    match value {
        Ok(value) => {
            if game_state {
                tracing::info!(cookie, state = %describe_window(&value, lock(store).now()), "GameState GET served");
            }
            Reply::document(value)
        }
        Err(_) => Reply::not_found(),
    }
}

pub(in crate::sbg) fn store_set(
    caller: Caller,
    ctx: Ctx,
    cookie: u16,
    parsed: &ParsedKey,
    value: Vec<u8>,
) -> Reply {
    let Ctx {
        settings,
        store,
        host,
        members,
        ..
    } = ctx;
    let key = parsed.raw.as_str();
    tracing::trace!(key, len = value.len(), "SBG store set");
    if parsed.kind == Key::UserError {
        log_user_error(cookie, key, &value);
        save_user_error(caller, store, key, &value);
    }
    let game_state = matches!(parsed.kind, Key::GameState { .. });
    if game_state {
        // A GameState post asks for the host role, as a GameState GET does.
        lock(host).claim_if_vacant(caller.peer);
        tracing::info!(cookie, host = lock(host).is_host(caller.peer), state = %describe_window(&value, lock(store).now()), "GameState SET received");
        dump_game_state(settings, &value, "raw", lock(store).now());
    }
    if game_state && !lock(host).is_host(caller.peer) && {
        // The roster first, then the store: one lock at a time.
        let players = lock(members).players_of(caller.peer).to_vec();
        !is_ones_console(&players, &value, &lock(store), key)
    } {
        // Progression comes from the session host or from the elected One's console (the One
        // posts the states of the round it plays); a follower's timeline is its own local view.
        tracing::info!(cookie, "GameState SET from follower ignored");
        return Reply::ok_empty();
    }
    let value = if game_state {
        let checked = {
            let mut store = lock(store);
            let stored = store.get(key).ok().map(<[u8]>::to_vec);
            let last = store.last_posted_mut();
            crate::gamestate::flow::check_post(
                settings.server_flow,
                last,
                key,
                value,
                stored.as_deref(),
            )
        };
        match checked {
            Some(value) => {
                dump_game_state(settings, &value, "stored", lock(store).now());
                value
            }
            None => return Reply::ok_empty(),
        }
    } else {
        value
    };
    let mut store = lock(store);
    if !store.set(key, value) {
        tracing::warn!(%key, "SBG ignored regressing GameState Set");
    }
    Reply::ok_empty()
}

/// The most error reports saved; older ones are removed as new ones arrive.
const ERROR_REPORTS_KEPT: usize = 10_000;

/// The largest error report saved. The title's reports are near 1 KiB.
const ERROR_REPORT_MAX_BYTES: usize = 16 * 1024;

/// Every report is kept: the store keeps only the latest of each name.
fn save_user_error(caller: Caller, store: &Mutex<Store>, key: &str, value: &[u8]) {
    if value.len() > ERROR_REPORT_MAX_BYTES {
        tracing::warn!(
            key,
            len = value.len(),
            max = ERROR_REPORT_MAX_BYTES,
            "console error report too large to save"
        );
        return;
    }
    let (storage, received) = {
        let store = lock(store);
        (store.storage(), store.now())
    };
    let report = ErrorReport {
        received,
        console: caller.peer,
        key: key.into(),
        document: value.to_vec(),
    };
    if let Err(error) = storage.add_error_report(&report, ERROR_REPORTS_KEPT) {
        tracing::warn!(%error, key, "console error report not saved");
    }
}

/// The title's own error report: what went wrong on the console, as the console saw it.
fn log_user_error(cookie: u16, key: &str, value: &[u8]) {
    let text = user_error::text(value);
    let Some(report) = text.as_deref().and_then(user_error::parse) else {
        tracing::warn!(cookie, key, "console error report in an unknown format");
        return;
    };
    let user_error::UserError {
        name,
        reason,
        fatal,
        utc,
        state,
        substate,
        round,
        question,
        key: about,
    } = report;
    if report.is_fatal() {
        tracing::warn!(
            cookie,
            name,
            reason,
            fatal,
            state,
            substate,
            round,
            question,
            key = about,
            console_utc = utc,
            "console error report"
        );
    } else {
        tracing::info!(
            cookie,
            name,
            reason,
            state,
            substate,
            round,
            question,
            key = about,
            console_utc = utc,
            "console report"
        );
    }
}

pub(in crate::sbg) fn store_get_with_wait(
    key: &str,
    store: &Mutex<Store>,
) -> Result<Vec<u8>, crate::store::GetError> {
    const WAIT_MAX: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(100);
    let clock = lock(store).clock();
    let deadline = clock.now() + WAIT_MAX;
    loop {
        // Bind first: a guard in the `match` scrutinee would stay locked across the sleep
        // below and block the very SET this GET is waiting for.
        let found = lock(store).get(key).map(<[u8]>::to_vec);
        match found {
            Ok(v) => return Ok(v),
            Err(e) => {
                let remaining = deadline.saturating_duration_since(clock.now());
                if remaining.is_zero() {
                    return Err(e);
                }
                clock.sleep(POLL_INTERVAL.min(remaining));
            }
        }
    }
}
