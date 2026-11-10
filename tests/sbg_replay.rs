//! A recorded session replays to the same replies: real captured request frames go through a
//! recording SBG listener, and the recording replays against a fresh store without a single
//! differing reply. The recorded sessions are in `recorded_sessions.rs`.
//!
//! The replay resets process-wide SBG state, so it lives in its own test binary.

use std::{
    io::Write,
    net::{TcpListener, TcpStream},
    sync::Arc,
    time::{Duration, Instant},
};

use onevs100_gameserver::{
    app::App,
    ids::Xuid,
    matchmaking::Members,
    sbg::{self, record, replay},
    storage::{BlobKind, MemoryStorage, Storage},
    store::{SimulationOptions, Store},
    wire::{self, Protocol},
};

const CAPTURE_1317: &str = include_str!("../fixtures/sbg-1317-capture-r1-fresh.txt");
/// The profile of the capture's `user_get_user_storage` request.
const CAPTURED_XUID: Xuid = Xuid(16_140_909_857_506_937_655);

/// Request frames of the 2026-09-26 v2.0.1317.0 capture, in the order a title sends them: register,
/// the first GameState sync, setup reads, the user documents, an election and its counters, the
/// question bank, and TheMob written before it is read back.
const REQUESTS: &[&str] = &[
    "register",
    "store_get_game_state",
    "store_get_config",
    "user_get_user_storage",
    "store_set_user_error",
    "top_n_post_the_one",
    "sum_add_player_count",
    "top_n_get_the_one",
    "sum_get_player_count",
    "store_get_question_bank_reset_id",
    "store_set_the_mob_compressed",
    "store_get_the_mob",
    "heartbeat",
];

fn frame(label: &str) -> Vec<u8> {
    let hex = CAPTURE_1317
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| line.strip_prefix(label)?.strip_prefix(' '))
        .unwrap_or_else(|| panic!("capture frame {label} missing"));
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn assert_replays_cleanly(name: &str, recording: &record::Recording) {
    let mismatches = replay::replay(recording).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert!(
        mismatches.is_empty(),
        "{name}: {} replies differ:\n{mismatches:#?}",
        mismatches.len()
    );
}

#[test]
fn a_recorded_session_replays_to_the_same_replies() {
    let dir = std::env::temp_dir().join(format!("sbg-replay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("roundtrip.sbg");
    let settings = record::Settings {
        protocol: Protocol::V1317,
        options: SimulationOptions::default(),
        server_flow: false,
        schedule: false,
        served: onevs100_gameserver::xsor::ServedConfig::LIVE,
    };
    // The player's stored document must reach the replay through the recording's snapshot.
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::default());
    storage
        .put_blob(
            BlobKind::UserStorage,
            CAPTURED_XUID,
            b"<UserStorage>recorded</UserStorage>",
            onevs100_gameserver::time::Utc::from_unix_secs(1),
        )
        .unwrap();
    let snapshot = storage.export().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Arc::new(App::new(
        sbg::Settings::new(Protocol::V1317),
        Default::default(),
        Store::bootstrap().with_storage(storage),
        Members::default(),
    ));
    let header = sbg::capture::Header {
        settings,
        started: onevs100_gameserver::time::now(),
        from: app.capture.next_stamp(),
        service_host: "127.0.0.1".into(),
        storage: snapshot,
    };
    app.capture
        .start(&path, sbg::capture::ExistingFile::Replace, header)
        .unwrap();
    std::thread::spawn(move || sbg::serve(listener, app));

    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    for label in REQUESTS {
        stream.write_all(&frame(label)).unwrap();
        if *label != "heartbeat" {
            // One request at a time, so the recording's order is the order of the requests.
            wire::read_one(&mut stream).unwrap();
        }
    }
    drop(stream);

    // The server records `close` once it has seen the connection end.
    let deadline = Instant::now() + Duration::from_secs(5);
    let recording = loop {
        let recording = record::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        if recording.events.last().map(|e| e.kind) == Some(record::Kind::Close) {
            break recording;
        }
        assert!(Instant::now() < deadline, "connection close never recorded");
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = std::fs::remove_dir_all(&dir);

    let count = |kind| recording.events.iter().filter(|e| e.kind == kind).count();
    assert_eq!(count(record::Kind::In), REQUESTS.len());
    assert_eq!(
        count(record::Kind::Out),
        REQUESTS.len() - 1,
        "heartbeat gets no reply"
    );
    assert_replays_cleanly("round trip", &recording);

    // Without the storage snapshot the player's document is missing and the reply differs.
    let mut no_storage = recording.clone();
    no_storage.storage = None;
    let mismatches = replay::replay(&no_storage).unwrap();
    assert_eq!(mismatches.len(), 1, "{mismatches:#?}");
    assert!(
        mismatches[0]
            .expected
            .as_deref()
            .unwrap()
            .contains("recorded")
    );

    // The replay must also notice a difference: claim the Config GET was answered 404.
    let mut tampered = recording.clone();
    let config_reply = tampered
        .events
        .iter_mut()
        .filter(|e| e.kind == record::Kind::Out)
        .nth(2)
        .unwrap();
    let cookie = u16::from_be_bytes([config_reply.frame[6], config_reply.frame[7]]);
    config_reply.frame = wire::v1317::generic_reply(cookie, 404, false, b"");
    let mismatches = replay::replay(&tampered).unwrap();
    assert_eq!(mismatches.len(), 1, "{mismatches:#?}");
    assert_eq!(mismatches[0].cookie, Some(cookie));
    assert!(
        mismatches[0]
            .actual
            .as_deref()
            .unwrap()
            .contains("status=200")
    );
    assert!(
        mismatches[0]
            .expected
            .as_deref()
            .unwrap()
            .contains("status=404")
    );
}
