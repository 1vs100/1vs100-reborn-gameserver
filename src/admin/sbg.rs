//! The captured SBG frames, decoded, and the recording files.

use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{ErrorResponse, Query, Reply, query_as};
use crate::{
    app::App,
    ids::{ConnId, Xuid},
    sbg::{
        capture::{self, RecordingStatus, StartError},
        record::Kind,
        replay,
    },
};

/// Events listed unless the request asks for another number.
const DEFAULT_EVENTS: usize = 500;
/// The most events one request may ask for.
const MAX_EVENTS: usize = 5000;
/// The recording files' directory below the session log directory.
const RECORDINGS_DIR: &str = "sbg";
const RECORDING_SUFFIX: &str = ".sbg";

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SbgEventsParams {
    /// Keep events at or after this time.
    since_unix_ms: Option<i64>,
    /// Keep events at or before this time.
    until_unix_ms: Option<i64>,
    /// Keep the events of this SBG connection.
    conn: Option<u64>,
    /// Keep the events of this console address.
    #[param(value_type = Option<String>)]
    peer: Option<IpAddr>,
    /// Keep the events of the console whose first player has this XUID, as 16 hex digits.
    xuid: Option<String>,
    /// Keep requests whose key contains this text, ignoring ASCII case, and their replies.
    key: Option<String>,
    /// Maximum events to return, the newest that match, oldest first (default 500, capped at
    /// 5000).
    limit: Option<usize>,
}

#[derive(Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SbgEventKind {
    Open,
    Close,
    In,
    Out,
}

/// One captured SBG event, decoded.
#[derive(Serialize, ToSchema)]
pub struct SbgEventInfo {
    /// Later events have larger numbers.
    seq: u64,
    /// On the clock of the server's log lines.
    at_unix_ms: i64,
    conn: u64,
    peer: Option<String>,
    /// The console's first player when the event was captured, as 16 hex digits.
    xuid_hex: Option<String>,
    kind: SbgEventKind,
    /// Pairs a request with its reply on one connection.
    cookie: Option<u16>,
    /// The key a request names; on a reply, the key of its request.
    key: Option<String>,
    /// The event decoded: action, key and value of a request, status and value of a reply.
    text: String,
}

/// A recording file being written.
#[derive(Serialize, ToSchema)]
pub struct SbgRecordingInfo {
    /// The file name below `/admin/sbg/recordings/`; absent for a file outside the session log
    /// directory (`GAMESERVER_RECORD_SBG`).
    name: Option<String>,
    path: String,
    started_unix_ms: i64,
    events: u64,
}

/// What the capture holds.
#[derive(Serialize, ToSchema)]
pub struct SbgCaptureInfo {
    recent_events: usize,
    recent_bytes: usize,
    recent_capacity_bytes: usize,
    /// When the oldest kept event happened; absent when none is kept.
    oldest_unix_ms: Option<i64>,
    /// Events dropped since the server started, because the capture fell behind.
    dropped: u64,
    recording: Option<SbgRecordingInfo>,
    /// The last recording that stopped on a write error.
    failure: Option<SbgRecordingFailure>,
}

/// A recording the server stopped because a write failed.
#[derive(Serialize, ToSchema)]
pub struct SbgRecordingFailure {
    path: String,
    error: String,
}

#[derive(Serialize, ToSchema)]
pub struct SbgRecordingFileInfo {
    name: String,
    size_bytes: u64,
}

fn recordings_dir() -> Option<PathBuf> {
    crate::logs::session_dir().map(|dir| dir.join(RECORDINGS_DIR))
}

/// The name of a file in the recordings directory; only names this server gives are accepted,
/// so a request can never leave the directory.
fn recording_name(path: &Path) -> Option<String> {
    let dir = recordings_dir()?;
    let name = path.strip_prefix(&dir).ok()?.to_str()?;
    is_recording_name(name).then(|| name.to_owned())
}

fn is_recording_name(name: &str) -> bool {
    name.strip_suffix(RECORDING_SUFFIX)
        .is_some_and(|stem| !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit()))
}

fn recording_info(status: RecordingStatus) -> SbgRecordingInfo {
    SbgRecordingInfo {
        name: recording_name(&status.path),
        path: status.path.display().to_string(),
        started_unix_ms: status.started.unix_ms(),
        events: status.events,
    }
}

/// What the SBG capture holds
#[utoipa::path(get, path = "/admin/sbg/capture", responses((status = 200, description = "The kept SBG events and the recording being written", body = SbgCaptureInfo)))]
pub(super) fn sbg_capture(state: &App) -> Reply {
    let status = state.capture.status();
    Reply::json(&SbgCaptureInfo {
        recent_events: status.recent_events,
        recent_bytes: status.recent_bytes,
        recent_capacity_bytes: status.recent_capacity_bytes,
        oldest_unix_ms: status.oldest.map(|at| at.unix_ms()),
        dropped: status.dropped,
        recording: status.recording.map(recording_info),
        failure: status.failure.map(|failure| SbgRecordingFailure {
            path: failure.path.display().to_string(),
            error: failure.error,
        }),
    })
}

/// The kept SBG events that match, decoded
#[utoipa::path(get, path = "/admin/sbg/events", params(SbgEventsParams), responses((status = 200, description = "The newest `limit` matching events, oldest first", body = Vec<SbgEventInfo>), (status = 400, description = "A parameter does not parse", body = ErrorResponse)))]
pub(super) fn sbg_events(state: &App, query: &Query) -> Reply {
    let params = match query_as::<SbgEventsParams>(query) {
        Ok(params) => params,
        Err(reply) => return reply,
    };
    let xuid = match params
        .xuid
        .as_deref()
        .map(|hex| u64::from_str_radix(hex, 16))
    {
        None => None,
        Some(Ok(xuid)) => Some(Xuid(xuid)),
        Some(Err(_)) => return Reply::error(400, "xuid must be hex"),
    };
    let since = params.since_unix_ms;
    let until = params.until_unix_ms;
    let conn = params.conn.map(ConnId);
    let peer = params.peer;
    // An empty key names every key, as the empty text is in every one.
    let key = params.key.as_deref().filter(|part| !part.is_empty());
    let limit = params.limit.unwrap_or(DEFAULT_EVENTS).min(MAX_EVENTS);
    let protocol = state.sbg.protocol;
    let events: Vec<SbgEventInfo> = state
        .capture
        .newest(limit, |event| {
            let at = event.at.unix_ms();
            since.is_none_or(|since| at >= since)
                && until.is_none_or(|until| at <= until)
                && conn.is_none_or(|conn| conn == event.conn)
                && peer.is_none_or(|peer| event.peer == Some(peer))
                && xuid.is_none_or(|xuid| event.xuid == Some(xuid))
                && key.is_none_or(|part| {
                    event
                        .key
                        .as_deref()
                        .is_some_and(|key| contains_ignoring_ascii_case(key, part))
                })
        })
        .into_iter()
        .map(|event| SbgEventInfo {
            seq: event.seq.0,
            at_unix_ms: event.at.unix_ms(),
            conn: event.conn.0,
            peer: event.peer.map(|peer| peer.to_string()),
            xuid_hex: event.xuid.map(|xuid| format!("{:016x}", xuid.0)),
            kind: match event.kind {
                Kind::Open => SbgEventKind::Open,
                Kind::Close => SbgEventKind::Close,
                Kind::In => SbgEventKind::In,
                Kind::Out => SbgEventKind::Out,
            },
            cookie: event.cookie,
            key: event.key.as_deref().map(str::to_owned),
            text: replay::summary(protocol, event.kind, &event.frame),
        })
        .collect();
    Reply::json(&events)
}

fn contains_ignoring_ascii_case(text: &str, part: &str) -> bool {
    text.as_bytes()
        .windows(part.len())
        .any(|window| window.eq_ignore_ascii_case(part.as_bytes()))
}

/// Start writing every SBG frame to a new file
#[utoipa::path(post, path = "/admin/sbg/recording/start", responses((status = 200, description = "The recording started, in the session log directory", body = SbgRecordingInfo), (status = 409, description = "A recording is open already", body = ErrorResponse), (status = 500, description = "The file or the storage snapshot failed", body = ErrorResponse), (status = 503, description = "The server has no session log directory", body = ErrorResponse)))]
pub(super) fn sbg_recording_start(state: &App) -> Reply {
    let Some(dir) = recordings_dir() else {
        return Reply::error(503, "no session log directory to record into");
    };
    if let Err(error) = std::fs::create_dir_all(&dir) {
        return Reply::error(500, format!("create {}: {error}", dir.display()));
    }
    let name = format!("{}{RECORDING_SUFFIX}", crate::time::now().unix_ms());
    match capture::start_recording(state, &dir.join(name), capture::ExistingFile::Refuse) {
        Ok(status) => Reply::json(&recording_info(status)),
        Err(error @ StartError::Recording { .. }) => Reply::error(409, error.to_string()),
        Err(error) => Reply::error(500, error.to_string()),
    }
}

/// Close the recording file
#[utoipa::path(post, path = "/admin/sbg/recording/stop", responses((status = 200, description = "The recording that was closed", body = SbgRecordingInfo), (status = 409, description = "No recording is open", body = ErrorResponse)))]
pub(super) fn sbg_recording_stop(state: &App) -> Reply {
    match state.capture.stop() {
        Some(status) => Reply::json(&recording_info(status)),
        None => Reply::error(409, "no recording is open"),
    }
}

/// The recording files in the session log directory
#[utoipa::path(get, path = "/admin/sbg/recordings", responses((status = 200, description = "Recording files, oldest first", body = Vec<SbgRecordingFileInfo>)))]
pub(super) fn sbg_recordings() -> Reply {
    let mut files: Vec<SbgRecordingFileInfo> = recordings_dir()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let meta = entry.metadata().ok()?;
            (meta.is_file() && is_recording_name(&name)).then_some(SbgRecordingFileInfo {
                name,
                size_bytes: meta.len(),
            })
        })
        .collect();
    // Names are start times; shorter is older.
    files.sort_by(|a, b| (a.name.len(), &a.name).cmp(&(b.name.len(), &b.name)));
    Reply::json(&files)
}

/// One recording file, for `sbg_replay`
#[utoipa::path(
    get,
    path = "/admin/sbg/recordings/{name}",
    params(("name" = String, Path, description = "A file name from GET /admin/sbg/recordings")),
    responses(
        (status = 200, description = "The recording, plain text"),
        (status = 400, description = "Not a recording file name", body = ErrorResponse),
        (status = 404, description = "No such recording", body = ErrorResponse),
    )
)]
pub(super) fn sbg_recording_file(name: &str) -> Reply {
    if !is_recording_name(name) {
        return Reply::status(400);
    }
    match recordings_dir().map(|dir| std::fs::read(dir.join(name))) {
        Some(Ok(text)) => Reply::text(200, text),
        _ => Reply::status(404),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_names_this_server_gives_are_recordings() {
        assert!(is_recording_name("1794235993756.sbg"));
        assert!(!is_recording_name(".sbg"));
        assert!(!is_recording_name("../1.sbg"));
        assert!(!is_recording_name("1.sbg.txt"));
    }
}
