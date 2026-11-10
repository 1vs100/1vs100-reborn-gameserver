use std::io;
use std::net::UdpSocket;
use std::time::{SystemTime, UNIX_EPOCH};

const NTP_PACKET_LEN: usize = 48;
const NTP_UNIX_EPOCH_DELTA: u64 = 2_208_988_800;

pub fn response(request: &[u8], unix_time: SystemTime) -> Option<[u8; 48]> {
    if request.len() < NTP_PACKET_LEN {
        return None;
    }

    let elapsed = unix_time.duration_since(UNIX_EPOCH).ok()?;
    let seconds = (elapsed.as_secs() + NTP_UNIX_EPOCH_DELTA) as u32;
    let fraction = ((u64::from(elapsed.subsec_nanos()) << 32) / 1_000_000_000) as u32;

    let mut packet = [0u8; NTP_PACKET_LEN];
    packet[0] = (request[0] & 0b00_111_000) | 4;
    packet[1] = 1;
    packet[2] = 6;
    packet[3] = 0xEC;
    packet[12..16].copy_from_slice(b"LOCL");
    packet[24..32].copy_from_slice(&request[40..48]);

    let mut timestamp = [0u8; 8];
    timestamp[..4].copy_from_slice(&seconds.to_be_bytes());
    timestamp[4..].copy_from_slice(&fraction.to_be_bytes());
    packet[16..24].copy_from_slice(&timestamp);
    packet[32..40].copy_from_slice(&timestamp);
    packet[40..48].copy_from_slice(&timestamp);

    Some(packet)
}

pub fn serve(socket: &UdpSocket) -> io::Result<()> {
    tracing::info!(addr = ?socket.local_addr().ok(), "SNTP listener started");
    let mut buf = [0u8; 512];
    loop {
        let (len, from) = socket.recv_from(&mut buf)?;
        if let Some(packet) = response(&buf[..len], SystemTime::now()) {
            tracing::trace!(%from, "SNTP request");
            socket.send_to(&packet, from)?;
        } else {
            tracing::debug!(%from, len, "ignored short SNTP datagram");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    const NTP_UNIX_EPOCH_DELTA: u64 = 2_208_988_800;

    #[test]
    fn response_builds_ntp_packet_from_request_and_time() {
        let mut request = [0u8; 48];
        request[0] = 0b00_011_011;
        request[40..48].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let unix_time = UNIX_EPOCH + Duration::new(1_700_000_000, 500_000_000);

        let packet = response(&request, unix_time).expect("valid request gets response");

        assert_eq!(packet.len(), 48);
        assert_eq!(packet[0], 0b00_011_100);
        assert_eq!(packet[1], 1);
        assert_eq!(packet[2], 6);
        assert_eq!(packet[3], 0xEC);
        assert_eq!(&packet[4..8], &[0, 0, 0, 0]);
        assert_eq!(&packet[8..12], &[0, 0, 0, 0]);
        assert_eq!(&packet[12..16], b"LOCL");
        assert_eq!(&packet[24..32], &request[40..48]);

        let seconds = (NTP_UNIX_EPOCH_DELTA + 1_700_000_000).to_be_bytes();
        let fraction = (1u32 << 31).to_be_bytes();
        let timestamp = [
            seconds[4],
            seconds[5],
            seconds[6],
            seconds[7],
            fraction[0],
            fraction[1],
            fraction[2],
            fraction[3],
        ];
        assert_eq!(&packet[32..40], &timestamp);
        assert_eq!(&packet[40..48], &timestamp);
    }

    #[test]
    fn response_ignores_short_requests() {
        assert_eq!(response(&[0u8; 47], UNIX_EPOCH), None);
    }
}
