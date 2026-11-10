//! Shared by the integration tests that read the recorded sessions: the GameState requests and
//! replies in their frames, decoded for both builds. `source` reads the crate's own code for the
//! tests that guard it.

#![allow(dead_code)] // each test binary uses its own part

pub mod source;
pub mod title_ssm;

use std::{io::Read, path::PathBuf};

use onevs100_gameserver::{
    gamestate::GameStateDoc,
    sbg::record::{self, Recording},
    time::Utc,
    wire::{self, Protocol, v1149, v1317},
};

/// Every recorded session (`fixtures/sessions/*.sbg`), sorted by path.
pub fn recorded_sessions() -> Vec<(PathBuf, Recording)> {
    recordings_in("fixtures/sessions")
}

/// The recordings where the first round opens twice (`fixtures/sessions/first-round-opens-twice/`):
/// they do not replay exactly and are kept as evidence for the title model.
pub fn first_round_opens_twice_recordings() -> Vec<(PathBuf, Recording)> {
    recordings_in("fixtures/sessions/first-round-opens-twice")
}

fn recordings_in(dir: &str) -> Vec<(PathBuf, Recording)> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .expect(dir)
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sbg"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            let recording = record::parse(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            (path, recording)
        })
        .collect()
}

/// A request on a console's GameState key (`...Session....GameState`, not the data-collection
/// copies).
#[derive(Debug)]
pub struct StateRequest {
    pub cookie: u16,
    /// When the server is to carry it out; `None` = now.
    pub execute_at: Option<Utc>,
    /// `None` for a GET.
    pub posted: Option<GameStateDoc>,
}

pub fn state_request(protocol: Protocol, frame: &[u8]) -> Option<StateRequest> {
    let (frame, _) = wire::decode(frame).ok()??;
    let (key, execute_time, value) = match protocol {
        Protocol::V1317 if frame.kind == v1317::KIND_NAMED_REQUEST => {
            let request = v1317::parse_named_body(&frame.body).ok()?;
            let value = (!request.value.is_empty())
                .then(|| inflate(request.value, request.compressed))
                .flatten();
            (request.key, request.execute_time_utc, value)
        }
        Protocol::V1149 => match v1149::parse_request(frame.kind, &frame.body).ok()?? {
            v1149::Request {
                execute_time_utc,
                op: v1149::Op::Get { key },
                ..
            } => (key, execute_time_utc, None),
            v1149::Request {
                execute_time_utc,
                op: v1149::Op::Set { key, value },
                ..
            } => (key, execute_time_utc, inflate_raw(value)),
            _ => return None,
        },
        _ => return None,
    };
    let key = std::str::from_utf8(key).ok()?.to_ascii_lowercase();
    let state_key = key.starts_with("v3.session.") || key.starts_with("v1.session.");
    if !state_key || !key.ends_with(".gamestate") || key.contains(".round.") {
        return None;
    }
    let posted = match value {
        Some(value) => Some(GameStateDoc::parse(&value)?),
        None => None,
    };
    Some(StateRequest {
        cookie: frame.cookie,
        execute_at: filetime(execute_time),
        posted,
    })
}

fn filetime(filetime: u64) -> Option<Utc> {
    const UNIX_EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;
    let ms = filetime.checked_sub(UNIX_EPOCH_AS_FILETIME)? / 10_000;
    Some(Utc::from_unix_ms(i64::try_from(ms).ok()?))
}

/// A reply frame: its cookie and the GameState it carries (`None`: anything else, NOT_FOUND
/// included).
pub fn replied_state(protocol: Protocol, frame: &[u8]) -> Option<(u16, Option<GameStateDoc>)> {
    let (parsed, _) = wire::decode(frame).ok()??;
    let value = match protocol {
        Protocol::V1317 => {
            let reply = v1317::GenericReply::parse(frame).ok()?;
            inflate(reply.value, reply.compressed)
        }
        Protocol::V1149 if parsed.kind == v1149::KIND_REPLY => {
            inflate_raw(parsed.body.get(v1149::REPLY_PREFIX_LEN..)?)
        }
        Protocol::V1149 => return None,
    };
    Some((
        parsed.cookie,
        value.and_then(|value| GameStateDoc::parse(&value)),
    ))
}

fn inflate(bytes: &[u8], compressed: bool) -> Option<Vec<u8>> {
    if !compressed {
        return Some(bytes.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(bytes)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

/// v2.0.1149.0 store values: a zlib stream without its two-byte header.
fn inflate_raw(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(bytes)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

/// Unix time of a recorded event.
pub fn at(recording: &Recording, at_ms: u64) -> Utc {
    recording.started + std::time::Duration::from_millis(at_ms)
}
