//! A decoded request to its handler, by key and action.

use std::time::SystemTime;

use crate::{sync::lock, time::Utc};

use crate::sbg::{
    conn::{Caller, Ctx},
    handlers::{stats_sum, store_get, store_set, sum, top_n_get, top_n_post, user_get, user_set},
    key::{ElectionItem, Key, ParsedKey},
    message::{Action, Reply},
    reset_show_state,
};

/// `execute_at`: v2.0.1149.0's `ExecuteTimeUtc` (None = now, and always for 1317).
pub(in crate::sbg) fn dispatch(
    caller: Caller,
    ctx: Ctx,
    cookie: u16,
    key: &str,
    action: Action,
    execute_at: Option<SystemTime>,
) -> Reply {
    let store = ctx.store;
    let key = ParsedKey::parse(key);
    // Scheduled shows carry their own session id in every key; the store logic works on one
    // canonical id, and a new show drops the previous show's session state.
    let scheduled = lock(ctx.schedule).is_enabled();
    let (key, asked) = if scheduled {
        crate::schedule::canonicalize_key(key)
    } else {
        (key, None)
    };
    let session = match &asked {
        Some(crate::schedule::ShowRef::Session(id)) => Some(id.clone()),
        _ => None,
    };
    let now = lock(store).now();
    if let Some(session) = &session
        && matches!(key.kind, Key::GameState { .. })
        && lock(ctx.schedule).activate_session(now.into(), session)
    {
        // A new show starts clean, host role included: its consoles elect again.
        tracing::info!(%session, "new scheduled show: show state and host role reset");
        reset_show_state(store);
        lock(ctx.host).forget_roles();
    }
    // Consoles key one election under different local-clock ids; fold them into one.
    let candidacy = matches!(action, Action::TopNPost { .. })
        && matches!(
            key.kind,
            Key::Election {
                item: ElectionItem::TheOne,
                ..
            }
        );
    let sent = key.raw.clone();
    let key = lock(store).fold_election(key, candidacy, now);
    if key.raw != sent {
        tracing::info!(key = %sent, canonical = %key.raw, "SBG election key folded into the round's first id");
    }
    match action {
        Action::UserGet { xuid } => user_get(&key, xuid, store),
        Action::UserSet { xuid, value } => user_set(&key, xuid, &value, store),
        Action::SumGet | Action::SumAdd(_) if key.kind == Key::StatsSum => {
            stats_sum(&key.raw, &action, store)
        }
        Action::SumGet | Action::SumAdd(_) => sum(&key, &action, execute_at, store),
        Action::TopNGet => top_n_get(&key, store),
        Action::TopNPost {
            capacity,
            score,
            metadata,
        } => top_n_post(&key.raw, capacity, score, metadata, store),
        Action::Get => store_get(
            caller,
            ctx,
            cookie,
            &key,
            asked.as_ref(),
            execute_at.map(Utc::from),
        ),
        Action::Set(value) => store_set(caller, ctx, cookie, &key, value),
    }
}
