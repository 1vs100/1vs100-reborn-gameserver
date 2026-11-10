//! Replay an SBG recording against a fresh store; report replies that differ.
//! Runs on a virtual clock at recorded time; UTC stamps are masked before compare.
//! A replay never plans shows; recordings without materialised shows are refused.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    io::Read,
    net::IpAddr,
    sync::Arc,
    time::Duration,
};

use flate2::read::ZlibDecoder;

use super::{
    message, record,
    sim::{Sim, SimError},
};
use crate::{
    ids::{ConnId, Xuid},
    storage::{ImportMode, MemoryStorage, Storage, StorageError},
    wire::{self, Protocol, v1149, v1317},
};

/// One reply that differs from the recording (or is missing on either side).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub conn: ConnId,
    /// The cookie that pairs a request with its reply; `None` for a frame too short to carry one.
    pub cookie: Option<u16>,
    /// Normalised recorded reply (`None`: the replay answered, the recording did not).
    pub expected: Option<String>,
    /// Normalised replayed reply (`None`: the recording answered, the replay did not).
    pub actual: Option<String>,
}

/// Why a recording could not be replayed.
#[derive(Debug)]
pub enum ReplayError {
    /// The recording was made with a show schedule, and a recording does not carry it.
    UnrecordedSchedule,
    /// The recorded storage snapshot does not import.
    Storage(StorageError),
    /// A connection was opened without the peer address a replay needs.
    NoPeer { conn: ConnId },
    /// The simulated server could not be set up or driven.
    Sim(SimError),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnrecordedSchedule => f.write_str(
                "recorded with a show schedule: the schedule is not recorded, so the replay cannot \
                 serve it",
            ),
            Self::Storage(error) => write!(f, "recorded storage: {error}"),
            Self::NoPeer { conn } => write!(f, "conn {conn} opened without a peer address"),
            Self::Sim(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Sim(error) => Some(error),
            Self::UnrecordedSchedule | Self::NoPeer { .. } => None,
        }
    }
}

impl From<SimError> for ReplayError {
    fn from(error: SimError) -> Self {
        Self::Sim(error)
    }
}

pub fn replay(recording: &record::Recording) -> Result<Vec<Mismatch>, ReplayError> {
    let settings = recording.settings;
    let carries_programme = recording
        .storage
        .as_ref()
        .is_some_and(|dump| dump.programme.is_some());
    if settings.schedule && !carries_programme {
        return Err(ReplayError::UnrecordedSchedule);
    }
    let mut served = super::Settings::new(settings.protocol);
    served.server_flow = settings.server_flow;
    if let Some(host) = &recording.service_host {
        served.service_host.clone_from(host);
    }
    let storage = Arc::new(MemoryStorage::default());
    if let Some(dump) = &recording.storage {
        storage
            .import(dump, ImportMode::Replace)
            .map_err(ReplayError::Storage)?;
    }
    let recorded_start = recording.started;
    let mut sim = Sim::new(served, settings.options, storage, recorded_start)?
        .with_served_config(settings.served);
    let mut received = Vec::new();
    for event in &recording.events {
        sim.advance_to(recorded_start + Duration::from_millis(event.at_ms));
        match event.kind {
            record::Kind::Open => {
                // The server closes a socket without a peer address; the replay refuses it.
                let peer = event.peer.ok_or(ReplayError::NoPeer { conn: event.conn })?;
                sim.open(event.conn, peer);
            }
            record::Kind::Close => sim.close(event.conn),
            record::Kind::Out => {}
            record::Kind::In => {
                if let (Some(peer), Some(xuid)) = (event.peer, event.xuid) {
                    sim.note_console_xuid(peer, xuid);
                }
                sim.receive(event.conn, &event.frame)?;
            }
        }
        received.extend(sim.replies());
    }
    // Held requests unanswered on the wire stay unanswered in the replay.

    let expected = recording
        .events
        .iter()
        .filter(|event| event.kind == record::Kind::Out)
        .map(|event| (event.conn, event.frame.clone()));
    Ok(compare(settings.protocol, expected, received.into_iter()))
}

/// Pair replies by connection and cookie (in order) and report the differing ones. A frame too
/// short to carry a cookie cannot be paired, so it is reported on its own.
fn compare(
    protocol: Protocol,
    expected: impl Iterator<Item = (ConnId, Vec<u8>)>,
    actual: impl Iterator<Item = (ConnId, Vec<u8>)>,
) -> Vec<Mismatch> {
    let mut mismatches = Vec::new();
    let mut pending: HashMap<(ConnId, u16), Vec<String>> = HashMap::new();
    for (conn, frame) in actual {
        let text = normalize(protocol, &frame);
        match frame_cookie(&frame) {
            Some(cookie) => pending.entry((conn, cookie)).or_default().push(text),
            None => mismatches.push(Mismatch {
                conn,
                cookie: None,
                expected: None,
                actual: Some(text),
            }),
        }
    }
    for (conn, frame) in expected {
        let want = normalize(protocol, &frame);
        let Some(cookie) = frame_cookie(&frame) else {
            mismatches.push(Mismatch {
                conn,
                cookie: None,
                expected: Some(want),
                actual: None,
            });
            continue;
        };
        let got = pending.get_mut(&(conn, cookie)).and_then(|queue| {
            let position = queue.iter().position(|got| *got == want).unwrap_or(0);
            // Without a match the first queued frame stands in, so the mismatch shows both.
            (!queue.is_empty()).then(|| queue.remove(position))
        });
        if got.as_ref() != Some(&want) {
            mismatches.push(Mismatch {
                conn,
                cookie: Some(cookie),
                expected: Some(want),
                actual: got,
            });
        }
    }
    for ((conn, cookie), extra) in pending {
        for actual in extra {
            mismatches.push(Mismatch {
                conn,
                cookie: Some(cookie),
                expected: None,
                actual: Some(actual),
            });
        }
    }
    mismatches.sort_by_key(|m| (m.conn, m.cookie));
    mismatches
}

/// The cookie of a recorded frame, which pairs a request with its reply.
pub fn frame_cookie(frame: &[u8]) -> Option<u16> {
    frame.get(6..8).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

/// The store key a recorded request frame names.
pub fn request_key(frame: &[u8]) -> Option<String> {
    let (parsed, _) = wire::decode(frame).ok()??;
    match message::decode(&parsed) {
        Ok(message::Message::Request { key, .. }) => Some(key),
        _ => None,
    }
}

/// A reply as text that compares equal across runs: [`reply_text`] with UTC timestamps masked.
pub fn normalize(protocol: Protocol, frame: &[u8]) -> String {
    mask_utc(&reply_text(protocol, frame, compared_text))
}

/// A reply as text: kind, status, decompressed value as `value_text` renders it.
fn reply_text(protocol: Protocol, frame: &[u8], value_text: fn(Vec<u8>) -> String) -> String {
    let Ok(Some((parsed, _))) = wire::decode(frame) else {
        return format!("raw {}", hex(frame));
    };
    let (status, value) = match (protocol, parsed.kind) {
        (Protocol::V1317, v1317::KIND_GENERIC_REPLY) => match v1317::GenericReply::parse(frame) {
            Ok(reply) if reply.compressed => (Some(reply.status), inflate(reply.value)),
            Ok(reply) => (Some(reply.status), Some(reply.value.to_vec())),
            Err(_) => (None, None),
        },
        (Protocol::V1149, v1149::KIND_REPLY) if parsed.body.len() >= v1149::REPLY_PREFIX_LEN => {
            let value = &parsed.body[v1149::REPLY_PREFIX_LEN..];
            // Store documents go out as zlib without its 2-byte header, user documents raw.
            let mut zlib = vec![0x78, 0x01];
            zlib.extend_from_slice(value);
            let value = (!value.is_empty())
                .then(|| inflate(&zlib))
                .flatten()
                .unwrap_or_else(|| value.to_vec());
            (
                Some(u16::from_be_bytes([parsed.body[0], parsed.body[1]])),
                Some(value),
            )
        }
        _ => (None, None),
    };
    let value = value.unwrap_or_else(|| parsed.body.clone());
    let value = value_text(value);
    match status {
        Some(status) => format!("kind={:#04x} status={status} {value}", parsed.kind),
        None => format!("kind={:#04x} {value}", parsed.kind),
    }
}

/// Which events [`list`] shows. Filters combine; an absent one does not filter.
#[derive(Clone, Debug, Default)]
pub struct ListFilter {
    /// Connections by number.
    pub conn: Option<ConnId>,
    /// Connections that carried this XUID.
    pub xuid: Option<Xuid>,
    /// Connections from this console address.
    pub peer: Option<IpAddr>,
    /// Requests whose key contains this text (any case), with their replies.
    pub key: Option<String>,
}

/// The events of a recording that `filter` selects, each as [`describe`] gives it.
pub fn list<'a>(
    recording: &'a record::Recording,
    filter: &ListFilter,
) -> impl Iterator<Item = String> + 'a {
    let conns: HashSet<ConnId> = recording
        .events
        .iter()
        .filter(|event| {
            filter.conn.is_none_or(|conn| conn == event.conn)
                && filter.xuid.is_none_or(|xuid| event.xuid == Some(xuid))
                && filter.peer.is_none_or(|peer| event.peer == Some(peer))
        })
        .map(|event| event.conn)
        .collect();
    let part = filter.key.as_ref().map(|part| part.to_ascii_lowercase());
    // Requests selected by key whose reply has not been seen yet, by connection and cookie.
    let mut awaiting: HashSet<(ConnId, u16)> = HashSet::new();
    let protocol = recording.settings.protocol;
    recording
        .events
        .iter()
        .filter(move |event| {
            if !conns.contains(&event.conn) {
                return false;
            }
            let Some(part) = &part else {
                return true;
            };
            let pair = frame_cookie(&event.frame).map(|cookie| (event.conn, cookie));
            match event.kind {
                record::Kind::In => {
                    let asked = request_key(&event.frame)
                        .is_some_and(|key| key.to_ascii_lowercase().contains(part.as_str()));
                    if asked && let Some(pair) = pair {
                        awaiting.insert(pair);
                    }
                    asked
                }
                record::Kind::Out => pair.is_some_and(|pair| awaiting.remove(&pair)),
                record::Kind::Open | record::Kind::Close => false,
            }
        })
        .map(move |event| describe(protocol, event))
}

/// One recorded event as text: time, connection, and what it carried. A request is listed by
/// cookie, action, key and what it writes; a reply by cookie, status and value.
pub fn describe(protocol: Protocol, event: &record::Event) -> String {
    let head = format!("{:>9} conn={}", event.at_ms, event.conn);
    format!("{head} {}", summary(protocol, event.kind, &event.frame))
}

/// What one event carried, decoded: `open`, `close`, or the direction and the frame as
/// [`describe`] lists it.
pub fn summary(protocol: Protocol, kind: record::Kind, frame: &[u8]) -> String {
    match kind {
        record::Kind::Open => "open".to_owned(),
        record::Kind::Close => "close".to_owned(),
        record::Kind::Out => match frame_cookie(frame) {
            Some(cookie) => format!(
                "out cookie={cookie:#06x} {}",
                reply_text(protocol, frame, displayed_text)
            ),
            None => format!("out {}", reply_text(protocol, frame, displayed_text)),
        },
        record::Kind::In => format!("in {}", request_text(frame)),
    }
}

fn request_text(frame: &[u8]) -> String {
    let Ok(Some((parsed, _))) = wire::decode(frame) else {
        return format!("raw {}", hex(frame));
    };
    let cookie = parsed.cookie;
    match message::decode(&parsed) {
        Ok(message::Message::Request { key, action, .. }) => {
            let name = action.name();
            let written = match action {
                message::Action::Get | message::Action::SumGet | message::Action::TopNGet => {
                    String::new()
                }
                message::Action::Set(value) => format!(" {}", displayed_text(value)),
                message::Action::SumAdd(delta) => format!(" delta={delta}"),
                message::Action::TopNPost {
                    capacity,
                    score,
                    metadata,
                } => format!(
                    " capacity={capacity} score={score} {}",
                    displayed_text(metadata)
                ),
                message::Action::UserGet { xuid } => format!(" xuid={:016x}", xuid.0),
                message::Action::UserSet { xuid, value } => {
                    format!(" xuid={:016x} {}", xuid.0, displayed_text(value))
                }
            };
            format!("cookie={cookie:#06x} {name} {key}{written}")
        }
        Ok(other) => format!("cookie={cookie:#06x} {other:?}"),
        Err(error) => format!("cookie={cookie:#06x} kind={:#04x} {error:?}", parsed.kind),
    }
}

/// A value as the replay compares it: UTF-8 as text, anything else as hex.
fn compared_text(value: Vec<u8>) -> String {
    match String::from_utf8(value) {
        Ok(text) => text,
        Err(bytes) => format!("hex:{}", hex(bytes.as_bytes())),
    }
}

/// A value as a listing shows it: text without its trailing NULs (1317 values end in one), or
/// hex when it is not printable text.
fn displayed_text(value: Vec<u8>) -> String {
    let end = value
        .iter()
        .rposition(|&b| b != 0)
        .map_or(0, |last| last + 1);
    match std::str::from_utf8(&value[..end]) {
        Ok(text)
            if (end > 0 || value.is_empty())
                && !text.chars().any(|c| c.is_control() && !c.is_whitespace()) =>
        {
            text.to_owned()
        }
        _ => format!("hex:{}", hex(&value)),
    }
}

fn inflate(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    ZlibDecoder::new(bytes).read_to_end(&mut out).ok()?;
    Some(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Replace every `YYYY-MM-DDTHH:MM:SS[.fff]Z` with `<utc>`.
fn mask_utc(text: &str) -> String {
    const SHAPE: &[u8] = b"dddd-dd-ddTdd:dd:dd";
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let fits = bytes.len() >= i + SHAPE.len()
            && SHAPE
                .iter()
                .zip(&bytes[i..])
                .all(|(shape, byte)| match shape {
                    b'd' => byte.is_ascii_digit(),
                    other => other == byte,
                });
        if fits {
            let mut end = i + SHAPE.len();
            if bytes.get(end) == Some(&b'.') {
                end += 1;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
            }
            if bytes.get(end) == Some(&b'Z') {
                out.push_str("<utc>");
                i = end + 1;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_lists_its_own_reply_when_the_cookie_is_reused() {
        let text = include_str!("../../fixtures/sessions/2026-10-23-1149-money-or-mob.sbg");
        let mut recording = record::parse(text).unwrap();
        // Give a Config get and the next request on its connection one cookie, as after a wrap.
        let index_of = |recording: &record::Recording, part: &str| {
            recording
                .events
                .iter()
                .position(|event| request_key(&event.frame).is_some_and(|key| key.ends_with(part)))
                .unwrap()
        };
        let config = index_of(&recording, ".Store.Config");
        let other = index_of(&recording, "UserStore.UserStorage");
        let reused = recording.events[config].frame[6..8].to_vec();
        for index in [other, other + 1] {
            recording.events[index].frame[6..8].copy_from_slice(&reused);
        }
        recording.events.truncate(other + 2);
        let filter = ListFilter {
            key: Some("store.config".into()),
            ..ListFilter::default()
        };
        let listed: Vec<String> = list(&recording, &filter).collect();
        assert_eq!(listed.len(), 2, "{listed:#?}");
        assert!(listed[0].contains(" get v3.Setup.MGS.1vs100.Store.Config"));
        assert!(listed[1].contains("name=\"Config\""));
    }

    #[test]
    fn a_recording_lists_its_requests_and_replies_as_text() {
        let text = include_str!("../../fixtures/sessions/2026-10-23-1149-money-or-mob.sbg");
        let recording = record::parse(text).unwrap();
        let shown = |filter: ListFilter, count: usize| -> String {
            list(&recording, &filter)
                .take(count)
                .map(|line| line.chars().take(160).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let first_conn = ListFilter {
            conn: Some(ConnId(1)),
            ..ListFilter::default()
        };
        insta::assert_snapshot!("conn_1", shown(first_conn, 16));
        let xuid = recording
            .events
            .iter()
            .find_map(|event| event.xuid)
            .unwrap();
        let user_storage = ListFilter {
            xuid: Some(xuid),
            key: Some("USERSTORAGE".into()),
            ..ListFilter::default()
        };
        insta::assert_snapshot!("xuid_and_key", shown(user_storage, 8));
    }

    #[test]
    fn utc_timestamps_are_masked_and_nothing_else() {
        assert_eq!(
            mask_utc("<StartTime>2026-10-11T01:33:52.000Z</StartTime><n>2026</n>"),
            "<StartTime><utc></StartTime><n>2026</n>"
        );
        assert_eq!(mask_utc("a 2026-10-11T01:33:52Z b"), "a <utc> b");
        assert_eq!(
            mask_utc("2026-10-11T01:33:52 no zone"),
            "2026-10-11T01:33:52 no zone"
        );
        assert_eq!(mask_utc("ünï 2026-10-11T01:33:52Z"), "ünï <utc>");
    }

    #[test]
    fn compressed_and_plain_replies_normalize_to_their_text() {
        let doc = b"<x><t>2026-10-11T01:33:52.000Z</t></x>";
        let compressed = super::super::message::zlib(doc).unwrap();
        let v1317_reply = v1317::generic_reply(7, 200, true, &compressed);
        assert_eq!(
            normalize(Protocol::V1317, &v1317_reply),
            "kind=0x82 status=200 <x><t><utc></t></x>"
        );
        let v1149_reply = v1149::reply(7, 200, &compressed[2..]);
        assert_eq!(
            normalize(Protocol::V1149, &v1149_reply),
            "kind=0x82 status=200 <x><t><utc></t></x>"
        );
        let raw_user_doc = v1149::reply(8, 200, b"<UserStorage/>");
        assert_eq!(
            normalize(Protocol::V1149, &raw_user_doc),
            "kind=0x82 status=200 <UserStorage/>"
        );
    }

    #[test]
    fn a_frame_too_short_for_a_cookie_is_a_mismatch() {
        // A reply header carries its cookie at bytes 6..8; below that it cannot be paired.
        let short = vec![0u8; 7];
        let expected = vec![(ConnId(1), v1317::generic_reply(0, 200, false, b""))];
        let actual = vec![(ConnId(1), short.clone())];
        let mismatches = compare(Protocol::V1317, expected.into_iter(), actual.into_iter());
        assert_eq!(mismatches.len(), 2, "{mismatches:#?}");
        assert_eq!(
            (mismatches[0].conn, mismatches[0].cookie),
            (ConnId(1), None)
        );
        assert_eq!(
            mismatches[0].actual,
            Some(normalize(Protocol::V1317, &short))
        );
        assert_eq!(mismatches[1].cookie, Some(0));
    }

    #[test]
    fn compare_pairs_by_connection_and_cookie_and_reports_both_sides() {
        let reply = |cookie, status| v1317::generic_reply(cookie, status, false, b"");
        let expected = [
            (ConnId(1), reply(1, 200)),
            (ConnId(1), reply(2, 200)),
            (ConnId(2), reply(1, 404)),
        ];
        let actual = [
            (ConnId(2), reply(1, 404)),
            (ConnId(1), reply(2, 404)),
            (ConnId(1), reply(3, 200)),
        ];
        let mismatches = compare(Protocol::V1317, expected.into_iter(), actual.into_iter());
        let summary: Vec<_> = mismatches
            .iter()
            .map(|m| (m.conn, m.cookie, m.expected.is_some(), m.actual.is_some()))
            .collect();
        assert_eq!(
            summary,
            [
                (ConnId(1), Some(1), true, false),
                (ConnId(1), Some(2), true, true),
                (ConnId(1), Some(3), false, true)
            ]
        );
    }
}
