//! Simulated consoles for exercising QNet groups without hardware.
//!
//!   [MOCK_RELAY=1] [MOCK_AVATAR=1] mock_peer <addr:port> <count> [seconds]
//!
//! `MOCK_RELAY` and `MOCK_AVATAR` stay off with real consoles: junk payloads break them.

use std::{
    io::Write,
    net::TcpStream,
    time::{Duration, Instant},
};

use onevs100_gameserver::{
    ids::Xuid,
    matchmaking::{self, AVATAR_PUT, HEARTBEAT, JOIN, LEAVE, RELAY_POLL, RELAY_PUT, ROSTER},
};

/// One request and its reply, through the server's own codec (protocol version 2).
fn call(stream: &mut TcpStream, kind: u8, payload: &[u8]) -> std::io::Result<(u8, Vec<u8>)> {
    stream.write_all(&matchmaking::frame(kind, payload))?;
    matchmaking::read_reply(stream)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:1001".into());
    let count: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(3);
    let lifetime = args.next().and_then(|v| v.parse::<u64>().ok());
    #[allow(clippy::disallowed_methods)] // a developer tool's switches, not server settings
    let relay = std::env::var_os("MOCK_RELAY").is_some();
    #[allow(clippy::disallowed_methods)]
    let avatar = std::env::var_os("MOCK_AVATAR").is_some();

    let mut peers = Vec::new();
    for n in 1..=count {
        let xuid = Xuid(0xE0FF_0000_0000_0000 + n);
        let mut stream = TcpStream::connect(&addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let name = format!("Mock{n}");
        call(
            &mut stream,
            JOIN,
            &matchmaking::join_payload(&[(xuid, &name)]),
        )?;
        if avatar {
            let mut put = xuid.to_be_bytes().to_vec();
            put.extend_from_slice(&1000u16.to_be_bytes());
            put.extend(std::iter::repeat_n(n as u8, 1000));
            call(&mut stream, AVATAR_PUT, &put)?;
        }
        peers.push((xuid, n, stream));
    }

    let started = Instant::now();
    loop {
        for (xuid, n, stream) in &mut peers {
            call(stream, HEARTBEAT, &xuid.to_be_bytes())?;
            let (_, relayed) = call(stream, RELAY_POLL, &xuid.to_be_bytes())?;
            let received = u16::from_be_bytes(relayed[..2].try_into().unwrap());
            let (_, body) = call(stream, ROSTER, &[])?;
            let roster = matchmaking::decode_roster(&body)?;
            let names: Vec<_> = roster
                .consoles
                .iter()
                .filter(|players| players.first().is_some_and(|first| first.xuid != *xuid))
                .flatten()
                .map(|player| player.gamertag.as_str())
                .collect();
            let group = match roster.group {
                Some(group) => group.to_string(),
                None => "none".to_owned(),
            };
            println!("Mock{n}: group {group}, roster {names:?}, relayed in {received}");
            if relay {
                let mut put = xuid.to_be_bytes().to_vec();
                put.extend_from_slice(&0u16.to_be_bytes());
                let payload = format!("<mock from=\"{n}\"/>");
                put.extend_from_slice(&(payload.len() as u16).to_be_bytes());
                put.extend_from_slice(payload.as_bytes());
                call(stream, RELAY_PUT, &put)?;
            }
        }
        if lifetime.is_some_and(|secs| started.elapsed().as_secs() >= secs) {
            break;
        }
        std::thread::sleep(Duration::from_secs(10));
    }
    for (xuid, _, stream) in &mut peers {
        stream.write_all(&matchmaking::frame(LEAVE, &xuid.to_be_bytes()))?;
    }
    Ok(())
}
