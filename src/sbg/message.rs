//! Build-neutral SBG messages. Each supported title build's wire format (`crate::wire::v1317`,
//! `crate::wire::v1149`) is decoded into one `Message`; `dispatch` only ever sees `Action`s and answers
//! with a `Reply`, which is encoded back into the requesting build's format.

use std::{
    io::{self, Read, Write},
    time::{Duration, SystemTime},
};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};

use super::{
    key::{Key, ParsedKey},
    v1149_keys,
};
use crate::{
    ids::Xuid,
    wire::{self, Frame, Protocol, v1149, v1317},
};

const MAX_INFLATED_VALUE: usize = 0x100000;

/// One decoded client frame.
#[derive(Debug, PartialEq)]
pub enum Message {
    Register,
    /// v2.0.1149.0: the player on `controller` leaves (`wire::v1149::KIND_UNREGISTER`).
    Unregister {
        controller: u8,
        xuid: Xuid,
    },
    Heartbeat,
    Request {
        key: String,
        action: Action,
        /// When the server carries the request out; `None` is now.
        execute_at: Option<SystemTime>,
    },
}

/// Windows FILETIME as `SystemTime`; 0 (now) is `None`.
pub(crate) fn filetime_to_system_time(filetime: u64) -> Option<SystemTime> {
    const UNIX_EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;
    let since_unix = filetime.checked_sub(UNIX_EPOCH_AS_FILETIME)?;
    (filetime != 0).then(|| SystemTime::UNIX_EPOCH + Duration::from_nanos(since_unix * 100))
}

/// What a request asks of the store, independent of how the build encoded it.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Read a store key (may be held until a write arrives, see `is_deferrable`).
    Get,
    /// Write a store key; `value` is already inflated.
    Set(Vec<u8>),
    /// Read a sum counter (session `...Sum...` or persistent `...Stats...Sum...`).
    SumGet,
    /// Add to a sum counter.
    SumAdd(f64),
    /// Read a TopN board.
    TopNGet,
    /// Post a score to a TopN board keeping at most `capacity` entries.
    TopNPost {
        capacity: usize,
        score: f64,
        metadata: Vec<u8>,
    },
    /// Read a per-user document (UserStorage / UserStats).
    UserGet { xuid: Xuid },
    /// Write a per-user document.
    UserSet { xuid: Xuid, value: Vec<u8> },
}

impl Action {
    /// Plain reads may wait for a write, answered from their own thread.
    pub fn is_deferrable(&self) -> bool {
        matches!(self, Self::Get)
    }

    /// The request's name in logs (no payload).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Set(_) => "set",
            Self::SumGet => "sum get",
            Self::SumAdd(_) => "sum add",
            Self::TopNGet => "topn get",
            Self::TopNPost { .. } => "topn post",
            Self::UserGet { .. } => "user get",
            Self::UserSet { .. } => "user set",
        }
    }
}

/// Why a frame produced no `Message`.
#[derive(Debug, PartialEq)]
pub enum DecodeError {
    /// A request frame whose body could not be understood: answered 404.
    Malformed,
    /// A frame type this backend does not serve: logged, not answered.
    Unsupported(u8),
}

pub fn decode(frame: &Frame) -> Result<Message, DecodeError> {
    match (frame.protocol(), frame.kind) {
        (_, wire::KIND_REGISTER) => Ok(Message::Register),
        (_, wire::KIND_HEARTBEAT) => Ok(Message::Heartbeat),
        (Protocol::V1149, v1149::KIND_UNREGISTER) => v1149::parse_unregister(&frame.body)
            .map(|(controller, xuid)| Message::Unregister {
                controller,
                xuid: Xuid(xuid),
            })
            .ok_or(DecodeError::Malformed),
        (Protocol::V1317, v1317::KIND_NAMED_REQUEST) => decode_v1317_named(frame),
        (Protocol::V1149, kind) => decode_v1149(kind, frame),
        (_, kind) => Err(DecodeError::Unsupported(kind)),
    }
}

/// Decode a v2.0.1149.0 frame; the kind names the action.
fn decode_v1149(kind: u8, frame: &Frame) -> Result<Message, DecodeError> {
    let cookie = frame.cookie;
    let request = match v1149::parse_request(kind, &frame.body) {
        Ok(Some(request)) => request,
        Ok(None) => return Err(DecodeError::Unsupported(kind)),
        Err(error) => {
            tracing::debug!(?error, cookie, kind, "SBG unparsable v1149 request");
            return Err(DecodeError::Malformed);
        }
    };
    let (key, action) = match request.op {
        v1149::Op::Get { key } => (key, Action::Get),
        v1149::Op::Set { key, value } => (key, Action::Set(inflate_v1149_set(cookie, value)?)),
        v1149::Op::SumAdd { key, delta } => (key, Action::SumAdd(delta)),
        v1149::Op::TopPost {
            key,
            capacity,
            score,
            metadata,
        } => (
            key,
            Action::TopNPost {
                capacity: capacity.into(),
                score,
                metadata: metadata.to_vec(),
            },
        ),
        v1149::Op::UserGet { xuid, key } => (key, Action::UserGet { xuid: Xuid(xuid) }),
        v1149::Op::UserSet { xuid, key, value } => (
            key,
            Action::UserSet {
                xuid: Xuid(xuid),
                value: value.to_vec(),
            },
        ),
    };
    let Ok(wire_key) = std::str::from_utf8(key) else {
        tracing::debug!(cookie, "SBG request key not UTF-8");
        return Err(DecodeError::Malformed);
    };
    let key = v1149_keys::to_store_key(wire_key);
    tracing::trace!(
        cookie,
        kind,
        wire_key,
        key,
        controller = request.controller,
        execute_time_utc = request.execute_time_utc,
        "SBG v1149 request"
    );
    let action = action_for_store_key(&key, action);
    Ok(Message::Request {
        key,
        action,
        execute_at: filetime_to_system_time(request.execute_time_utc),
    })
}

/// Inflate a v2.0.1149.0 `set` value (raw deflate); empty stays empty.
fn inflate_v1149_set(cookie: u16, value: &[u8]) -> Result<Vec<u8>, DecodeError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut stream = Vec::with_capacity(value.len() + 2);
    stream.extend_from_slice(&[0x78, 0x01]);
    stream.extend_from_slice(value);
    inflate(true, &stream).map_err(|error| {
        tracing::debug!(%error, cookie, "SBG v1149 set value does not inflate");
        DecodeError::Malformed
    })
}

/// Resolve the store action by key class; 1149 UserError posts are not user documents.
fn action_for_store_key(key: &str, action: Action) -> Action {
    let key = ParsedKey::parse(key);
    let user_key = matches!(key.kind, Key::User(_));
    match action {
        Action::Get if key.is_sum() => Action::SumGet,
        Action::Get if key.is_top_n() => Action::TopNGet,
        Action::UserGet { .. } if !user_key => Action::Get,
        Action::UserSet { value, .. } if !user_key => Action::Set(value),
        action => action,
    }
}

/// v2.0.1317.0: the action is implied by the key namespace, the tag and the value.
fn decode_v1317_named(frame: &Frame) -> Result<Message, DecodeError> {
    let cookie = frame.cookie;
    let request = v1317::parse_named_body(&frame.body).map_err(|error| {
        tracing::debug!(?error, cookie, "SBG unparsable named request");
        DecodeError::Malformed
    })?;
    let (Ok(key), Ok(tag)) = (
        std::str::from_utf8(request.key),
        std::str::from_utf8(request.tag),
    ) else {
        tracing::debug!(cookie, "SBG request key/tag not UTF-8");
        return Err(DecodeError::Malformed);
    };
    tracing::trace!(
        cookie,
        key,
        tag,
        compressed = request.compressed,
        value_len = request.value.len(),
        "SBG request"
    );
    let action = action_from_v1317(key, tag, request.compressed, request.value)
        .ok_or(DecodeError::Malformed)?;
    // Only store GETs act on the execute time: a GameState GET is answered at it. Sums keep
    // their own settle.
    let execute_at = match action {
        Action::Get => filetime_to_system_time(request.execute_time_utc),
        _ => None,
    };
    Ok(Message::Request {
        key: key.to_owned(),
        action,
        execute_at,
    })
}

fn action_from_v1317(key: &str, tag: &str, compressed: bool, value: &[u8]) -> Option<Action> {
    let key = ParsedKey::parse(key);
    if matches!(key.kind, Key::User(_)) {
        let xuid = Xuid(tag.strip_prefix("xuid=")?.parse::<u64>().ok()?);
        if compressed {
            return None;
        }
        return Some(if value.is_empty() {
            Action::UserGet { xuid }
        } else {
            Action::UserSet {
                xuid,
                value: value.to_vec(),
            }
        });
    }
    if key.is_sum() {
        if compressed || !value.is_empty() {
            return None;
        }
        if tag.is_empty() {
            return Some(Action::SumGet);
        }
        return Some(Action::SumAdd(tag.strip_prefix("value=")?.parse().ok()?));
    }
    if key.is_top_n() {
        if compressed {
            return None;
        }
        if tag.is_empty() {
            return value.is_empty().then_some(Action::TopNGet);
        }
        let (capacity, score) = parse_top_n_tag(tag)?;
        return Some(Action::TopNPost {
            capacity,
            score,
            metadata: value.to_vec(),
        });
    }
    // Tagged Store GETs (rounds 2+ TheMob, conditional-GET markers): the tag is ignored. It was
    // once a reject guard; it blocked store_get_with_wait from catching the SET that follows.
    let value = inflate(compressed, value).ok()?;
    Some(if value.is_empty() {
        Action::Get
    } else {
        Action::Set(value)
    })
}

fn parse_top_n_tag(tag: &str) -> Option<(usize, f64)> {
    let (capacity, score) = tag.strip_prefix("capacity=")?.split_once("&value=")?;
    if capacity.is_empty() || score.is_empty() || score.contains('&') {
        return None;
    }
    Some((capacity.parse().ok()?, score.parse().ok()?))
}

fn inflate(compressed: bool, bytes: &[u8]) -> io::Result<Vec<u8>> {
    if !compressed {
        return Ok(bytes.to_vec());
    }
    let mut value = Vec::new();
    ZlibDecoder::new(bytes)
        .take((MAX_INFLATED_VALUE + 1) as u64)
        .read_to_end(&mut value)?;
    if value.len() > MAX_INFLATED_VALUE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "inflated SBG value exceeds 1 MiB",
        ));
    }
    Ok(value)
}

/// Backend answer, build-independent.
#[derive(Debug, PartialEq)]
pub enum Reply {
    Register,
    Generic {
        status: u16,
        value: Vec<u8>,
        /// Whether 1317 compresses the value; user documents and 1149 stay raw.
        compressible: bool,
    },
}

impl Reply {
    /// 200 with a compressible document.
    pub fn document(value: Vec<u8>) -> Self {
        Self::Generic {
            status: 200,
            value,
            compressible: true,
        }
    }

    /// 200 with a value that is never compressed.
    pub fn raw(value: Vec<u8>) -> Self {
        Self::Generic {
            status: 200,
            value,
            compressible: false,
        }
    }

    pub fn ok_empty() -> Self {
        Self::raw(Vec::new())
    }

    pub fn not_found() -> Self {
        Self::Generic {
            status: 404,
            value: Vec::new(),
            compressible: false,
        }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Generic { status: 404, .. })
    }

    pub fn encode(&self, protocol: Protocol, cookie: u16) -> Vec<u8> {
        match self {
            Self::Register => wire::register_reply(protocol, cookie),
            // v2.0.1149.0 inflates every non-empty `get` value (`sub_82236D40` -> `sub_82134FA0`, which
            // supplies the zlib header `78 01` itself), so store documents go out as a zlib stream
            // without its 2-byte header; user documents (userget) are read raw.
            Self::Generic {
                status,
                value,
                compressible: true,
            } if protocol == Protocol::V1149 && !value.is_empty() => {
                match zlib(&v1149::adapt_document(value)) {
                    Ok(compressed) => v1149::reply(cookie, *status, &compressed[2..]),
                    Err(_) => v1149::reply(cookie, 500, &[]),
                }
            }
            Self::Generic { status, value, .. } if protocol == Protocol::V1149 => {
                v1149::reply(cookie, *status, &v1149::adapt_document(value))
            }
            Self::Generic {
                status,
                value,
                compressible: true,
            } => match zlib(value) {
                Ok(compressed) => v1317::generic_reply(cookie, *status, true, &compressed),
                Err(_) => v1317::generic_reply(cookie, 500, false, &[]),
            },
            Self::Generic { status, value, .. } => {
                v1317::generic_reply(cookie, *status, false, value)
            }
        }
    }
}

pub(super) fn zlib(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes)?;
    encoder.finish()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        ids::QuestionId,
        wire::{FIXTURE_1149, FIXTURE_1317, fixture_frame},
    };

    /// What `sub_82134FA0` does with a v2.0.1149.0 get value: prepend `78 01`, inflate.
    pub(crate) fn inflate_v1149_value(value: &[u8]) -> Vec<u8> {
        let mut stream = vec![0x78, 0x01];
        stream.extend_from_slice(value);
        let mut out = Vec::new();
        ZlibDecoder::new(&stream[..]).read_to_end(&mut out).unwrap();
        out
    }

    fn decode_from(fixture: &str, label: &str) -> Message {
        let raw = fixture_frame(fixture, label);
        let (frame, used) = wire::decode(&raw).unwrap().unwrap();
        assert_eq!(used, raw.len(), "{label}: one whole frame");
        decode(&frame).unwrap()
    }

    /// A payload as its length and its two ends: the middle of a 5 kB document adds nothing.
    fn payload(value: &[u8]) -> String {
        const HEAD: usize = 48;
        const TAIL: usize = 24;
        if value.len() <= HEAD + TAIL {
            return format!("{} bytes {:?}", value.len(), String::from_utf8_lossy(value));
        }
        format!(
            "{} bytes {:?} .. {:?}",
            value.len(),
            String::from_utf8_lossy(&value[..HEAD]),
            String::from_utf8_lossy(&value[value.len() - TAIL..]),
        )
    }

    /// A decoded message as one line; the execute time as unix milliseconds.
    fn message_shown(message: Message) -> String {
        match message {
            Message::Request {
                key,
                action,
                execute_at,
            } => {
                let action = match &action {
                    Action::Set(value) => format!("Set({})", payload(value)),
                    Action::UserSet { xuid, value } => {
                        format!("UserSet({xuid:?}, {})", payload(value))
                    }
                    Action::TopNPost {
                        capacity,
                        score,
                        metadata,
                    } => format!(
                        "TopNPost(capacity {capacity}, score {score}, {})",
                        payload(metadata)
                    ),
                    other => format!("{other:?}"),
                };
                let at = execute_at.map(|at| {
                    at.duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                });
                format!("Request {{ key: {key}, action: {action}, execute_at: {at:?} }}")
            }
            other => format!("{other:?}"),
        }
    }

    /// The named frames of `fixture`, decoded, one per line.
    fn shown(fixture: &str, labels: &[&str]) -> String {
        labels
            .iter()
            .map(|label| format!("{label}: {}", message_shown(decode_from(fixture, label))))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn captured_1317_requests_decode() {
        let labels = [
            "register",
            "heartbeat",
            "store_get_game_state",
            "store_get_config",
            "store_get_question_bank_reset_id",
            "store_get_the_mob",
            "store_set_user_error",
            "store_set_the_mob_compressed",
            "user_get_user_storage",
            "sum_add_player_count",
            "sum_get_player_count",
            "top_n_post_the_one",
            "top_n_get_the_one",
        ];
        insta::assert_snapshot!(shown(FIXTURE_1317, &labels));
    }

    #[test]
    fn captured_1149_requests_decode() {
        const LOBBY: &str = include_str!("../../fixtures/sbg-1149-capture-20261019-lobby.txt");
        let captured = shown(
            FIXTURE_1149,
            &[
                "register",
                "heartbeat",
                "get_config",
                "get_pangolin_config",
                "userset_user_error",
            ],
        );
        let lobby = shown(
            LOBBY,
            &[
                "sum_rs_total_player_count",
                "top_mob_selection",
                "top_the_one",
                "set_the_mob",
            ],
        );
        insta::assert_snapshot!(format!("{captured}\n{lobby}"));
    }

    #[test]
    fn a_filetime_of_zero_is_now() {
        assert_eq!(filetime_to_system_time(0), None);
    }

    #[test]
    fn question_lists_of_a_large_bank_fit_the_v1149_inflate_buffer() {
        use crate::questions::{
            BANK_FILE_VERSION, BankFile, Choice, Difficulty, Question, QuestionBank, Source,
        };
        let questions = (0..3000)
            .map(|n| Question {
                id: QuestionId(u32::MAX - n),
                locale: crate::questions::LOCALE.into(),
                difficulty: Difficulty::Medium,
                text: format!("Q{n}"),
                choices: ["A".into(), "B".into(), "C".into()],
                correct: Choice::A,
                categories: Vec::new(),
                source: "otdb".into(),
                origin: Default::default(),
            })
            .collect();
        let bank = QuestionBank::from_file(BankFile {
            version: BANK_FILE_VERSION,
            sources: vec![Source {
                id: "otdb".into(),
                name: "Open Trivia Database".into(),
                licence: String::new(),
                url: String::new(),
            }],
            categories: Vec::new(),
            questions,
        })
        .unwrap();
        let show = crate::questions::ShowSeed::of("f605b5d4-7f6b-42b0-95ab-16aee9b51db4");
        for (list, doc) in [
            (
                "ResetID",
                crate::xsor::question_reset_ids(&bank, 3, None, show),
            ),
            (
                "Available",
                crate::xsor::question_bank_available(&bank, 3, None, show),
            ),
        ] {
            let reply = Reply::document(doc.unwrap()).encode(Protocol::V1149, 7);
            let value = &reply[wire::HEADER_LEN + v1149::REPLY_PREFIX_LEN..];
            let inflated = inflate_v1149_value(value).len();
            assert!(
                inflated <= v1149::INFLATE_CAPACITY,
                "{list}: {inflated} bytes inflated"
            );
        }
    }

    #[test]
    fn replies_take_the_requesting_build_layout() {
        assert_eq!(
            Reply::Register.encode(Protocol::V1149, 0x65),
            fixture_frame(FIXTURE_1149, "reply_register")
        );
        let doc = Reply::document(b"<x/>".to_vec());
        let v1149 = doc.encode(Protocol::V1149, 7);
        assert_eq!(inflate_v1149_value(&v1149[26..]), b"<x/>");
        let user = Reply::raw(b"<x/>".to_vec()).encode(Protocol::V1149, 7);
        assert_eq!((user.len(), &user[26..]), (30, &b"<x/>"[..]));
        let v1317 = doc.encode(Protocol::V1317, 7);
        assert_eq!(v1317[1], v1317::KIND_GENERIC_REPLY);
        assert!(v1317::GenericReply::parse(&v1317).unwrap().compressed);
    }

    #[test]
    fn only_a_404_reply_counts_as_not_found() {
        assert!(Reply::not_found().is_not_found());
        for found in [
            Reply::ok_empty(),
            Reply::document(b"<x/>".to_vec()),
            Reply::Register,
        ] {
            assert!(!found.is_not_found(), "{found:?}");
        }
    }
}
