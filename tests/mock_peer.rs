//! `bin/mock_peer` speaks the server's matchmaking protocol: two simulated consoles join, see
//! each other in one group, and leave.

use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use onevs100_gameserver::matchmaking::{self, Members};

#[test]
fn two_mock_consoles_join_one_group_and_leave() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let members = Arc::new(Mutex::new(Members::default()));
    let served = Arc::clone(&members);
    std::thread::spawn(move || {
        matchmaking::serve(listener, served, matchmaking::Registry::default())
    });

    // Lifetime 0: one round of heartbeat, relay poll and roster, then LEAVE.
    let output = Command::new(env!("CARGO_BIN_EXE_mock_peer"))
        .args([addr.to_string(), "2".into(), "0".into()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(r#"Mock1: group 1, roster ["Mock2"]"#),
        "{stdout}"
    );
    assert!(
        stdout.contains(r#"Mock2: group 1, roster ["Mock1"]"#),
        "{stdout}"
    );

    // Both left: their places are held for the grace period.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let members = onevs100_gameserver::sync::lock(&members);
        if members.snapshot().is_empty() && members.reservations_snapshot().len() == 2 {
            break;
        }
        drop(members);
        assert!(Instant::now() < deadline, "the mock consoles did not leave");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A server that answers the mock peer's requests, but cuts its roster reply short of the group.
fn malformed_roster_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        loop {
            let mut header = [0; 6];
            if stream.read_exact(&mut header).is_err() {
                return;
            }
            let len = u32::from_be_bytes(header[2..6].try_into().unwrap()) as usize;
            let mut payload = vec![0; len];
            stream.read_exact(&mut payload).unwrap();
            let reply = match header[1] {
                matchmaking::JOIN => {
                    matchmaking::frame(matchmaking::JOIN_ACK, &[0, 0, 0, 1, 0, 0, 0, 1])
                }
                matchmaking::HEARTBEAT => {
                    matchmaking::frame(matchmaking::HEARTBEAT_ACK, &[0, 0, 0, 1])
                }
                matchmaking::RELAY_POLL => matchmaking::frame(matchmaking::RELAY_REPLY, &[0, 0]),
                // One console with no players, and then no group.
                _ => matchmaking::frame(matchmaking::ROSTER_REPLY, &[0, 1, 0]),
            };
            stream.write_all(&reply).unwrap();
        }
    });
    addr
}

#[test]
fn a_malformed_roster_fails_the_mock_peer() {
    let addr = malformed_roster_server();
    let output = Command::new(env!("CARGO_BIN_EXE_mock_peer"))
        .args([addr.to_string(), "1".into(), "0".into()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains(r#"Truncated { what: "group" }"#),
        "{stderr}"
    );
}
