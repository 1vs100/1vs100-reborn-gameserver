//! The server itself: health, logs, timeline, matchmaking sessions and groups, reset.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{ErrorResponse, Query, Reply, non_empty, query_as};
use crate::{app::App, ids::Xuid, logs, matchmaking::MoveError, sync::lock, time::Utc, timeline};

const DEFAULT_LOG_LINES: usize = 200;
const MAX_LOG_LINES: usize = 5000;
const DEFAULT_TIMELINE: usize = 100;
const MAX_TIMELINE: usize = 1000;

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    uptime_secs: u64,
    /// Seconds since any SBG connection last received a byte; `null` if none ever connected.
    sbg_last_activity_secs_ago: Option<u64>,
}

#[derive(Serialize, ToSchema)]
pub struct LogFileInfo {
    name: String,
    size_bytes: u64,
}

/// A connected console.
#[derive(Serialize, ToSchema)]
pub struct SessionMemberInfo {
    /// The first player's XUID: the console's identity.
    xuid_hex: String,
    /// The first player's gamertag.
    gamertag: String,
    /// Every signed-in local player, user 0 first.
    players: Vec<SessionPlayerInfo>,
    last_heartbeat_secs_ago: u64,
    /// Wall-clock join time (unix seconds).
    #[serde(
        rename = "first_seen_unix_secs",
        serialize_with = "crate::time::unix_secs::serialize"
    )]
    #[schema(value_type = u64)]
    first_seen: Utc,
    /// QNet group inside the session (roster, relay and avatars are scoped to it).
    group_id: u32,
}

#[derive(Serialize, ToSchema)]
pub struct SessionPlayerInfo {
    xuid_hex: String,
    gamertag: String,
    /// Size of the last avatar metadata blob uploaded, absent until one is.
    avatar_bytes: Option<usize>,
}

/// A console that left, holding its place in its group for the grace period.
#[derive(Serialize, ToSchema)]
pub struct SessionReservationInfo {
    xuid_hex: String,
    group_id: u32,
    players: usize,
    expires_in_secs: u64,
}

#[derive(Serialize, ToSchema)]
pub struct SessionInfo {
    session_id: String,
    members: Vec<SessionMemberInfo>,
    reservations: Vec<SessionReservationInfo>,
}

/// One entry in the state timeline.
#[derive(Serialize, ToSchema)]
pub struct TimelineEvent {
    /// ISO-8601 / RFC-3339 timestamp in UTC.
    timestamp: String,
    /// Broad category for filtering / colour-coding in the UI.
    category: String,
    /// Human-readable description of what changed.
    description: String,
    /// The program / session id this event relates to, if any.
    program_id: Option<String>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LogTailParams {
    /// Number of trailing lines to return, after filtering (default 200, capped at 5000).
    lines: Option<usize>,
    /// Keep only records at this level or more severe.
    #[param(inline)]
    level: Option<logs::LogLevel>,
    /// Keep only records with a line containing this text, ignoring ASCII case.
    contains: Option<String>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimelineParams {
    /// Maximum events to return (default 100, capped at 1000).
    limit: Option<usize>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GroupsMoveParams {
    /// The console's first player's XUID as 16 hex digits.
    xuid: String,
    /// Target group id (>= 1).
    group: u32,
}

/// Server uptime
#[utoipa::path(get, path = "/admin/health", responses((status = 200, description = "Server uptime", body = HealthResponse)))]
pub(super) fn health(state: &App) -> Reply {
    let now = crate::time::now();
    Reply::json(&HealthResponse {
        uptime_secs: state.started.elapsed().as_secs(),
        sbg_last_activity_secs_ago: state.last_sbg_activity_unix_secs().map(|at| {
            now.saturating_duration_since(Utc::from_unix_secs(at))
                .as_secs()
        }),
    })
}

/// Log files in this session's log directory
#[utoipa::path(get, path = "/admin/logs", responses((status = 200, description = "Log files in this session's log directory", body = Vec<LogFileInfo>)))]
pub(super) fn logs() -> Reply {
    let files: Vec<_> = logs::list()
        .into_iter()
        .map(|(name, size_bytes)| LogFileInfo { name, size_bytes })
        .collect();
    Reply::json(&files)
}

/// Trailing lines of a log file
#[utoipa::path(
    get,
    path = "/admin/log/{name}",
    params(("name" = String, Path, description = "A file name from GET /admin/logs"), LogTailParams),
    responses(
        (status = 200, description = "Trailing lines of the log file, plain text"),
        (status = 400, description = "Invalid log name", body = ErrorResponse),
        (status = 404, description = "No such log file", body = ErrorResponse),
    )
)]
pub(super) fn log_tail(name: &str, query: &Query) -> Reply {
    let params = match query_as::<LogTailParams>(query) {
        Ok(params) => params,
        Err(reply) => return reply,
    };
    let lines = params.lines.unwrap_or(DEFAULT_LOG_LINES).min(MAX_LOG_LINES);
    let filter = logs::TailFilter {
        level: params.level,
        contains: params.contains.as_deref(),
    };
    match logs::tail(name, lines, &filter) {
        Ok(text) => Reply::text(200, text),
        Err(logs::TailError::InvalidName) => Reply::status(400),
        Err(logs::TailError::NotFound) => Reply::status(404),
    }
}

/// Matchmaking sessions and their members
#[utoipa::path(get, path = "/admin/sessions", responses((status = 200, description = "Matchmaking sessions and their members", body = Vec<SessionInfo>)))]
pub(super) fn sessions(state: &App) -> Reply {
    let members = lock(&state.members);
    let mut by_session: BTreeMap<String, SessionInfo> = BTreeMap::new();
    for console in members.snapshot() {
        let players: Vec<_> = console
            .players
            .into_iter()
            .map(|player| SessionPlayerInfo {
                xuid_hex: format!("{:016x}", player.xuid.0),
                gamertag: player.gamertag,
                avatar_bytes: player.avatar_bytes,
            })
            .collect();
        session_entry(&mut by_session, &console.session)
            .members
            .push(SessionMemberInfo {
                xuid_hex: format!("{:016x}", console.xuid.0),
                // A console joins with at least one player, user 0 first.
                gamertag: players
                    .first()
                    .map(|p| p.gamertag.clone())
                    .unwrap_or_default(),
                players,
                last_heartbeat_secs_ago: console.last_heartbeat_secs_ago,
                first_seen: console.first_seen,
                group_id: console.group.get(),
            });
    }
    for reservation in members.reservations_snapshot() {
        session_entry(&mut by_session, &reservation.session)
            .reservations
            .push(SessionReservationInfo {
                xuid_hex: format!("{:016x}", reservation.xuid.0),
                group_id: reservation.group.get(),
                players: reservation.players,
                expires_in_secs: reservation.expires_in_secs,
            });
    }
    drop(members);
    let sessions: Vec<_> = by_session.into_values().collect();
    Reply::json(&sessions)
}

/// The session `id` in `sessions`, added when new.
fn session_entry<'a>(
    sessions: &'a mut BTreeMap<String, SessionInfo>,
    id: &str,
) -> &'a mut SessionInfo {
    sessions
        .entry(id.to_owned())
        .or_insert_with(|| SessionInfo {
            session_id: id.to_owned(),
            members: Vec::new(),
            reservations: Vec::new(),
        })
}

/// What `POST /admin/reset` dropped and what it left alone.
#[derive(Serialize, ToSchema)]
pub struct ResetInfo {
    cleared: Vec<&'static str>,
    kept: Vec<&'static str>,
}

/// Drop all volatile state (show, election, GameState host) as if the server had just started
#[utoipa::path(post, path = "/admin/reset", responses((status = 200, description = "What was cleared and what was kept", body = ResetInfo)))]
pub(super) fn reset_volatile(state: &App) -> Reply {
    crate::sbg::reset_volatile(state);
    lock(&state.members).clear_reservations();
    timeline::record("reset", "volatile state reset", None);
    Reply::json(&ResetInfo {
        cleared: vec![
            "store values, sums and TopN boards",
            "game flow bookkeeping",
            "GameState host",
            "active scheduled session",
            "matchmaking reservations",
        ],
        kept: vec![
            "player documents",
            "global counters",
            "programme, special episodes and planned shows",
            "connected matchmaking consoles and their groups",
            "options",
        ],
    })
}

/// Put a console into another QNet group; both groups get an ANNOUNCE
#[utoipa::path(post, path = "/admin/groups/move", params(GroupsMoveParams), responses((status = 200, description = "Moved"), (status = 400, description = "Bad xuid or group", body = ErrorResponse), (status = 404, description = "No such console", body = ErrorResponse), (status = 409, description = "The group has no room for the console's players", body = ErrorResponse)))]
pub(super) fn groups_move(state: &App, query: &Query) -> Reply {
    let params = match query_as::<GroupsMoveParams>(query) {
        Ok(params) => params,
        Err(reply) => return reply,
    };
    let xuid = match non_empty(params.xuid, "xuid").map(|v| u64::from_str_radix(&v, 16)) {
        Ok(Ok(xuid)) => Xuid(xuid),
        Ok(Err(_)) => return Reply::error(400, "xuid must be hex"),
        Err(reply) => return reply,
    };
    let group = params.group;
    match crate::matchmaking::move_console(&state.members, xuid, group) {
        Ok(()) => Reply::text(200, "moved"),
        Err(error) => {
            let code = match error {
                MoveError::NoSuchConsole { .. } => 404,
                MoveError::InvalidGroup { .. } => 400,
                MoveError::GroupFull { .. } => 409,
            };
            Reply::error(code, error.to_string())
        }
    }
}

/// Recent state-change events, newest first
#[utoipa::path(get, path = "/admin/timeline", params(TimelineParams), responses((status = 200, description = "Recent state-change events, newest first", body = Vec<TimelineEvent>)))]
pub(super) fn timeline(query: &Query) -> Reply {
    let limit = match query_as::<TimelineParams>(query).map(|params| params.limit) {
        Ok(limit) => limit.unwrap_or(DEFAULT_TIMELINE).min(MAX_TIMELINE),
        Err(reply) => return reply,
    };
    let events: Vec<_> = timeline::recent(limit)
        .into_iter()
        .map(|event| TimelineEvent {
            timestamp: event.timestamp,
            category: event.category.to_owned(),
            description: event.description,
            program_id: event.program_id,
        })
        .collect();
    Reply::json(&events)
}
