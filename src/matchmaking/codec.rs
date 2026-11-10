//! The wire format: frame header, message kinds, payload reading and writing; shared with
//! `bin/mock_peer`.

use std::{
    fmt,
    io::{self, Read},
};

use crate::{groups::GroupId, ids::Xuid};

/// Every frame starts with it, both ways; a frame with another version is answered with REJECT.
pub const VERSION: u8 = 2;

/// `version u8 | type u8 | length u32 BE`.
pub(super) const HEADER_LEN: usize = 6;

/// The title caps matchmaking frames at 512 bytes in both directions.
pub(super) const MAX_MM_FRAME: usize = 512;

/// Avatar uploads carry a 1000-byte metadata blob, so inbound frames may be larger than the
/// 512-byte cap that still applies to replies the title has to read back into its buffers.
pub(super) const MAX_INBOUND_PAYLOAD: usize = 9000;

const MAX_STRING_BYTES: usize = 256;

/// Message types (`type u8` of every frame).
pub const JOIN: u8 = 0x01;
pub const JOIN_ACK: u8 = 0x02;
pub const HEARTBEAT: u8 = 0x03;
pub const HEARTBEAT_ACK: u8 = 0x04;
pub const LEAVE: u8 = 0x05;
pub const ROSTER: u8 = 0x06;
pub const ROSTER_REPLY: u8 = 0x07;
pub const ANNOUNCE: u8 = 0x08;
pub const AVATAR_PUT: u8 = 0x09;
pub const AVATAR_ACK: u8 = 0x0A;
pub const AVATAR_GET: u8 = 0x0B;
pub const AVATAR_REPLY: u8 = 0x0C;
pub const RELAY_PUT: u8 = 0x0D;
pub const RELAY_ACK: u8 = 0x0E;
pub const RELAY_POLL: u8 = 0x0F;
pub const RELAY_REPLY: u8 = 0x10;
pub const REJECT: u8 = 0x11;

/// Why a matchmaking payload does not decode.
#[derive(Debug)]
pub enum CodecError {
    /// The payload ended inside `what`.
    Truncated { what: &'static str },
    /// A string is longer than the codec's 256-byte cap, or than the wire's length field.
    StringTooLong { len: usize },
    /// A string is not UTF-8.
    BadUtf8,
    /// Bytes follow the end of the payload.
    TrailingBytes { len: usize },
    /// A count does not fit the wire field it goes into.
    CountTooLarge { count: usize },
    /// A frame is longer than [`MAX_INBOUND_PAYLOAD`].
    FrameTooLarge { len: usize },
    /// A roster body does not fit the 512-byte reply frame.
    RosterTooLarge { len: usize },
    /// A reply frame is of another protocol version.
    WrongVersion { version: u8 },
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::Truncated { what } => write!(f, "truncated {what}"),
            CodecError::StringTooLong { len } => write!(f, "string of {len} bytes is too long"),
            CodecError::BadUtf8 => f.write_str("string is not UTF-8"),
            CodecError::TrailingBytes { len } => write!(f, "{len} trailing bytes"),
            CodecError::CountTooLarge { count } => {
                write!(f, "count {count} does not fit the wire")
            }
            CodecError::FrameTooLarge { len } => write!(f, "frame of {len} bytes is too large"),
            CodecError::RosterTooLarge { len } => {
                write!(f, "roster of {len} bytes exceeds the frame cap")
            }
            CodecError::WrongVersion { version } => {
                write!(f, "frame of protocol version {version}")
            }
        }
    }
}

impl std::error::Error for CodecError {}

#[derive(Debug)]
pub(super) struct Frame {
    pub(super) version: u8,
    pub(super) kind: u8,
    pub(super) payload: Vec<u8>,
}

pub(super) fn u32_of(count: usize) -> Result<u32, CodecError> {
    u32::try_from(count).map_err(|_| CodecError::CountTooLarge { count })
}

pub(super) fn encode(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.push(VERSION);
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// A frame as a client sends it (`bin/mock_peer`, tests).
pub fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    encode(kind, payload)
}

/// A JOIN payload: the players, user 0 first.
pub fn join_payload(players: &[(Xuid, &str)]) -> Vec<u8> {
    let mut payload = vec![players.len() as u8];
    for (xuid, gamertag) in players {
        payload.extend_from_slice(&xuid.to_be_bytes());
        payload.extend_from_slice(&(gamertag.len() as u16).to_be_bytes());
        payload.extend_from_slice(gamertag.as_bytes());
    }
    payload
}

/// `Ok(None)` means the peer closed or went idle; other I/O errors propagate.
fn read_or_idle(stream: &mut impl Read, buf: &mut [u8]) -> io::Result<bool> {
    match stream.read_exact(buf) {
        Ok(()) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// A frame of another version is returned without its payload: its length is not trusted.
pub(super) fn read_frame(stream: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut header = [0; HEADER_LEN];
    if !read_or_idle(stream, &mut header)? {
        return Ok(None);
    }
    let (version, kind) = (header[0], header[1]);
    if version != VERSION {
        return Ok(Some(Frame {
            version,
            kind,
            payload: Vec::new(),
        }));
    }
    let len = u32::from_be_bytes(header[2..6].try_into().unwrap()) as usize;
    if len > MAX_INBOUND_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            CodecError::FrameTooLarge { len },
        ));
    }
    let mut payload = vec![0; len];
    if !read_or_idle(stream, &mut payload)? {
        return Ok(None);
    }
    Ok(Some(Frame {
        version,
        kind,
        payload,
    }))
}

/// A client's read of the reply to its request: `(type, payload)` of the first frame that is not
/// a pushed ANNOUNCE, as the loader reads. A frame of another version is an error.
pub fn read_reply(stream: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    loop {
        let frame = read_frame(stream)?.ok_or(io::ErrorKind::UnexpectedEof)?;
        if frame.version != VERSION {
            let version = frame.version;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                CodecError::WrongVersion { version },
            ));
        }
        if frame.kind != ANNOUNCE {
            return Ok((frame.kind, frame.payload));
        }
    }
}

/// A player as a roster or a JOIN carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterPlayer {
    pub xuid: Xuid,
    pub gamertag: String,
}

/// A ROSTER_REPLY or ANNOUNCE payload: its consoles, each with its players, user 0 first, then
/// the group id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roster {
    pub consoles: Vec<Vec<RosterPlayer>>,
    /// Absent when the receiver is not in a group.
    pub group: Option<GroupId>,
}

pub fn decode_roster(payload: &[u8]) -> Result<Roster, CodecError> {
    let mut cursor = Cursor(payload);
    let consoles = u16::from_be_bytes(cursor.take(2, "console count")?.try_into().unwrap());
    let mut out = Vec::with_capacity(consoles.into());
    for _ in 0..consoles {
        let players = cursor.take(1, "player count")?[0];
        let mut console = Vec::with_capacity(players.into());
        for _ in 0..players {
            console.push(RosterPlayer {
                xuid: cursor.xuid()?,
                gamertag: cursor.string()?,
            });
        }
        out.push(console);
    }
    let group = GroupId::new(u32::from_be_bytes(
        cursor.take(4, "group")?.try_into().unwrap(),
    ));
    cursor.finish()?;
    Ok(Roster {
        consoles: out,
        group,
    })
}

/// The group field of a roster body: 0 stands for a receiver that is in no group.
pub(super) fn put_group(out: &mut Vec<u8>, group: Option<GroupId>) {
    out.extend_from_slice(&group.map_or(0, GroupId::get).to_be_bytes());
}

/// Sequential big-endian reader over a request payload.
pub(super) struct Cursor<'a>(pub(super) &'a [u8]);

impl<'a> Cursor<'a> {
    pub(super) fn take(&mut self, len: usize, what: &'static str) -> Result<&'a [u8], CodecError> {
        let bytes = self.0.get(..len).ok_or(CodecError::Truncated { what })?;
        self.0 = &self.0[len..];
        Ok(bytes)
    }

    pub(super) fn string(&mut self) -> Result<String, CodecError> {
        let len = u16::from_be_bytes(self.take(2, "string length")?.try_into().unwrap()) as usize;
        if len > MAX_STRING_BYTES {
            return Err(CodecError::StringTooLong { len });
        }
        let bytes = self.take(len, "string")?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| CodecError::BadUtf8)
    }

    pub(super) fn xuid(&mut self) -> Result<Xuid, CodecError> {
        let bytes = self.take(8, "xuid")?.try_into().unwrap();
        Ok(Xuid(u64::from_be_bytes(bytes)))
    }

    pub(super) fn finish(self) -> Result<(), CodecError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(CodecError::TrailingBytes { len: self.0.len() })
        }
    }
}

pub(super) fn put_string(out: &mut Vec<u8>, string: &str) -> Result<(), CodecError> {
    if string.len() > MAX_STRING_BYTES || string.len() > u16::MAX as usize {
        return Err(CodecError::StringTooLong { len: string.len() });
    }
    out.extend_from_slice(&(string.len() as u16).to_be_bytes());
    out.extend_from_slice(string.as_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A roster payload as the server writes it: the consoles' players, then the group.
    fn roster_bytes(consoles: &[&[(u64, &str)]], group: u32) -> Vec<u8> {
        let mut out = (consoles.len() as u16).to_be_bytes().to_vec();
        for players in consoles {
            out.push(players.len() as u8);
            for (xuid, gamertag) in *players {
                out.extend_from_slice(&xuid.to_be_bytes());
                out.extend_from_slice(&(gamertag.len() as u16).to_be_bytes());
                out.extend_from_slice(gamertag.as_bytes());
            }
        }
        out.extend_from_slice(&group.to_be_bytes());
        out
    }

    /// One console with one player, up to the player's gamertag length.
    fn up_to_the_gamertag_length() -> Vec<u8> {
        let mut payload = vec![0, 1, 1];
        payload.extend_from_slice(&1u64.to_be_bytes());
        payload
    }

    #[test]
    fn roster_payloads_decode_to_their_consoles_and_group() {
        let mut string_too_long = vec![0, 1, 1];
        string_too_long.extend_from_slice(&1u64.to_be_bytes());
        string_too_long.extend_from_slice(&((MAX_STRING_BYTES + 1) as u16).to_be_bytes());
        let mut bad_utf8 = vec![0, 1, 1];
        bad_utf8.extend_from_slice(&1u64.to_be_bytes());
        bad_utf8.extend_from_slice(&1u16.to_be_bytes());
        bad_utf8.push(0xFF);
        let mut trailing = roster_bytes(&[&[(1, "A")]], 1);
        trailing.push(0);

        let cases = [
            (
                "two consoles",
                roster_bytes(&[&[(1, "A0"), (2, "A1")], &[(3, "B0")]], 1),
            ),
            ("no group", roster_bytes(&[&[(1, "A")]], 0)),
            ("no consoles", roster_bytes(&[], 0)),
            ("the console count cut short", vec![0]),
            (
                "the player count cut short",
                roster_bytes(&[&[]], 1)[..2].to_vec(),
            ),
            (
                "the xuid cut short",
                roster_bytes(&[&[(1, "A")]], 1)[..5].to_vec(),
            ),
            (
                "the string length cut short",
                [up_to_the_gamertag_length(), vec![0]].concat(),
            ),
            (
                "the string cut short",
                [up_to_the_gamertag_length(), vec![0, 4, b'A']].concat(),
            ),
            ("a string too long", string_too_long),
            ("a gamertag that is not UTF-8", bad_utf8),
            ("the group cut short", roster_bytes(&[&[]], 1)[..4].to_vec()),
            ("trailing bytes", trailing),
        ];
        let shown: Vec<String> = cases
            .into_iter()
            .map(|(what, payload)| format!("{what}: {:?}", decode_roster(&payload)))
            .collect();
        insta::assert_snapshot!(shown.join("\n"));
    }

    /// A reply as the loader reads it, with its I/O error kind and message.
    fn reply_shown(reply: io::Result<(u8, Vec<u8>)>) -> String {
        match reply {
            Ok((kind, payload)) => format!("type {kind}, {payload:02x?}"),
            Err(error) => format!("{:?}: {error}", error.kind()),
        }
    }

    #[test]
    fn a_reply_is_the_first_frame_that_is_not_an_announce() {
        let mut stream = Vec::new();
        stream.extend(frame(ANNOUNCE, &[0, 1, 0, 0, 0, 0, 0, 1]));
        stream.extend(frame(ROSTER_REPLY, &[0, 0]));

        let cases = [
            ("after an announce", read_reply(&mut &stream[..])),
            (
                "first frame",
                read_reply(&mut &frame(HEARTBEAT_ACK, &[0; 4])[..]),
            ),
            (
                "another protocol version",
                read_reply(&mut &[1, ROSTER_REPLY, 0, 0, 0, 0][..]),
            ),
            ("no frame", read_reply(&mut &[][..])),
        ];
        let shown: Vec<String> = cases
            .into_iter()
            .map(|(what, reply)| format!("{what}: {}", reply_shown(reply)))
            .collect();
        insta::assert_snapshot!(shown.join("\n"));
    }
}
