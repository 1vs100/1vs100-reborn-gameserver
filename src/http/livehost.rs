//! Live host audio as the title's Windows Media player reads it over HTTP: an ASF file split
//! into MMS-over-HTTP chunks, one header chunk, one chunk per data packet and an end chunk.

use std::{
    fmt, io,
    path::{Path, PathBuf},
    sync::Arc,
};

/// 2 minutes of WMA; the title fetches it for each live host segment and plays it from the start;
/// `scripts/generate_livehost_audio.sh` writes it.
const LIVEHOST_ASF: &[u8] = include_bytes!("../../assets/livehost.wma");

/// The live host audio framed for the title's player, ready to send.
#[derive(Clone, Debug)]
pub struct LivehostStream(Arc<[u8]>);

impl LivehostStream {
    /// The stream of `file`, else of the built-in asset.
    pub fn load(file: Option<&Path>) -> Result<Self, LivehostError> {
        let stream = match file {
            None => mmsh_stream(LIVEHOST_ASF)
                .map_err(|source| LivehostError::Asf { path: None, source })?,
            Some(path) => {
                let asf = std::fs::read(path).map_err(|source| LivehostError::Read {
                    path: path.to_owned(),
                    source,
                })?;
                mmsh_stream(&asf).map_err(|source| LivehostError::Asf {
                    path: Some(path.to_owned()),
                    source,
                })?
            }
        };
        Ok(Self(stream.into()))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug)]
pub enum LivehostError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    /// `path` is `None` for the built-in asset.
    Asf {
        path: Option<PathBuf>,
        source: AsfError,
    },
}

impl fmt::Display for LivehostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "livehost file {}: {source}", path.display())
            }
            Self::Asf {
                path: Some(path),
                source,
            } => write!(f, "livehost file {}: {source}", path.display()),
            Self::Asf { path: None, source } => write!(f, "built-in livehost asset: {source}"),
        }
    }
}

impl std::error::Error for LivehostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Asf { source, .. } => Some(source),
        }
    }
}

const HEADER_OBJECT: [u8; 16] = guid(
    0x75B22630,
    0x668E,
    0x11CF,
    *b"\xA6\xD9\x00\xAA\x00\x62\xCE\x6C",
);
const FILE_PROPERTIES_OBJECT: [u8; 16] = guid(
    0x8CABDCA1,
    0xA947,
    0x11CF,
    *b"\x8E\xE4\x00\xC0\x0C\x20\x53\x65",
);
const DATA_OBJECT: [u8; 16] = guid(
    0x75B22636,
    0x668E,
    0x11CF,
    *b"\xA6\xD9\x00\xAA\x00\x62\xCE\x6C",
);
/// Data object header: GUID, size, file id, packet count, reserved.
const DATA_OBJECT_HEADER_LEN: usize = 50;

/// Chunk types, `$` with the high bit set on the header and data chunks.
const CHUNK_HEADER: [u8; 2] = *b"\xA4H";
const CHUNK_DATA: [u8; 2] = *b"\xA4D";
const CHUNK_END: [u8; 2] = *b"$E";
const HEADER_CHUNK_FLAGS: u8 = 0x0C;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsfError {
    Truncated {
        offset: usize,
    },
    UnexpectedObject {
        offset: usize,
        expected: &'static str,
    },
    NoFileProperties,
    PacketSize {
        min: u32,
        max: u32,
    },
    PartialPacket {
        len: usize,
        packet_size: usize,
    },
}

impl fmt::Display for AsfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { offset } => write!(f, "ASF truncated at offset {offset}"),
            Self::UnexpectedObject { offset, expected } => {
                write!(f, "ASF object at offset {offset} is not the {expected}")
            }
            Self::NoFileProperties => f.write_str("ASF header has no file properties object"),
            Self::PacketSize { min, max } => write!(
                f,
                "ASF packet size {min} to {max} is not fixed or does not fit a chunk"
            ),
            Self::PartialPacket { len, packet_size } => write!(
                f,
                "ASF data of {len} bytes is not whole packets of {packet_size}"
            ),
        }
    }
}

impl std::error::Error for AsfError {}

const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> [u8; 16] {
    let a = data1.to_le_bytes();
    let b = data2.to_le_bytes();
    let c = data3.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], data4[0], data4[1], data4[2], data4[3],
        data4[4], data4[5], data4[6], data4[7],
    ]
}

fn bytes<const N: usize>(asf: &[u8], offset: usize) -> Result<[u8; N], AsfError> {
    asf.get(offset..offset + N)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(AsfError::Truncated { offset })
}

fn object_len(asf: &[u8], offset: usize) -> Result<usize, AsfError> {
    let len = u64::from_le_bytes(bytes(asf, offset + 16)?);
    usize::try_from(len)
        .ok()
        .filter(|len| offset.checked_add(*len).is_some_and(|end| end <= asf.len()))
        .ok_or(AsfError::Truncated { offset })
}

fn expect_object(
    asf: &[u8],
    offset: usize,
    id: [u8; 16],
    expected: &'static str,
) -> Result<(), AsfError> {
    if bytes::<16>(asf, offset)? == id {
        Ok(())
    } else {
        Err(AsfError::UnexpectedObject { offset, expected })
    }
}

/// The fixed data packet size from the file properties object.
fn packet_size(header: &[u8]) -> Result<usize, AsfError> {
    let object_count = u32::from_le_bytes(bytes(header, 24)?);
    // Header object: GUID, size, object count, two reserved bytes.
    let mut offset = 30;
    for _ in 0..object_count {
        if bytes::<16>(header, offset)? == FILE_PROPERTIES_OBJECT {
            // GUID, size, file id, file size, date, packet count, play, send, preroll, flags.
            let min = u32::from_le_bytes(bytes(header, offset + 92)?);
            let max = u32::from_le_bytes(bytes(header, offset + 96)?);
            return match usize::try_from(min) {
                Ok(size) if min == max && size > 0 && size + 8 <= usize::from(u16::MAX) => Ok(size),
                _ => Err(AsfError::PacketSize { min, max }),
            };
        }
        offset += object_len(header, offset)?;
    }
    Err(AsfError::NoFileProperties)
}

/// One chunk: type, length of the rest, location (packet number), incarnation, flags, length.
fn push_chunk(out: &mut Vec<u8>, kind: [u8; 2], location: u32, flags: u8, payload: &[u8]) {
    // Callers bound payloads by the packet size, checked to fit a u16 with the 8-byte prefix.
    let len = u16::try_from(payload.len() + 8).expect("chunk payload fits u16");
    out.extend_from_slice(&kind);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&location.to_le_bytes());
    out.push(0);
    out.push(flags);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
}

/// Frames a whole ASF file (header object, data object, optional index) as an MMSH stream.
pub fn mmsh_stream(asf: &[u8]) -> Result<Vec<u8>, AsfError> {
    expect_object(asf, 0, HEADER_OBJECT, "header object")?;
    let header_len = object_len(asf, 0)?;
    let packet_size = packet_size(&asf[..header_len])?;
    expect_object(asf, header_len, DATA_OBJECT, "data object")?;
    let data_end = header_len + object_len(asf, header_len)?;
    let packets_start = header_len + DATA_OBJECT_HEADER_LEN;
    let packets = asf
        .get(packets_start..data_end)
        .ok_or(AsfError::Truncated {
            offset: packets_start,
        })?;
    if packets.len() % packet_size != 0 {
        return Err(AsfError::PartialPacket {
            len: packets.len(),
            packet_size,
        });
    }

    let mut out = Vec::with_capacity(asf.len() + (packets.len() / packet_size + 2) * 12);
    push_chunk(
        &mut out,
        CHUNK_HEADER,
        0,
        HEADER_CHUNK_FLAGS,
        &asf[..packets_start],
    );
    for (location, packet) in (0u32..).zip(packets.chunks_exact(packet_size)) {
        // The flags byte carries the low byte of the packet number.
        push_chunk(&mut out, CHUNK_DATA, location, location as u8, packet);
    }
    out.extend_from_slice(&CHUNK_END);
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file in the temp directory, removed when dropped, so a failing test leaves none behind.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!("{name}-{}.wma", std::process::id())))
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn chunks(stream: &[u8]) -> Vec<(&[u8], &[u8])> {
        let mut chunks = Vec::new();
        let mut rest = stream;
        while !rest.is_empty() {
            let len = usize::from(u16::from_le_bytes([rest[2], rest[3]]));
            chunks.push((&rest[..2], &rest[4..4 + len]));
            rest = &rest[4 + len..];
        }
        chunks
    }

    #[test]
    fn livehost_asset_frames_as_header_packets_and_end() {
        let stream = LivehostStream::load(None).expect("asset frames");
        let chunks = chunks(stream.bytes());
        let (kind, header) = chunks[0];
        assert_eq!(kind, CHUNK_HEADER);
        assert_eq!(&header[..8], &[0, 0, 0, 0, 0, 0x0C, header[6], header[7]]);
        assert_eq!(&header[8..24], &HEADER_OBJECT);
        let data = &chunks[1..chunks.len() - 1];
        let header_len = object_len(LIVEHOST_ASF, 0).unwrap();
        let data_len = object_len(LIVEHOST_ASF, header_len).unwrap();
        assert_eq!(
            data.len() * 3200,
            data_len - DATA_OBJECT_HEADER_LEN,
            "every packet"
        );
        for (location, (kind, chunk)) in data.iter().enumerate() {
            assert_eq!(*kind, CHUNK_DATA);
            assert_eq!(chunk.len(), 8 + 3200);
            assert_eq!(&chunk[..4], &(location as u32).to_le_bytes());
            assert_eq!(chunk[5], location as u8);
            // ASF packet with error correction data: flags 0x82, two bytes of zero.
            assert_eq!(&chunk[8..11], &[0x82, 0, 0]);
        }
        assert_eq!(
            chunks[chunks.len() - 1],
            (&CHUNK_END[..], &[0u8, 0, 0, 0][..])
        );
    }

    #[test]
    fn rejects_non_asf_input() {
        assert_eq!(
            mmsh_stream(&[0; 64]),
            Err(AsfError::UnexpectedObject {
                offset: 0,
                expected: "header object"
            })
        );
        assert_eq!(
            mmsh_stream(&LIVEHOST_ASF[..100]),
            Err(AsfError::Truncated { offset: 0 })
        );
    }

    #[test]
    fn a_livehost_file_replaces_the_built_in_asset() {
        let file = TempFile::new("livehost-file");
        let path = &file.0;
        let mut asf = LIVEHOST_ASF.to_vec();
        // A different file: the last packet's padding byte.
        *asf.last_mut().unwrap() ^= 0xFF;
        std::fs::write(path, &asf).unwrap();
        let served = LivehostStream::load(Some(path)).unwrap();
        let built_in = LivehostStream::load(None).unwrap();
        assert_eq!(served.bytes().len(), built_in.bytes().len());
        assert_ne!(served.bytes(), built_in.bytes());
    }

    #[test]
    fn a_livehost_file_that_is_missing_or_not_asf_names_the_file() {
        let file = TempFile::new("livehost-bad");
        let path = &file.0;
        assert!(matches!(
            LivehostStream::load(Some(path)),
            Err(LivehostError::Read { path: p, .. }) if p == *path
        ));
        std::fs::write(path, [0; 64]).unwrap();
        assert!(matches!(
            LivehostStream::load(Some(path)),
            Err(LivehostError::Asf { path: Some(p), source: AsfError::UnexpectedObject { .. } })
                if p == *path
        ));
    }
}
