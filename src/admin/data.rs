//! Persistent data: players, export and import.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{ErrorResponse, Query, Reply, query_as, serve_saved_questions};
use crate::{app::App, ids::Xuid, sync::lock, time::Utc};

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ImportParams {
    /// merge (the default: what has the same id is replaced, the rest kept) or replace.
    mode: Option<String>,
}

/// A registered player and the size of what is stored for them.
#[derive(Serialize, ToSchema)]
pub struct PlayerInfo {
    xuid_hex: String,
    /// Absent until the title reports a name.
    gamertag: Option<String>,
    #[serde(
        rename = "first_seen_unix_secs",
        serialize_with = "crate::time::unix_secs::serialize"
    )]
    #[schema(value_type = i64)]
    first_seen: Utc,
    #[serde(
        rename = "last_seen_unix_secs",
        serialize_with = "crate::time::unix_secs::serialize"
    )]
    #[schema(value_type = i64)]
    last_seen: Utc,
    /// Absent until the title uploads an avatar.
    avatar_bytes: Option<usize>,
    user_storage_bytes: usize,
    user_stats_bytes: usize,
}

/// Result of deleting a player.
#[derive(Serialize, ToSchema)]
pub struct DeleteResponse {
    deleted: bool,
}

/// Result of a data import.
#[derive(Serialize, ToSchema)]
pub struct ImportResponse {
    players: usize,
    blobs: usize,
    counters: usize,
    /// Whether the dump carried a programme, which replaced the saved one.
    programme: bool,
    specials: usize,
    shows: usize,
    questions: usize,
}

/// Players in the persistent registry
#[utoipa::path(get, path = "/admin/data/players", responses((status = 200, description = "Registered players (survive restarts) and the size of their stored documents", body = Vec<PlayerInfo>)))]
pub(super) fn data_players(state: &App) -> Reply {
    let storage = lock(&state.store).storage();
    let players = match storage.player_summaries() {
        Ok(players) => players,
        Err(error) => return Reply::error(500, error.to_string()),
    };
    let infos: Vec<_> = players
        .into_iter()
        .map(|player| PlayerInfo {
            xuid_hex: format!("{:016x}", player.xuid),
            user_storage_bytes: player.user_storage_bytes,
            user_stats_bytes: player.user_stats_bytes,
            avatar_bytes: player.avatar_bytes,
            gamertag: player.gamertag,
            first_seen: player.first_seen,
            last_seen: player.last_seen,
        })
        .collect();
    Reply::json(&infos)
}

/// Delete a player and their stored documents
#[utoipa::path(post, path = "/admin/data/players/{xuid}/delete", params(("xuid" = String, Path, description = "XUID as 16 hex digits")), responses((status = 200, description = "Whether a player was removed", body = DeleteResponse), (status = 400, description = "Not a hex XUID", body = ErrorResponse)))]
pub(super) fn data_delete_player(state: &App, xuid: &str) -> Reply {
    let Ok(xuid) = u64::from_str_radix(xuid, 16).map(Xuid) else {
        return Reply::error(400, "xuid must be hex");
    };
    let storage = lock(&state.store).storage();
    match storage.delete_player(xuid) {
        Ok(deleted) => Reply::json(&DeleteResponse { deleted }),
        Err(error) => Reply::error(500, error.to_string()),
    }
}

/// Export all persistent data as JSON
#[utoipa::path(get, path = "/admin/data/export", responses((status = 200, description = "Backend-neutral JSON dump (version 1): players with their documents, global counters", content_type = "application/json", body = Object)))]
pub(super) fn data_export(state: &App) -> Reply {
    let storage = lock(&state.store).storage();
    match storage.export() {
        Ok(dump) => Reply::json(&dump),
        Err(error) => Reply::error(500, error.to_string()),
    }
}

/// Import a JSON dump
#[utoipa::path(post, path = "/admin/data/import", params(ImportParams), request_body(content = Object, content_type = "application/json", description = "A dump produced by /admin/data/export"), responses((status = 200, description = "Counts of what was loaded", body = ImportResponse), (status = 400, description = "Invalid dump or mode; nothing was changed", body = ErrorResponse)))]
pub(super) fn data_import(state: &App, query: &Query, body: &[u8]) -> Reply {
    let mode = match import_mode(query) {
        Ok(mode) => mode,
        Err(reply) => return reply,
    };
    let dump: crate::storage::Dump = match serde_json::from_slice(body) {
        Ok(dump) => dump,
        Err(error) => return Reply::error(400, format!("not a data dump: {error}")),
    };
    let storage = lock(&state.store).storage();
    match storage.import(&dump, mode) {
        Ok(summary) => {
            tracing::info!(?mode, ?summary, "data import");
            // The schedule and the question bank are served from memory: take what is saved now.
            if let Err(error) = lock(&state.schedule).reload() {
                return Reply::error(
                    500,
                    format!("imported, but the schedule not reloaded: {error}"),
                );
            }
            if let Err(reply) = serve_saved_questions(state) {
                return reply;
            }
            Reply::json(&ImportResponse {
                players: summary.players,
                blobs: summary.blobs,
                counters: summary.counters,
                programme: summary.programme,
                specials: summary.specials,
                shows: summary.shows,
                questions: summary.questions,
            })
        }
        Err(error) => Reply::error(400, error.to_string()),
    }
}

pub(super) fn import_mode(query: &Query) -> Result<crate::storage::ImportMode, Reply> {
    use crate::storage::ImportMode;
    match query_as::<ImportParams>(query)?.mode.as_deref() {
        None | Some("merge") => Ok(ImportMode::Merge),
        Some("replace") => Ok(ImportMode::Replace),
        Some(_) => Err(Reply::error(400, "mode must be merge or replace")),
    }
}
