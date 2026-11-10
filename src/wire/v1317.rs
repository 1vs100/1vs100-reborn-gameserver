//! v2.0.1317.0 request/reply bodies: every store action travels as one named request (kind
//! 0x0A: key, tag, optional zlib value); the action is implied by key namespace, tag and value.

use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned,
    byteorder::network_endian::{U16, U32, U64},
};

use super::{FrameError, HEADER_LEN, MAX_FRAME, Protocol, encode, parse_header};

/// v2.0.1317.0 named request.
pub const KIND_NAMED_REQUEST: u8 = 0x0a;
/// Reply to a named request.
pub const KIND_GENERIC_REPLY: u8 = 0x82;

/// Named request prefix: execute time (FILETIME, 0 = now) precedes the key.
#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct NamedRequestPrefix {
    reserved: u8,
    /// A FILETIME of unknown meaning; it drifts against the arrival time over a session.
    unknown_time: U64,
    /// 0, 1 or 2 in the captures; meaning unknown.
    unknown: u8,
    execute_time_utc: U64,
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct NamedValuePrefix {
    compression: u8,
    reserved: [u8; 2],
    value_length: U16,
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct GenericReplyPrefix {
    status: U16,
    reserved: [u8; 24],
    compression: u8,
    value_length: U32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NamedRequest<'a> {
    pub key: &'a [u8],
    pub tag: &'a [u8],
    pub compressed: bool,
    pub value: &'a [u8],
    /// Execute time, FILETIME; 0 is now.
    pub execute_time_utc: u64,
}

impl NamedRequest<'_> {
    /// Frame body in the layout the title sends.
    pub fn to_body(&self) -> Result<Vec<u8>, FrameError> {
        let too_long =
            |bytes: &[u8]| u16::try_from(bytes.len()).map_err(|_| FrameError::InvalidNamedBody);
        let (key_len, tag_len, value_len) = (
            too_long(self.key)?,
            too_long(self.tag)?,
            too_long(self.value)?,
        );
        let prefix = NamedValuePrefix {
            compression: u8::from(self.compressed),
            reserved: [0; 2],
            value_length: U16::new(value_len),
        };
        let mut body = NamedRequestPrefix {
            reserved: 0,
            unknown_time: U64::ZERO,
            unknown: 0,
            execute_time_utc: U64::new(self.execute_time_utc),
        }
        .as_bytes()
        .to_vec();
        body.extend_from_slice(&key_len.to_be_bytes());
        body.extend_from_slice(self.key);
        body.extend_from_slice(&tag_len.to_be_bytes());
        body.extend_from_slice(self.tag);
        body.extend_from_slice(prefix.as_bytes());
        body.extend_from_slice(self.value);
        Ok(body)
    }

    pub fn to_frame(&self, cookie: u16) -> Result<Vec<u8>, FrameError> {
        Ok(encode(
            Protocol::V1317,
            KIND_NAMED_REQUEST,
            cookie,
            &self.to_body()?,
        ))
    }
}

/// Bare layout key offset: nine zero bytes, no execute time; accepted, never sent.
const BARE_KEY_LENGTH_OFFSET: usize = 9;

pub fn parse_named_body(body: &[u8]) -> Result<NamedRequest<'_>, FrameError> {
    let prefixed = NamedRequestPrefix::ref_from_prefix(body)
        .map_err(|_| FrameError::InvalidNamedBody)
        .and_then(|(prefix, _)| {
            let execute_time_utc = prefix.execute_time_utc.get();
            parse_named_body_at(
                body,
                std::mem::size_of::<NamedRequestPrefix>(),
                execute_time_utc,
            )
        });
    prefixed
        .or_else(|error| parse_named_body_at(body, BARE_KEY_LENGTH_OFFSET, 0).map_err(|_| error))
}

fn parse_named_body_at(
    body: &[u8],
    key_length_offset: usize,
    execute_time_utc: u64,
) -> Result<NamedRequest<'_>, FrameError> {
    let (key_length, _) = U16::ref_from_prefix(
        body.get(key_length_offset..)
            .ok_or(FrameError::InvalidNamedBody)?,
    )
    .map_err(|_| FrameError::InvalidNamedBody)?;
    let key_start = key_length_offset + std::mem::size_of::<U16>();
    let key_end = key_start + key_length.get() as usize;
    let key = body
        .get(key_start..key_end)
        .ok_or(FrameError::InvalidNamedBody)?;
    let mut tail = body.get(key_end..).ok_or(FrameError::InvalidNamedBody)?;
    let tag_len = u16::from_be_bytes(
        tail.get(..2)
            .ok_or(FrameError::InvalidNamedBody)?
            .try_into()
            .unwrap(),
    ) as usize;
    tail = &tail[2..];
    let tag = tail.get(..tag_len).ok_or(FrameError::InvalidNamedBody)?;
    tail = &tail[tag_len..];
    let (value_prefix, _) =
        NamedValuePrefix::ref_from_prefix(tail).map_err(|_| FrameError::InvalidNamedBody)?;
    if value_prefix.reserved != [0; 2] {
        return Err(FrameError::InvalidNamedBody);
    }
    let compressed = value_prefix.compression != 0;
    let value_len = value_prefix.value_length.get() as usize;
    tail = &tail[std::mem::size_of::<NamedValuePrefix>()..];
    if tail.len() != value_len {
        return Err(FrameError::InvalidNamedBody);
    }
    Ok(NamedRequest {
        key,
        tag,
        compressed,
        value: tail,
        execute_time_utc,
    })
}

/// The reply to a named request: status, optional zlib flag and a value.
#[derive(Debug, PartialEq, Eq)]
pub struct GenericReply<'a> {
    pub cookie: u16,
    pub status: u16,
    pub compressed: bool,
    pub value: &'a [u8],
}

impl<'a> GenericReply<'a> {
    pub fn parse(frame: &'a [u8]) -> Result<Self, FrameError> {
        let (header, len) =
            parse_header(frame.get(..HEADER_LEN).ok_or(FrameError::InvalidLength)?)?;
        if frame.len() != len || header.kind != KIND_GENERIC_REPLY {
            return Err(FrameError::InvalidLength);
        }
        let (prefix, value) = GenericReplyPrefix::ref_from_prefix(&frame[HEADER_LEN..])
            .map_err(|_| FrameError::InvalidLength)?;
        if prefix.value_length.get() as usize != value.len() {
            return Err(FrameError::InvalidLength);
        }
        Ok(Self {
            cookie: header.cookie.get(),
            status: prefix.status.get(),
            compressed: prefix.compression != 0,
            value,
        })
    }

    /// Serializes the reply. A value that would overflow `MAX_FRAME` is replaced by an empty 500.
    pub fn encode(&self) -> Vec<u8> {
        let fits =
            HEADER_LEN + std::mem::size_of::<GenericReplyPrefix>() + self.value.len() <= MAX_FRAME;
        let (status, compressed, value) = if fits {
            (self.status, self.compressed, self.value)
        } else {
            (500, false, &[][..])
        };
        let prefix = GenericReplyPrefix {
            status: U16::new(status),
            reserved: [0; 24],
            compression: u8::from(compressed),
            value_length: U32::new(value.len() as u32),
        };
        let mut body = prefix.as_bytes().to_vec();
        body.extend_from_slice(value);
        encode(Protocol::V1317, KIND_GENERIC_REPLY, self.cookie, &body)
    }
}

pub fn generic_reply(cookie: u16, status: u16, compressed: bool, value: &[u8]) -> Vec<u8> {
    GenericReply {
        cookie,
        status,
        compressed,
        value,
    }
    .encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{FIXTURE_1317, decode, fixture_frame, register_reply};

    /// A key, tag or value as its text; a compressed value is binary and shows as its length.
    fn shown(bytes: &[u8]) -> String {
        match std::str::from_utf8(bytes) {
            Ok("") => "-".to_owned(),
            Ok(text) => text.to_owned(),
            Err(_) => format!("{} bytes", bytes.len()),
        }
    }

    /// A parsed body as one line, its key, tag and value as text rather than byte lists.
    fn named_request_shown(parsed: Result<NamedRequest<'_>, FrameError>) -> String {
        let Ok(request) = parsed else {
            return format!("{:?}", parsed.unwrap_err());
        };
        format!(
            "compressed {}, execute time {}: key {}, tag {}, value {}",
            request.compressed,
            request.execute_time_utc,
            shown(request.key),
            shown(request.tag),
            shown(request.value),
        )
    }

    #[test]
    fn named_request_bodies_parse_to_their_fields() {
        const BARE: &[u8] = &[
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, b'k', 0, 1, b't', 0, 0, 0, 0, 0,
        ];
        const CAPTURED: &[u8] = b"\x00\x01\xdd\x4f\x92\x03\xa1\x90\xc0\
            \x01\x01\xdd\x4f\x91\xe1\xce\x9f\x90\
            \x00\x54v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.591.TheOne\
            \x00\x19capacity=1&value=0.720000\
            \x00\x00\x00\x00\x29<Metadata>16140909857506937655</Metadata>";
        let mut cases = [
            ("the bare layout", BARE.to_vec()),
            ("a captured prefix", CAPTURED.to_vec()),
            (
                "a captured prefix cut short",
                CAPTURED[..CAPTURED.len() - 1].to_vec(),
            ),
        ]
        .to_vec();
        for label in [
            "store_get_game_state",
            "store_set_the_mob_compressed",
            "user_get_user_storage",
            "top_n_post_the_one",
        ] {
            let raw = fixture_frame(FIXTURE_1317, label);
            let (frame, _) = decode(&raw).unwrap().unwrap();
            cases.push((label, frame.body));
        }
        let shown_bodies: Vec<String> = cases
            .into_iter()
            .map(|(label, body)| {
                format!("{label}: {}", named_request_shown(parse_named_body(&body)))
            })
            .collect();
        insta::assert_snapshot!(shown_bodies.join("\n"));
    }

    #[test]
    fn documented_ack_and_register_reply_sizes() {
        let ack = generic_reply(1, 200, false, &[]);
        assert_eq!(ack.len(), 39);
        assert_eq!(&ack[..10], &[3, 0x82, 0, 0, 0, 39, 0, 1, 0, 200]);
        assert_eq!(ack[26..], [0; 13]);
        assert_eq!(generic_reply(1, 404, false, &[])[8..10], [1, 148]);
        assert_eq!(register_reply(Protocol::V1317, 1).len(), 26);
    }

    #[test]
    fn named_request_round_trips_through_the_canonical_layout() {
        for (compressed, value) in [(false, &b""[..]), (true, &b"\x78\x9c payload"[..])] {
            let request = NamedRequest {
                key: b"v3.Session.MGS.1vs100.Store.P.GameState",
                tag: b"xuid=123",
                compressed,
                value,
                execute_time_utc: 0x01dd_61a6_d0e7_0790,
            };
            let body = request.to_body().unwrap();
            assert_eq!(parse_named_body(&body).unwrap(), request);

            let frame = request.to_frame(9).unwrap();
            let (decoded, used) = decode(&frame).unwrap().unwrap();
            assert_eq!(
                (decoded.kind, decoded.cookie, used),
                (KIND_NAMED_REQUEST, 9, frame.len())
            );
            assert_eq!(decoded.body, body);
        }
        let oversized = vec![0; u16::MAX as usize + 1];
        let request = NamedRequest {
            key: b"k",
            tag: b"",
            compressed: false,
            value: &oversized,
            execute_time_utc: 0,
        };
        assert_eq!(request.to_body(), Err(FrameError::InvalidNamedBody));
    }

    #[test]
    fn generic_reply_round_trips_and_rejects_malformed_frames() {
        let reply = GenericReply {
            cookie: 4,
            status: 200,
            compressed: true,
            value: b"abc",
        };
        let bytes = reply.encode();
        assert_eq!(GenericReply::parse(&bytes).unwrap(), reply);

        assert!(GenericReply::parse(&bytes[..bytes.len() - 1]).is_err());
        assert!(GenericReply::parse(&register_reply(Protocol::V1317, 4)).is_err());
        let mut lying = bytes.clone();
        lying[38] = 9; // value_length 9 does not match the payload
        assert!(GenericReply::parse(&lying).is_err());
    }

    #[test]
    fn generic_reply_over_max_frame_becomes_bounded_error_without_changing_register() {
        let reply = generic_reply(3, 200, false, &vec![0; MAX_FRAME - 38]);
        assert!(reply.len() <= MAX_FRAME);
        assert_eq!(&reply[..8], &[3, 0x82, 0, 0, 0, 39, 0, 3]);
        assert_eq!(&reply[8..10], &500u16.to_be_bytes());

        assert_eq!(register_reply(Protocol::V1317, 1).len(), 26);
        assert_eq!(
            &register_reply(Protocol::V1317, 1)[..10],
            &[3, 0x80, 0, 0, 0, 26, 0, 1, 0, 200]
        );
    }
}
