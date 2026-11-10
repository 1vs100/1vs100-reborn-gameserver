//! v2.0.1149.0 (retail) request/reply bodies: one request type per Lua `BinaryMessage` action
//! (`sub_82225718`). Layouts: `documentation/protocol/SBG_WIRE_1149.md`.

use super::{FrameError, HEADER_LEN, MAX_FRAME, Protocol, encode};

/// v2.0.1149.0 unregister (`SRM: UnregisterUser`, a player leaving): 25 bytes, controller u8,
/// 8 opaque bytes, xuid u64 BE; the register frame (0x00) without its trailing flag byte.
pub const KIND_UNREGISTER: u8 = 0x01;
/// v2.0.1149.0 `get` (built by `sub_82238F68`).
pub const KIND_GET: u8 = 0x03;
/// v2.0.1149.0 `set` (built by `sub_82238FF8`).
pub const KIND_SET: u8 = 0x04;
/// v2.0.1149.0 `sum` add (`sub_82239120`).
pub const KIND_SUM: u8 = 0x05;
/// v2.0.1149.0 TopN post (`sub_82239210`).
pub const KIND_TOP: u8 = 0x06;
/// v2.0.1149.0 `rankget` (`sub_822394C8`). Not decoded yet: no captured frame.
pub const KIND_RANK_GET: u8 = 0x07;
/// v2.0.1149.0 `rankset` (`sub_82239368`). Not decoded yet: no captured frame.
pub const KIND_RANK_SET: u8 = 0x08;
/// v2.0.1149.0 `userget` (built by `sub_822395A0`).
pub const KIND_USER_GET: u8 = 0x09;
/// v2.0.1149.0 `userset` (built by `sub_82239658`).
pub const KIND_USER_SET: u8 = 0x0a;
/// v2.0.1149.0 reply to a request (`sub_8223D940`; 0x81 is read the same way).
pub const KIND_REPLY: u8 = 0x82;
/// v2.0.1149.0 unregister reply: 26 bytes, status at +8; unknown cookies ignored.
pub const KIND_UNREGISTER_REPLY: u8 = 0x83;

/// Body bytes before the payload: controller (u8) and 8 opaque bytes.
const REQUEST_PREFIX_LEN: usize = 9;
/// Reply bytes between header and value: status u16 and two u64.
pub const REPLY_PREFIX_LEN: usize = 18;

/// Size of the buffer the title inflates a `get` value into (`sub_82236D40` and `sub_82231D50`
/// pass 0x8000 to `sub_82134FA0`). A larger value does not decode and the get returns no table.
pub const INFLATE_CAPACITY: usize = 0x8000;

/// Decoded v2.0.1149.0 request; `execute_time_utc` 0 means now.
#[derive(Debug, PartialEq)]
pub struct Request<'a> {
    pub controller: u8,
    pub execute_time_utc: u64,
    pub op: Op<'a>,
}

#[derive(Debug, PartialEq)]
pub enum Op<'a> {
    /// payload: time u64, key (u16 len)
    Get { key: &'a [u8] },
    /// payload: time u64, key (u16 len), value (u32 len)
    Set { key: &'a [u8], value: &'a [u8] },
    /// payload: time u64, key (u16 len), delta f64
    SumAdd { key: &'a [u8], delta: f64 },
    /// payload: time u64, key (u16 len), capacity u16, score f64, metadata (u16 len)
    TopPost {
        key: &'a [u8],
        capacity: u16,
        score: f64,
        metadata: &'a [u8],
    },
    /// payload: time u64, xuid u64, key (u16 len)
    UserGet { xuid: u64, key: &'a [u8] },
    /// payload: time u64, xuid u64, key (u16 len), value (u32 len)
    UserSet {
        xuid: u64,
        key: &'a [u8],
        value: &'a [u8],
    },
}

/// `Ok(None)` for a request type that is known but not decoded yet (rank types 7-8).
pub fn parse_request(kind: u8, body: &[u8]) -> Result<Option<Request<'_>>, FrameError> {
    let mut r = Reader(body);
    let controller = r.u8()?;
    r.take(REQUEST_PREFIX_LEN - 1)?;
    let execute_time_utc = r.u64()?;
    let op = match kind {
        KIND_GET => Op::Get { key: r.str16()? },
        KIND_SET => Op::Set {
            key: r.str16()?,
            value: r.blob32()?,
        },
        KIND_SUM => Op::SumAdd {
            key: r.str16()?,
            delta: r.f64()?,
        },
        KIND_TOP => Op::TopPost {
            key: r.str16()?,
            capacity: r.u16()?,
            score: r.f64()?,
            metadata: r.str16()?,
        },
        KIND_USER_GET => Op::UserGet {
            xuid: r.u64()?,
            key: r.str16()?,
        },
        KIND_USER_SET => Op::UserSet {
            xuid: r.u64()?,
            key: r.str16()?,
            value: r.blob32()?,
        },
        _ => return Ok(None),
    };
    if !r.0.is_empty() {
        return Err(FrameError::InvalidNamedBody);
    }
    Ok(Some(Request {
        controller,
        execute_time_utc,
        op,
    }))
}

/// Reply to a request: status, two zero u64, raw value; oversize values become an empty 500.
pub fn reply(cookie: u16, status: u16, value: &[u8]) -> Vec<u8> {
    let (status, value) = if HEADER_LEN + REPLY_PREFIX_LEN + value.len() <= MAX_FRAME {
        (status, value)
    } else {
        (500, &[][..])
    };
    let mut body = Vec::with_capacity(REPLY_PREFIX_LEN + value.len());
    body.extend_from_slice(&status.to_be_bytes());
    body.extend_from_slice(&[0; REPLY_PREFIX_LEN - 2]);
    body.extend_from_slice(value);
    encode(Protocol::V1149, KIND_REPLY, cookie, &body)
}

/// Controller and xuid of an unregister body.
pub fn parse_unregister(body: &[u8]) -> Option<(u8, u64)> {
    let body: &[u8; 17] = body.try_into().ok()?;
    let xuid = u64::from_be_bytes(body[9..].try_into().ok()?);
    Some((body[0], xuid))
}

/// Answer to an unregister: kind 0x83, request cookie, status 200 at +8.
pub fn unregister_reply(cookie: u16) -> Vec<u8> {
    let mut body = [0; REPLY_PREFIX_LEN];
    body[..2].copy_from_slice(&200u16.to_be_bytes());
    encode(Protocol::V1149, KIND_UNREGISTER_REPLY, cookie, &body)
}

/// Answer to a heartbeat: an unregister reply with the heartbeat's cookie, noted as traffic.
pub fn heartbeat_ack(cookie: u16) -> Vec<u8> {
    unregister_reply(cookie)
}

/// Rewrite a backend document for 1149's XSOR reader: `XsorItem` to `xsor:item`, drop the outer Metadata wrap.
/// Anything that is not one XML element (user documents, plain values) is returned unchanged.
pub fn adapt_document(value: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let contains = |needle: &[u8]| value.windows(needle.len()).any(|w| w == needle);
    if !contains(b"<XsorItem") && !contains(b"<Metadata><Metadata") {
        return std::borrow::Cow::Borrowed(value);
    }
    let Some(mut doc) = crate::xsor::table::parse(value) else {
        return std::borrow::Cow::Borrowed(value);
    };
    doc.visit_mut(&mut |node| match node.name() {
        "XsorItem" => node.set_name("xsor:item"),
        "Metadata" => {
            node.unwrap_only_child("Metadata");
        }
        _ => {}
    });
    std::borrow::Cow::Owned(doc.to_bytes())
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FrameError> {
        if self.0.len() < n {
            return Err(FrameError::InvalidNamedBody);
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, FrameError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, FrameError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn f64(&mut self) -> Result<f64, FrameError> {
        Ok(f64::from_bits(self.u64()?))
    }

    fn u64(&mut self) -> Result<u64, FrameError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn str16(&mut self) -> Result<&'a [u8], FrameError> {
        let len = self.u16()?;
        self.take(len as usize)
    }

    fn blob32(&mut self) -> Result<&'a [u8], FrameError> {
        let len = u32::from_be_bytes(self.take(4)?.try_into().unwrap());
        self.take(len as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{FIXTURE_1149, decode, fixture_frame};

    fn request(label: &str) -> (u8, Vec<u8>) {
        lobby_request(FIXTURE_1149, label)
    }

    fn lobby_request(fixture: &str, label: &str) -> (u8, Vec<u8>) {
        let raw = fixture_frame(fixture, label);
        let (frame, used) = decode(&raw).unwrap().unwrap();
        assert_eq!((used, frame.protocol()), (raw.len(), Protocol::V1149));
        (frame.kind, frame.body)
    }

    /// A payload as its text when it is text, else its length: a compressed value is binary.
    fn value_shown(value: &[u8]) -> String {
        const SHOWN: usize = 48;
        let Ok(text) = std::str::from_utf8(value) else {
            return format!("{} bytes", value.len());
        };
        let shown: String = text.chars().take(SHOWN).collect();
        let more = if shown.len() < text.len() { " .." } else { "" };
        format!("{shown:?}{more}")
    }

    fn key_shown(key: &[u8]) -> String {
        String::from_utf8_lossy(key).into_owned()
    }

    /// The parsed request as one line, its key and value as text rather than byte lists.
    fn request_shown(parsed: Option<Request<'_>>) -> String {
        let Some(request) = parsed else {
            return "a request type this backend does not read".to_owned();
        };
        let op = match &request.op {
            Op::Get { key } => format!("get {}", key_shown(key)),
            Op::Set { key, value } => format!("set {} {}", key_shown(key), value_shown(value)),
            Op::SumAdd { key, delta } => format!("sum {} {delta}", key_shown(key)),
            Op::TopPost {
                key,
                capacity,
                score,
                metadata,
            } => format!(
                "top {} capacity {capacity} score {score} {}",
                key_shown(key),
                value_shown(metadata)
            ),
            Op::UserGet { xuid, key } => format!("userget {xuid:016x} {}", key_shown(key)),
            Op::UserSet { xuid, key, value } => {
                format!(
                    "userset {xuid:016x} {} {}",
                    key_shown(key),
                    value_shown(value)
                )
            }
        };
        format!(
            "controller {}, execute time {}: {op}",
            request.controller, request.execute_time_utc
        )
    }

    #[test]
    fn captured_requests_parse_to_their_fields() {
        const LOBBY: &str = include_str!("../../fixtures/sbg-1149-capture-20261019-lobby.txt");
        let cases = [
            (FIXTURE_1149, "get_config"),
            (FIXTURE_1149, "get_pangolin_config"),
            (FIXTURE_1149, "userset_user_error"),
            (LOBBY, "set_the_mob"),
            (LOBBY, "sum_rs_total_player_count"),
            (LOBBY, "top_mob_selection"),
            (LOBBY, "top_the_one"),
        ];
        let shown: Vec<String> = cases
            .iter()
            .map(|(fixture, label)| {
                let (kind, body) = lobby_request(fixture, label);
                format!(
                    "{label}: {}",
                    request_shown(parse_request(kind, &body).unwrap())
                )
            })
            .collect();
        insta::assert_snapshot!(shown.join("\n"));
    }

    #[test]
    fn truncated_or_trailing_bytes_are_malformed() {
        let (kind, body) = request("get_config");
        assert!(parse_request(kind, &body[..body.len() - 1]).is_err());
        let mut long = body.clone();
        long.push(0);
        assert!(parse_request(kind, &long).is_err());
        assert_eq!(parse_request(KIND_RANK_GET, &body).unwrap(), None);
    }

    #[test]
    fn heartbeat_ack_is_a_26_byte_unregister_reply_with_the_heartbeat_cookie() {
        let (kind, _) = request("heartbeat");
        assert_eq!(kind, crate::wire::KIND_HEARTBEAT);
        let heartbeat = fixture_frame(FIXTURE_1149, "heartbeat");
        let ack = heartbeat_ack(u16::from_be_bytes([heartbeat[6], heartbeat[7]]));
        assert_eq!(ack.len(), 26);
        assert_eq!(
            &ack[..10],
            &[3, KIND_UNREGISTER_REPLY, 0, 0, 0, 26, 0, 0x63, 0, 200]
        );
        assert_eq!(&ack[10..], &[0; 16]);
    }

    #[test]
    fn reply_layout_matches_the_client_parser() {
        let bytes = reply(0x66, 200, b"<x/>");
        assert_eq!(bytes.len(), 26 + 4);
        assert_eq!(&bytes[..10], &[3, KIND_REPLY, 0, 0, 0, 30, 0, 0x66, 0, 200]);
        assert_eq!(&bytes[10..26], &[0; 16]);
        assert_eq!(&bytes[26..], b"<x/>");
        assert_eq!(reply(1, 404, &[]).len(), 26);
        let oversized = reply(1, 200, &vec![0; MAX_FRAME]);
        assert_eq!(
            (oversized.len(), &oversized[8..10]),
            (26, &500u16.to_be_bytes()[..])
        );
    }

    #[test]
    fn adapt_document_renames_list_items_and_unwraps_metadata() {
        let doc = b"<Values type=\"table\"><XsorItem type=\"table\"><Value>1</Value><Metadata><Metadata>16140907667073616695</Metadata></Metadata></XsorItem><XsorItem>y</XsorItem></Values>";
        assert_eq!(
            &*adapt_document(doc),
            &b"<Values type=\"table\"><xsor:item type=\"table\"><Value>1</Value><Metadata>16140907667073616695</Metadata></xsor:item><xsor:item>y</xsor:item></Values>"[..]
        );
        assert!(matches!(
            adapt_document(b"<Value>1</Value>"),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    /// A real console's MobChoices post: its `Metadata` element carries a `type` attribute.
    #[test]
    fn adapt_document_unwraps_metadata_with_attributes() {
        let doc = br#"<Values type="table"><XsorItem type="table"><Value>3</Value><Metadata><Metadata type="table"><xuid>16140907667073616695</xuid><correct type="boolean">1</correct></Metadata></Metadata></XsorItem></Values>"#;
        assert_eq!(
            &*adapt_document(doc),
            &br#"<Values type="table"><xsor:item type="table"><Value>3</Value><Metadata type="table"><xuid>16140907667073616695</xuid><correct type="boolean">1</correct></Metadata></xsor:item></Values>"#[..]
        );
    }

    #[test]
    fn adapt_document_leaves_other_values_alone() {
        for value in [
            &b"<Metadata>a</Metadata>"[..],
            b"\x00\x01<XsorItem",
            b"<A><XsorItem>",
        ] {
            assert!(matches!(
                adapt_document(value),
                std::borrow::Cow::Borrowed(_)
            ));
        }
    }
}
