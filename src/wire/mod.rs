//! SBG framing shared by every supported title build: the 8-byte header, frame reassembly and
//! the register exchange. Build-specific request/reply bodies live in the per-build modules.

use std::io::{self, Read};

use zerocopy::{
    FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned,
    byteorder::network_endian::{U16, U32},
};

pub mod v1149;
pub mod v1317;

pub const HEADER_LEN: usize = 8;
pub const MAX_FRAME: usize = 0x7800;

pub const KIND_REGISTER: u8 = 0x00;
pub const KIND_HEARTBEAT: u8 = 0x02;
pub const KIND_REGISTER_REPLY: u8 = 0x80;

/// Title build of a frame, by header version byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// Version byte 3; every store action is a named request.
    V1317,
    /// Version byte 0xFF; one request type per action.
    V1149,
}

impl Protocol {
    /// Configured build (`GAMESERVER_PROTOCOL`).
    pub fn parse(build: &str) -> Option<Self> {
        match build {
            "1149" => Some(Self::V1149),
            "1317" => Some(Self::V1317),
            _ => None,
        }
    }

    pub fn build(self) -> &'static str {
        match self {
            Self::V1149 => "1149",
            Self::V1317 => "1317",
        }
    }

    pub fn from_version(version: u8) -> Option<Self> {
        match version {
            3 => Some(Self::V1317),
            0xFF => Some(Self::V1149),
            _ => None,
        }
    }

    /// Version byte stamped on frames sent to this build.
    pub fn version(self) -> u8 {
        match self {
            Self::V1317 => 3,
            // The 1149 receiver ignores the byte.
            Self::V1149 => 3,
        }
    }
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
pub struct SbgHeader {
    pub version: u8,
    pub kind: u8,
    pub length: U32,
    pub cookie: U16,
}

#[derive(FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RegisterReplyBody {
    status: U16,
    reserved: [u8; 16],
}

#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub version: u8,
    pub kind: u8,
    pub cookie: u16,
    pub body: Vec<u8>,
}

impl Frame {
    /// Build that sent this frame; unknown versions decode as 1317.
    pub fn protocol(&self) -> Protocol {
        Protocol::from_version(self.version).unwrap_or(Protocol::V1317)
    }

    /// Frame as sent on the wire, version byte included.
    pub fn to_bytes(&self) -> Vec<u8> {
        let header = SbgHeader {
            version: self.version,
            kind: self.kind,
            length: U32::new((HEADER_LEN + self.body.len()) as u32),
            cookie: U16::new(self.cookie),
        };
        let mut out = header.as_bytes().to_vec();
        out.extend_from_slice(&self.body);
        out
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    InvalidLength,
    InvalidVersion,
    InvalidNamedBody,
}

/// Validate a frame header; returns it with the total frame length.
fn parse_header(bytes: &[u8]) -> Result<(&SbgHeader, usize), FrameError> {
    let header = SbgHeader::ref_from_bytes(bytes).map_err(|_| FrameError::InvalidLength)?;
    let len = header.length.get() as usize;
    if !(HEADER_LEN..=MAX_FRAME).contains(&len) {
        return Err(FrameError::InvalidLength);
    }
    if Protocol::from_version(header.version).is_none() {
        return Err(FrameError::InvalidVersion);
    }
    Ok((header, len))
}

pub fn decode(input: &[u8]) -> Result<Option<(Frame, usize)>, FrameError> {
    if input.len() < HEADER_LEN {
        return Ok(None);
    }
    let (h, len) = parse_header(&input[..HEADER_LEN])?;
    if input.len() < len {
        return Ok(None);
    }
    Ok(Some((
        Frame {
            version: h.version,
            kind: h.kind,
            cookie: h.cookie.get(),
            body: input[HEADER_LEN..len].to_vec(),
        },
        len,
    )))
}

pub fn register_reply(protocol: Protocol, cookie: u16) -> Vec<u8> {
    let body = RegisterReplyBody {
        status: U16::new(200),
        reserved: [0; 16],
    };
    encode(protocol, KIND_REGISTER_REPLY, cookie, body.as_bytes())
}

pub fn encode(protocol: Protocol, kind: u8, cookie: u16, body: &[u8]) -> Vec<u8> {
    let len = HEADER_LEN + body.len();
    let header = SbgHeader {
        version: protocol.version(),
        kind,
        length: U32::new(len as u32),
        cookie: U16::new(cookie),
    };
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(body);
    out
}

#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Header(FrameError),
}

impl From<io::Error> for ReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Header(error) => write!(f, "invalid SBG frame header: {error:?}"),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Header(_) => None,
        }
    }
}

impl From<ReadError> for io::Error {
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::Io(error) => error,
            header @ ReadError::Header(_) => io::Error::new(io::ErrorKind::InvalidData, header),
        }
    }
}

pub fn read_one<R: Read>(reader: &mut R) -> Result<Frame, ReadError> {
    let mut header_bytes = [0; HEADER_LEN];
    reader.read_exact(&mut header_bytes)?;
    let (h, len) = parse_header(&header_bytes).map_err(ReadError::Header)?;
    let (version, kind, cookie) = (h.version, h.kind, h.cookie.get());
    let mut body = vec![0; len - HEADER_LEN];
    reader.read_exact(&mut body)?;
    Ok(Frame {
        version,
        kind,
        cookie,
        body,
    })
}

/// Raw frames from a `tools/sbg-frames.py` fixture: `<label> <hex>` per line, `#` comments.
#[cfg(test)]
pub(crate) fn fixture_frame(fixture: &str, label: &str) -> Vec<u8> {
    let hex = fixture
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| line.strip_prefix(label)?.strip_prefix(' '))
        .unwrap_or_else(|| panic!("fixture frame {label} missing"));
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

#[cfg(test)]
pub(crate) const FIXTURE_1149: &str = include_str!("../../fixtures/sbg-1149-capture-20261018.txt");

#[cfg(test)]
pub(crate) const FIXTURE_1317: &str = include_str!("../../fixtures/sbg-1317-capture-r1-fresh.txt");

#[cfg(test)]
mod tests {
    #[test]
    fn protocol_parses_from_the_build_number_only() {
        assert_eq!(Protocol::parse("1149"), Some(Protocol::V1149));
        assert_eq!(Protocol::parse("1317"), Some(Protocol::V1317));
        for bad in ["", "v1149", "1149 ", "2.0.1149.0", "1167"] {
            assert_eq!(Protocol::parse(bad), None, "{bad:?}");
        }
    }

    use super::*;

    #[test]
    fn read_one_reports_a_bad_header_apart_from_io_errors() {
        let mut http = &b"GET / HTTP/1.1\r\n"[..];
        assert!(matches!(
            read_one(&mut http),
            Err(ReadError::Header(FrameError::InvalidLength))
        ));
        let mut short = &[0xff, 0x00][..];
        assert!(matches!(read_one(&mut short), Err(ReadError::Io(_))));
    }

    #[test]
    fn frames_serialize_back_to_their_captured_bytes() {
        for (fixture, label) in [
            (FIXTURE_1149, "register"),
            (FIXTURE_1149, "get_config"),
            (FIXTURE_1317, "store_get_game_state"),
            (FIXTURE_1317, "store_set_the_mob_compressed"),
        ] {
            let raw = fixture_frame(fixture, label);
            let (frame, used) = decode(&raw).unwrap().unwrap();
            assert_eq!(used, raw.len(), "{label}");
            assert_eq!(frame.to_bytes(), raw, "{label}");
        }
    }

    #[test]
    fn accepts_retail_1149_register_frame() {
        // Captured 2026-10-18: v2.0.1149.0 register (kind 0, len 26, cookie 0x65).
        let raw = [
            0xff, 0x00, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x65, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0xe0, 0x00, 0x06, 0x01, 0x48, 0x4d, 0x4b, 0x37, 0x00,
        ];
        let (frame, used) = decode(&raw).unwrap().unwrap();
        assert_eq!(
            (frame.version, frame.kind, frame.cookie, used),
            (0xff, KIND_REGISTER, 0x65, 26)
        );
        assert_eq!(frame.protocol(), Protocol::V1149);
        let mut bad = raw;
        bad[0] = 2;
        assert_eq!(decode(&bad), Err(FrameError::InvalidVersion));
    }

    #[test]
    fn decodes_captured_1317_register_and_heartbeat() {
        let register = fixture_frame(FIXTURE_1317, "register");
        let (frame, used) = decode(&register).unwrap().unwrap();
        assert_eq!((frame.kind, frame.cookie, used), (KIND_REGISTER, 1, 26));
        assert_eq!(frame.protocol(), Protocol::V1317);
        let heartbeat = fixture_frame(FIXTURE_1317, "heartbeat");
        let (frame, _) = decode(&heartbeat).unwrap().unwrap();
        assert_eq!(frame.kind, KIND_HEARTBEAT);
    }

    #[test]
    fn register_reply_matches_captured_1317_bytes() {
        assert_eq!(
            register_reply(Protocol::V1317, 1),
            fixture_frame(FIXTURE_1317, "reply_register")
        );
    }

    #[test]
    fn waits_for_partial_frame_and_decodes_coalesced_frames() {
        let a = encode(Protocol::V1317, 2, 7, &[]);
        assert_eq!(decode(&a[..4]).unwrap(), None);
        let b = encode(Protocol::V1317, 0, 8, &[1, 2]);
        let mut both = a.clone();
        both.extend_from_slice(&b);
        let (first, used) = decode(&both).unwrap().unwrap();
        let (second, used2) = decode(&both[used..]).unwrap().unwrap();
        assert_eq!((first.kind, first.cookie), (2, 7));
        assert_eq!((second.kind, second.cookie), (0, 8));
        assert_eq!(used + used2, both.len());
    }

    #[test]
    fn rejects_oversized_frame() {
        let mut header = [0; 8];
        header[0] = 3;
        header[2..6].copy_from_slice(&((MAX_FRAME as u32) + 1).to_be_bytes());
        assert_eq!(decode(&header), Err(FrameError::InvalidLength));
    }
}
