use std::sync::{Mutex, atomic::AtomicU64};

use crate::{app::App, store::Store, sync::lock, wire::Protocol};

pub mod capture;
pub mod host;
pub mod key;
mod message;
pub mod record;
pub mod replay;
pub mod sim;
pub(crate) mod user_error;
mod v1149_keys;

use host::LiveConnection;

mod conn;
mod dispatch;
mod handlers;

pub use conn::serve;
use conn::*;

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// Player document keys for tests.
#[cfg(test)]
const USER_STORAGE: &str = "v3.Season.MGS.1vs100.UserStore.UserStorage";
#[cfg(test)]
const USER_STATS: &str = "v3.Season.MGS.1vs100.UserStore.UserStats";
use message::Message;

/// SBG settings, fixed at startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Title build served; other-build frames end their connection.
    pub protocol: Protocol,
    /// Re-time posts onto the server's chain.
    pub server_flow: bool,
    /// Directory for posted GameState documents.
    pub dump_dir: Option<std::path::PathBuf>,
    /// Console-facing address (the bind IP), for URLs handed to the title.
    pub service_host: String,
}

impl Settings {
    /// Plain store on loopback.
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            server_flow: false,
            dump_dir: None,
            service_host: "127.0.0.1".into(),
        }
    }
}

/// Show-scoped state; a new show starts from scratch so chains never leak across shows.
fn reset_show_state(store: &Mutex<Store>) {
    lock(store).reset_session();
}

/// Volatile state; persistent data, programme, shows, and options stay.
pub fn reset_volatile(app: &App) {
    reset_show_state(&app.store);
    lock(&app.host).forget_roles();
    lock(&app.schedule).reset_active_session();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ids::{ConnId, Xuid},
        matchmaking::Members,
        schedule::Schedule,
        store::SimulationOptions,
        wire,
    };
    use flate2::read::ZlibDecoder;
    use host::HostAuthority;
    use message::zlib;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::Arc,
        time::{Duration, SystemTime},
    };

    /// The console the unit tests speak for.
    const TEST_CALLER: Caller = Caller {
        peer: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        conn: ConnId(0),
    };

    /// One frame through the request path, from [`TEST_CALLER`].
    fn handle(frame: wire::Frame, store: &Mutex<Store>) -> Option<Vec<u8>> {
        handle_scheduled(frame, store, &Mutex::default())
    }

    /// Store on a [`ManualClock`](crate::time::ManualClock); unwritten held GETs give up at once.
    fn on_manual_clock(store: Store) -> Mutex<Store> {
        let now = crate::time::Utc::from(SystemTime::now());
        Mutex::new(store.with_clock(Arc::new(crate::time::ManualClock::starting_at(now))))
    }

    /// [`handle`] with a schedule.
    fn handle_scheduled(
        frame: wire::Frame,
        store: &Mutex<Store>,
        schedule: &Mutex<Schedule>,
    ) -> Option<Vec<u8>> {
        let message = decode(&frame)?;
        let settings = Settings::new(frame.protocol());
        let host = Mutex::new(HostAuthority::default());
        let members = Mutex::new(Members::default());
        let ctx = Ctx {
            settings: &settings,
            store,
            host: &host,
            members: &members,
            schedule,
        };
        respond(TEST_CALLER, ctx, frame.cookie, message)
    }

    /// Recorded unregister of a second player: controller 1, 8 opaque bytes, xuid.
    const RECORDED_UNREGISTER: &str = "ff010000001900700103bac6fa5c71c6d0e0000754484d4b37";

    fn frame_from_hex(hex: &str) -> wire::Frame {
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let (frame, used) = wire::decode(&bytes).unwrap().unwrap();
        assert_eq!(used, bytes.len());
        frame
    }

    #[test]
    fn a_1149_unregister_is_answered_with_its_cookie_in_a_26_byte_0x83() {
        let frame = frame_from_hex(RECORDED_UNREGISTER);
        assert_eq!(
            decode(&frame),
            Some(Ok(Message::Unregister {
                controller: 1,
                xuid: Xuid(0xe000_0754_484d_4b37),
            }))
        );
        let reply = handle(frame, &Mutex::new(Store::bootstrap())).unwrap();
        let mut expected = vec![0x03, 0x83, 0, 0, 0, 0x1a, 0x00, 0x70, 0x00, 0xc8];
        expected.extend([0; 16]);
        assert_eq!(reply, expected);
    }

    #[test]
    fn a_1149_unregister_of_the_wrong_length_is_malformed() {
        // One byte short of the 17-byte body.
        let frame = frame_from_hex("ff010000001800700103bac6fa5c71c6d0e0000754484d4b");
        assert_eq!(decode(&frame), Some(Err(())));
    }

    const CONFIG: &str = "v3.Setup.MGS.1vs100.Store.Config";
    const WEEK1: &str = "v3.Setup.MGS.1vs100.Store.Config.Week1";
    const WEEK52: &str = "v3.Setup.MGS.1vs100.Store.Config.Week52";
    const MERGED: &str =
        "v3.Setup.MGS.1vs100.Store.Sessions.00000000-0000-0000-0000-000000000001.MergedLaunchData";
    const LANGUAGES: &str =
        "v3.Setup.MGS.1vs100.Store.00000000-0000-0000-0000-000000000000.AvailableLanguages";
    const GAME_STATE: &str = "v3.Session.MGS.1vs100.Store.P.GameState";
    const SUM: &str = "v3.Session.MGS.1vs100.Sum.AnswerCount";
    const TOP_N: &str =
        "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.591.TheOne";

    fn named_frame(
        cookie: u16,
        key: &str,
        tag: &str,
        compressed: bool,
        value: &[u8],
    ) -> wire::Frame {
        let body = wire::v1317::NamedRequest {
            key: key.as_bytes(),
            tag: tag.as_bytes(),
            compressed,
            value,
            execute_time_utc: 0,
        }
        .to_body()
        .unwrap();
        wire::Frame {
            version: 3,
            kind: wire::v1317::KIND_NAMED_REQUEST,
            cookie,
            body,
        }
    }

    fn get(cookie: u16, key: &str) -> wire::Frame {
        named_frame(cookie, key, "", false, b"")
    }

    fn set(cookie: u16, key: &str, value: &[u8]) -> wire::Frame {
        named_frame(cookie, key, "", false, value)
    }

    fn sum_post(cookie: u16, key: &str, delta: &str) -> wire::Frame {
        named_frame(cookie, key, &format!("value={delta}"), false, b"")
    }

    fn top_n_post(
        cookie: u16,
        key: &str,
        capacity: usize,
        score: &str,
        metadata: &[u8],
    ) -> wire::Frame {
        named_frame(
            cookie,
            key,
            &format!("capacity={capacity}&value={score}"),
            false,
            metadata,
        )
    }

    fn captured_top_n_post(cookie: u16) -> wire::Frame {
        const BODY: &[u8] = b"\x00\x01\xdd\x4f\x92\x03\xa1\x90\xc0\
            \x01\x01\xdd\x4f\x91\xe1\xce\x9f\x90\
            \x00\x54v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.591.TheOne\
            \x00\x19capacity=1&value=0.720000\
            \x00\x00\x00\x00\x29<Metadata>16140909857506937655</Metadata>";
        wire::Frame {
            version: 3,
            kind: 0x0a,
            cookie,
            body: BODY.to_vec(),
        }
    }

    fn user_get(cookie: u16, key: &str, xuid: &str) -> wire::Frame {
        named_frame(cookie, key, &format!("xuid={xuid}"), false, b"")
    }

    fn user_set(cookie: u16, key: &str, xuid: &str, value: &[u8]) -> wire::Frame {
        named_frame(cookie, key, &format!("xuid={xuid}"), false, value)
    }

    /// A 1317 generic reply as parsed back off the wire.
    #[derive(Debug, PartialEq, Eq)]
    struct GenericParts<'a> {
        kind: u8,
        cookie: u16,
        status: u16,
        compressed: bool,
        value: &'a [u8],
    }

    fn generic_parts(reply: &[u8]) -> GenericParts<'_> {
        let parsed = wire::v1317::GenericReply::parse(reply).unwrap();
        GenericParts {
            kind: wire::v1317::KIND_GENERIC_REPLY,
            cookie: parsed.cookie,
            status: parsed.status,
            compressed: parsed.compressed,
            value: parsed.value,
        }
    }

    fn hit_xml(reply: &[u8]) -> String {
        let parts = generic_parts(reply);
        assert_eq!(parts.kind, 0x82);
        assert_eq!(parts.status, 200);
        assert!(parts.compressed);
        let mut xml = String::new();
        ZlibDecoder::new(parts.value)
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    fn xml_tag<'a>(xml: &'a str, name: &str) -> &'a str {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        xml.split_once(&open)
            .unwrap()
            .1
            .split_once(&close)
            .unwrap()
            .0
    }

    #[test]
    fn merged_launch_data_get_uses_the_current_grid_window() {
        let store = Mutex::new(Store::bootstrap());
        let now = std::time::SystemTime::now();
        let later = now + Duration::from_secs(60);
        let current_grid = crate::grid::sessions_xml(
            &Default::default(),
            &mut Schedule::default(),
            "1vs100",
            "20260927",
            now,
        );
        let later_grid = crate::grid::sessions_xml(
            &Default::default(),
            &mut Schedule::default(),
            "1vs100",
            "20260927",
            later,
        );
        let merged = hit_xml(&handle(get(3, MERGED), &store).unwrap());

        let start = xml_tag(&merged, "StartTime").replace(".000Z", "Z");
        let earliest_start = xml_tag(&current_grid, "StartTimeUtc");
        let latest_start = xml_tag(&later_grid, "StartTimeUtc");
        assert!(
            (earliest_start..=latest_start).contains(&start.as_str()),
            "MergedLaunchData StartTime {start} is outside the current Grid window"
        );

        let end = xml_tag(&merged, "EndTime").replace(".000Z", "Z");
        let earliest_end = xml_tag(&current_grid, "EndTimeUtc");
        let latest_end = xml_tag(&later_grid, "EndTimeUtc");
        assert!(
            (earliest_end..=latest_end).contains(&end.as_str()),
            "MergedLaunchData EndTime {end} is outside the current Grid window"
        );
    }

    #[test]
    fn question_bank_fixtures_round_trip_through_compressed_gets() {
        const RESET: &str = "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-US.Difficulty.1.ResetID";
        const QUESTION: &str = "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-US.Difficulty.1.ID.251317";
        let store = on_manual_clock(Store::bootstrap());

        let reset = hit_xml(&handle(get(1, RESET), &store).unwrap());
        assert!(reset.contains("<XsorItem type=\"integer\">251317</XsorItem>"));
        assert!(reset.contains("<XsorItem type=\"integer\">251401</XsorItem>"));
        assert_eq!(
            hit_xml(&handle(get(2, QUESTION), &store).unwrap()).as_bytes(),
            include_bytes!("../../fixtures/question-251317-en-us.xml")
        );

        let reply = handle(get(3, &format!("{QUESTION}0")), &store).unwrap();
        assert_eq!(generic_parts(&reply).status, 404);
    }

    #[test]
    fn register_heartbeat_and_bare_gamestate_follow_wire_contract() {
        let store = Mutex::new(Store::bootstrap());
        let register = wire::Frame {
            version: 3,
            kind: 0,
            cookie: 0x65,
            body: vec![0; 17],
        };
        let reply = handle(register, &store).unwrap();
        assert_eq!(reply.len(), 26);
        assert_eq!(&reply[..10], &[3, 0x80, 0, 0, 0, 26, 0, 0x65, 0, 200]);
        assert_eq!(
            handle(
                wire::Frame {
                    version: 3,
                    kind: 2,
                    cookie: 7,
                    body: vec![]
                },
                &store
            ),
            None
        );

        let miss = handle(get(0x0b, GAME_STATE), &store).unwrap();
        let parts = generic_parts(&miss);
        assert_eq!(parts.kind, 0x82);
        assert_eq!(parts.cookie, 0x0b);
        assert_eq!(parts.status, 404);
        assert!(!parts.compressed);
        assert!(parts.value.is_empty());
    }

    #[test]
    fn captured_1149_config_get_and_heartbeat_are_answered() {
        // v2.0.1149.0 retail, fixtures/sbg-1149-capture-20261018.txt.
        let store = Mutex::new(Store::bootstrap());
        let frame = |label| {
            let raw = wire::fixture_frame(wire::FIXTURE_1149, label);
            wire::decode(&raw).unwrap().unwrap().0
        };
        let reply = handle(frame("get_config"), &store).unwrap();
        assert_eq!(&reply[1..2], &[wire::v1149::KIND_REPLY]);
        assert_eq!(&reply[6..10], &[0, 0x68, 0, 200]);
        let config = message::tests::inflate_v1149_value(&reply[26..]);
        let config = std::str::from_utf8(&config).unwrap();
        assert!(
            config.contains("<MinGameVersionNumber type=\"float\">2.02</MinGameVersionNumber>")
        );

        let pangolin = handle(frame("get_pangolin_config"), &store).unwrap();
        assert_eq!(&pangolin[6..10], &[0, 0x66, 0, 200]);
        assert_eq!(
            message::tests::inflate_v1149_value(&pangolin[26..]),
            0.0f64.to_be_bytes()
        );

        let ack = handle(frame("heartbeat"), &store).unwrap();
        assert_eq!(ack, wire::v1149::heartbeat_ack(0x63));
    }

    #[test]
    fn captured_console_config_get_is_not_rejected_as_malformed() {
        let mut body = vec![
            0x00, 0x01, 0xdd, 0x4f, 0x7b, 0x00, 0x05, 0xcc, 0x60, // control + timestamp
            0, 0, 0, 0, 0, 0, 0, 0, 0, // captured 9-byte zero prefix
        ];
        body.extend_from_slice(b"\x00\x20v3.Setup.MGS.1vs100.Store.Config");
        body.extend_from_slice(&[0; 7]); // empty tag and value suffix
        assert_eq!(body.len(), 59);

        let reply = handle(
            wire::Frame {
                version: 3,
                kind: 0x0a,
                cookie: 2,
                body,
            },
            &Mutex::new(Store::bootstrap()),
        )
        .unwrap();

        let config = hit_xml(&reply);
        assert!(
            config.contains("<MinGameVersionNumber type=\"float\">2.02</MinGameVersionNumber>")
        );
    }

    #[test]
    fn bootstrap_fixtures_are_exact_keyed_compressed_typed_xsor() {
        let store = Mutex::new(Store::bootstrap());
        let config = hit_xml(&handle(get(1, CONFIG), &store).unwrap());
        assert!(config.contains("name=\"Config\""));
        assert!(
            config.contains("<MinGameVersionNumber type=\"float\">2.02</MinGameVersionNumber>")
        );
        assert!(config.contains(
            "<PTQuestionProgression type=\"table\"><XsorItem type=\"integer\">1</XsorItem>"
        ));
        assert!(!config.contains("xsor:item"));

        for key in [WEEK1, WEEK52] {
            let xml = hit_xml(&handle(get(2, key), &store).unwrap());
            assert!(xml.contains("name=\"PrizingMatrix\""));
            assert!(xml.contains("<PTOneT10 type=\"integer\">10000</PTOneT10>"));
            assert!(xml.contains("<PTTop1>SomeGame</PTTop1>"));
        }

        let merged = hit_xml(&handle(get(3, MERGED), &store).unwrap());
        assert!(merged.contains("name=\"MergedLaunchData\""));
        assert!(merged.contains("<LiveStreamPathURI type=\"table\"><en-us>http://127.0.0.1/livehost.wma</en-us></LiveStreamPathURI>"));
        assert!(merged.contains("<Duration>600</Duration>"));

        let languages = hit_xml(&handle(get(4, LANGUAGES), &store).unwrap());
        assert!(languages.contains("name=\"AvailableLanguages\""));
        assert!(
            languages.contains("<Languages type=\"table\"><XsorItem>en-US</XsorItem></Languages>")
        );
        assert!(!languages.contains("xsor:item"));
    }

    #[test]
    fn question_bank_serves_the_observed_question_and_reset_ids() {
        let store = Mutex::new(Store::bootstrap());
        let question_key = "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-us.Difficulty.1.ID.251317";
        let question = hit_xml(&handle(get(100, question_key), &store).unwrap());
        assert!(question.contains("<Question>Which company makes and sells the iPod?</Question>"));
        assert!(question.contains("<CorrectAnswer type=\"integer\">3</CorrectAnswer>"));
        assert!(question.contains("<ChoiceList type=\"table\"><XsorItem>Dell</XsorItem><XsorItem>Nokia</XsorItem><XsorItem>Apple</XsorItem></ChoiceList>"));

        let reset_key = "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-us.Difficulty.1.ResetID";
        let reset_ids = hit_xml(&handle(get(101, reset_key), &store).unwrap());
        assert!(reset_ids.contains("<XsorItem type=\"integer\">251317</XsorItem>"));
        for difficulty in 2..=5 {
            let reset_key = format!(
                "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-us.Difficulty.{difficulty}.ResetID"
            );
            let reply = handle(get(104, &reset_key), &store).unwrap();
            assert_eq!(generic_parts(&reply).status, 200);
            let ids = hit_xml(&reply);
            assert!(ids.contains("name=\"ResetID\""));
            assert_eq!(
                ids.matches("<XsorItem").count(),
                bank_size(difficulty),
                "{ids}"
            );
        }
    }

    fn bank_size(difficulty: u32) -> usize {
        let difficulty = crate::questions::Difficulty::from_number(difficulty)
            .expect("2 to 5 names a difficulty");
        crate::questions::QuestionBank::seed()
            .ids(difficulty, None)
            .len()
    }

    #[test]
    fn session_question_bank_serves_the_served_ids_and_empty_used_list() {
        let store = Mutex::new(Store::bootstrap());
        let prefix = "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.QuestionBank.Difficulty.1";
        let available = hit_xml(&handle(get(102, &format!("{prefix}.Available")), &store).unwrap());
        assert_eq!(
            available,
            "<XsorObject><XsorItem type=\"float\">251402</XsorItem><XsorItem type=\"float\">251401</XsorItem><XsorItem type=\"float\">251317</XsorItem></XsorObject>",
            "the session's order"
        );

        let used = hit_xml(&handle(get(103, &format!("{prefix}.Used")), &store).unwrap());
        assert_eq!(used, "<XsorObject />");

        for difficulty in 2..=5 {
            let prefix = format!(
                "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.QuestionBank.Difficulty.{difficulty}"
            );
            let reply = handle(get(104, &format!("{prefix}.Available")), &store).unwrap();
            assert_eq!(generic_parts(&reply).status, 200);
            assert_eq!(
                hit_xml(&reply).matches("<XsorItem").count(),
                bank_size(difficulty)
            );
            let reply = handle(get(105, &format!("{prefix}.Used")), &store).unwrap();
            assert_eq!(generic_parts(&reply).status, 200);
            assert_eq!(hit_xml(&reply), "<XsorObject />");
        }
    }

    #[test]
    fn session_themob_get_without_prior_set_returns_not_found() {
        // With the simulated mob on, TheMob still comes only from the title's SET.
        let store = on_manual_clock(Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        }));
        let key =
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.387.TheMob";

        let reply = handle(get(105, key), &store).unwrap();
        assert_eq!(generic_parts(&reply).status, 404);
    }
    #[test]
    fn question_bank_does_not_invent_unknown_question_ids() {
        let store = on_manual_clock(Store::bootstrap());
        let key = "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-us.Difficulty.1.ID.999999";
        assert_eq!(
            generic_parts(&handle(get(102, key), &store).unwrap()).status,
            404
        );
    }

    #[test]
    fn question_bank_rejects_compressed_gets_without_a_payload() {
        let store = Mutex::new(Store::bootstrap());
        let key = "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-us.Difficulty.1.ID.251317";
        let request = named_frame(106, key, "", true, b"");
        assert_eq!(generic_parts(&handle(request, &store).unwrap()).status, 404);
    }

    #[test]
    fn sum_posts_accumulate_and_get_returns_compressed_xsor() {
        #[derive(serde::Deserialize)]
        #[serde(rename = "xsor:object")]
        struct SumReply {
            #[serde(rename = "@name")]
            name: String,
            #[serde(rename = "@type")]
            value_type: String,
            #[serde(rename = "Value")]
            value: f64,
        }

        let store = Mutex::new(Store::default());
        let missing = handle(get(30, SUM), &store).unwrap();
        assert_eq!(
            generic_parts(&missing),
            GenericParts {
                kind: 0x82,
                cookie: 30,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );

        for (cookie, delta) in [(31, "1.000000"), (32, "-0.250000")] {
            let ack = handle(sum_post(cookie, SUM, delta), &store).unwrap();
            assert_eq!(
                generic_parts(&ack),
                GenericParts {
                    kind: 0x82,
                    cookie,
                    status: 200,
                    compressed: false,
                    value: &b""[..]
                }
            );
        }

        let xml = hit_xml(&handle(get(33, SUM), &store).unwrap());
        assert!(xml.contains("<Value>0.75</Value>"));
        assert!(!xml.contains("<Value type="));
        let reply: SumReply = quick_xml::de::from_str(&xml).unwrap();
        assert_eq!(reply.name, "Sum");
        assert_eq!(reply.value_type, "table");
        assert_eq!(reply.value, 0.75);
    }

    #[test]
    fn sum_rejects_malformed_forms_without_mutating_total() {
        let store = on_manual_clock(Store::default());
        let ack = handle(sum_post(34, SUM, "1"), &store).unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 34,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );

        let malformed = [
            named_frame(35, SUM, "delta=1", false, b""),
            named_frame(36, SUM, "value=", false, b""),
            named_frame(37, SUM, "value=1", true, b""),
            named_frame(38, SUM, "value=1", false, b"payload"),
            named_frame(39, SUM, "", false, b"payload"),
            named_frame(40, "v3.Session.MGS.1vs100.Sum", "value=1", false, b""),
        ];
        for request in malformed {
            let reply = handle(request, &store).unwrap();
            assert_eq!(generic_parts(&reply).status, 404);
        }

        let xml = hit_xml(&handle(get(41, SUM), &store).unwrap());
        assert!(xml.contains("<Value>1</Value>"));
    }

    #[test]
    fn sum_rejects_non_finite_and_overflow_without_mutating_total() {
        let store = Mutex::new(Store::default());
        let ack = handle(sum_post(42, SUM, "1"), &store).unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 42,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );

        for (cookie, delta) in [(43, "NaN"), (44, "inf"), (45, "-inf")] {
            let rejected = handle(sum_post(cookie, SUM, delta), &store).unwrap();
            assert_eq!(
                generic_parts(&rejected),
                GenericParts {
                    kind: 0x82,
                    cookie,
                    status: 404,
                    compressed: false,
                    value: &b""[..]
                }
            );
        }
        let xml = hit_xml(&handle(get(46, SUM), &store).unwrap());
        assert!(xml.contains("<Value>1</Value>"));

        let overflow_key = "v3.Session.MGS.1vs100.Sum.Overflow";
        let max = "1.7976931348623157e308";
        let ack = handle(sum_post(47, overflow_key, max), &store).unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 47,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );
        let rejected = handle(sum_post(48, overflow_key, max), &store).unwrap();
        assert_eq!(
            generic_parts(&rejected),
            GenericParts {
                kind: 0x82,
                cookie: 48,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );
        let xml = hit_xml(&handle(get(49, overflow_key), &store).unwrap());
        let value = xml_tag(&xml, "Value").parse::<f64>().unwrap();
        assert_eq!(value, f64::MAX);
    }

    #[test]
    fn captured_top_n_post_acknowledges_and_get_returns_compressed_xsor() {
        let store = Mutex::new(Store::default());
        let missing = handle(get(50, TOP_N), &store).unwrap();
        assert_eq!(
            generic_parts(&missing),
            GenericParts {
                kind: 0x82,
                cookie: 50,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );

        let ack = handle(captured_top_n_post(51), &store).unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 51,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );

        let xml = hit_xml(&handle(get(52, TOP_N), &store).unwrap());
        assert!(xml.contains("<Values type=\"table\">"));
        assert!(xml.contains("<XsorItem type=\"table\">"));
        assert!(xml.contains("<Value>0.72</Value>"));
        assert!(xml.contains("<Metadata><Metadata>16140909857506937655</Metadata></Metadata>"));
    }

    #[test]
    fn season_top_n_posts_are_ranked_lists_not_raw_blobs() {
        // Posts store ranked lists, not raw bodies; raw echo crashed the title.
        const KEY: &str = "v3.Season.MGS.1vs100.TopN.Prizing.Session.00000000-0000-0000-0000-000000000001.Round.0.RoundEndTopScores";
        let store = Mutex::new(Store::default());
        let missing = handle(get(1, KEY), &store).unwrap();
        assert_eq!(
            generic_parts(&missing),
            GenericParts {
                kind: 0x82,
                cookie: 1,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );
        let ack = handle(
            top_n_post(
                2,
                KEY,
                3,
                "0.000000",
                b"<Metadata>16140909857506937655</Metadata>",
            ),
            &store,
        )
        .unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 2,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );
        let xml = hit_xml(&handle(get(3, KEY), &store).unwrap());
        assert!(xml.contains("<Values type=\"table\">"), "{xml}");
        assert!(xml.contains("<Metadata><Metadata>16140909857506937655</Metadata></Metadata>"));
    }

    #[test]
    fn synthetic_mob_population_roundtrips_through_sbg() {
        const MOB_KEY: &str = "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.693.MobSelection";
        const PLAYER_COUNT_KEY: &str = "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.693.RSTotalPlayerCount";
        const REAL_METADATA: &[u8] = b"<Metadata>e00007ff484d4b37</Metadata>";

        let synthetic = Mutex::new(Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        }));
        assert_eq!(
            generic_parts(
                &handle(
                    top_n_post(120, MOB_KEY, 101, "0.6743096", REAL_METADATA),
                    &synthetic
                )
                .unwrap()
            )
            .status,
            200
        );
        assert_eq!(
            generic_parts(&handle(sum_post(121, PLAYER_COUNT_KEY, "1.0"), &synthetic).unwrap())
                .status,
            200
        );

        let mob_xml = hit_xml(&handle(get(122, MOB_KEY), &synthetic).unwrap());
        assert_eq!(mob_xml.matches("<XsorItem type=\"table\">").count(), 101);
        let real_item = "<Metadata><Metadata>e00007ff484d4b37</Metadata></Metadata>";
        let synthetic_item = "<Metadata><Metadata>f000000000000001</Metadata></Metadata>";
        assert!(mob_xml.contains(real_item));
        assert!(mob_xml.contains(synthetic_item));
        assert!(mob_xml.find(real_item).unwrap() < mob_xml.find(synthetic_item).unwrap());
        assert!(
            hit_xml(&handle(get(123, PLAYER_COUNT_KEY), &synthetic).unwrap())
                .contains("<Value>102</Value>")
        );

        let ordinary = Mutex::new(Store::bootstrap());
        assert_eq!(
            generic_parts(
                &handle(
                    top_n_post(124, MOB_KEY, 101, "0.6743096", REAL_METADATA),
                    &ordinary
                )
                .unwrap()
            )
            .status,
            200
        );
        assert_eq!(
            generic_parts(&handle(sum_post(125, PLAYER_COUNT_KEY, "1.0"), &ordinary).unwrap())
                .status,
            200
        );
        let ordinary_mob = hit_xml(&handle(get(126, MOB_KEY), &ordinary).unwrap());
        assert_eq!(ordinary_mob.matches("<XsorItem type=\"table\">").count(), 1);
        assert!(
            hit_xml(&handle(get(127, PLAYER_COUNT_KEY), &ordinary).unwrap())
                .contains("<Value>1</Value>")
        );
    }

    #[test]
    fn top_n_rejects_invalid_posts_and_capacity_mismatch_without_mutation() {
        let store = Mutex::new(Store::default());
        let invalid = [
            named_frame(
                53,
                TOP_N,
                "capacity=0&value=1",
                false,
                b"<Metadata>zero</Metadata>",
            ),
            named_frame(
                54,
                TOP_N,
                "capacity=102&value=1",
                false,
                b"<Metadata>large</Metadata>",
            ),
            named_frame(
                55,
                TOP_N,
                "capacity=1&value=NaN",
                false,
                b"<Metadata>nan</Metadata>",
            ),
            named_frame(
                56,
                TOP_N,
                "capacity=1&value=1",
                false,
                b"<Metadata>unclosed",
            ),
            named_frame(
                57,
                TOP_N,
                "value=1&capacity=1",
                false,
                b"<Metadata>order</Metadata>",
            ),
            named_frame(
                58,
                TOP_N,
                "capacity=1&value=1&extra=1",
                false,
                b"<Metadata>extra</Metadata>",
            ),
            named_frame(
                59,
                TOP_N,
                "capacity=1&value=1",
                true,
                b"<Metadata>compressed</Metadata>",
            ),
            named_frame(60, TOP_N, "", false, b"<Metadata>get-payload</Metadata>"),
        ];
        for request in invalid {
            let reply = handle(request, &store).unwrap();
            assert_eq!(generic_parts(&reply).status, 404);
        }
        assert_eq!(
            generic_parts(&handle(get(61, TOP_N), &store).unwrap()).status,
            404
        );

        let ack = handle(
            top_n_post(62, TOP_N, 2, "1", b"<Metadata>accepted</Metadata>"),
            &store,
        )
        .unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 62,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );
        let mismatch = handle(
            top_n_post(63, TOP_N, 1, "2", b"<Metadata>rejected</Metadata>"),
            &store,
        )
        .unwrap();
        assert_eq!(
            generic_parts(&mismatch),
            GenericParts {
                kind: 0x82,
                cookie: 63,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );
        let xml = hit_xml(&handle(get(64, TOP_N), &store).unwrap());
        assert!(xml.contains("<Metadata><Metadata>accepted</Metadata></Metadata>"));
        assert!(!xml.contains("rejected"));
    }

    #[test]
    fn oversized_top_n_get_returns_bounded_500() {
        let store = Mutex::new(Store::default());
        let key = "v3.Session.MGS.1vs100.TopN.Oversized";
        let mut state = 0x1234_5678_9abc_def0u64;
        for index in 0..101 {
            let text = (0..700)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    b'a' + (state % 26) as u8
                })
                .collect::<Vec<_>>();
            let mut metadata = b"<Metadata>".to_vec();
            metadata.extend_from_slice(&text);
            metadata.extend_from_slice(b"</Metadata>");
            let ack = handle(
                top_n_post(65, key, 101, &index.to_string(), &metadata),
                &store,
            )
            .unwrap();
            assert_eq!(generic_parts(&ack).status, 200);
        }

        let reply = handle(get(66, key), &store).unwrap();
        assert!(reply.len() <= wire::MAX_FRAME);
        assert_eq!(
            generic_parts(&reply),
            GenericParts {
                kind: 0x82,
                cookie: 66,
                status: 500,
                compressed: false,
                value: &b""[..]
            }
        );
    }

    #[test]
    fn every_console_error_report_is_saved_even_under_one_name() {
        let store = Mutex::new(Store::bootstrap());
        let key = "v3.Session.MGS.1vs100.Log.UserError.LobbyFPP";
        let first = b"<xsor:object>errorName: LobbyFPP, errorReason: first</xsor:object>";
        let second = b"<xsor:object>errorName: LobbyFPP, errorReason: second</xsor:object>";
        handle(set(1, key, first), &store).unwrap();
        handle(set(2, key, second), &store).unwrap();
        let saved = lock(&store).storage().error_reports(10).unwrap();
        let documents: Vec<&[u8]> = saved
            .iter()
            .map(|saved| saved.report.document.as_slice())
            .collect();
        assert_eq!(documents, [&second[..], &first[..]]);
        assert!(
            saved
                .iter()
                .all(|saved| saved.report.key == key && saved.report.console == TEST_CALLER.peer)
        );
    }

    #[test]
    fn set_then_get_round_trips_generic_values_with_compressed_gets() {
        let store = Mutex::new(Store::bootstrap());
        let key = "v3.Session.MGS.1vs100.Store.P.Custom";
        let value = b"<xsor:object xmlns:xsor=\"http://schemas.xbox.com/xsor\" xmlns=\"http://schemas.xbox.com/SBG\" name=\"Custom\" type=\"table\"><Value>ok</Value></xsor:object>";
        let ack = handle(set(8, key, value), &store).unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 8,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );
        assert_eq!(
            hit_xml(&handle(get(9, key), &store).unwrap()).as_bytes(),
            value
        );

        let compressed_key = "v3.Session.MGS.1vs100.Store.P.CompressedCustom";
        let compressed = zlib(value).unwrap();
        let ack = handle(
            named_frame(10, compressed_key, "", true, &compressed),
            &store,
        )
        .unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 10,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );
        assert_eq!(
            hit_xml(&handle(get(11, compressed_key), &store).unwrap()).as_bytes(),
            value
        );
    }

    #[test]
    fn user_storage_is_raw_per_xuid_and_user_stats_missing_is_not_fabricated() {
        let store = Mutex::new(Store::bootstrap());
        let default = handle(user_get(12, USER_STORAGE, "16140901064495857664"), &store).unwrap();
        let parts = generic_parts(&default);
        assert_eq!(parts.kind, 0x82);
        assert_eq!(parts.cookie, 12);
        assert_eq!(parts.status, 200);
        assert!(!parts.compressed);
        let default_xml = std::str::from_utf8(parts.value).unwrap();
        assert!(default_xml.contains("name=\"UserStorage\""));
        assert!(default_xml.contains("<LQ type=\"integer\">0</LQ>"));
        assert!(default_xml.contains("<PS type=\"table\"><bValid type=\"boolean\">false</bValid>"));

        let stats = handle(user_get(13, USER_STATS, "16140901064495857664"), &store).unwrap();
        assert_eq!(
            generic_parts(&stats),
            GenericParts {
                kind: 0x82,
                cookie: 13,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );

        let custom = b"<xsor:object xmlns:xsor=\"http://schemas.xbox.com/xsor\" xmlns=\"http://schemas.xbox.com/SBG\" name=\"UserStorage\" type=\"table\"><LQ type=\"integer\">7</LQ></xsor:object>";
        let ack = handle(
            user_set(14, USER_STORAGE, "16140901064495857664", custom),
            &store,
        )
        .unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 14,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );
        let got = handle(user_get(15, USER_STORAGE, "16140901064495857664"), &store).unwrap();
        assert_eq!(
            generic_parts(&got),
            GenericParts {
                kind: 0x82,
                cookie: 15,
                status: 200,
                compressed: false,
                value: &custom[..]
            }
        );
    }

    #[test]
    fn user_stats_set_then_get_round_trips_raw_without_default_fixture() {
        let store = Mutex::new(Store::bootstrap());
        let missing = handle(user_get(18, USER_STATS, "16140901064495857664"), &store).unwrap();
        assert_eq!(
            generic_parts(&missing),
            GenericParts {
                kind: 0x82,
                cookie: 18,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );

        let stats = b"<xsor:object xmlns:xsor=\"http://schemas.xbox.com/xsor\" xmlns=\"http://schemas.xbox.com/SBG\" name=\"UserStats\" type=\"table\"><Games type=\"integer\">1</Games></xsor:object>";
        let ack = handle(
            user_set(19, USER_STATS, "16140901064495857664", stats),
            &store,
        )
        .unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 19,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );
        let got = handle(user_get(20, USER_STATS, "16140901064495857664"), &store).unwrap();
        assert_eq!(
            generic_parts(&got),
            GenericParts {
                kind: 0x82,
                cookie: 20,
                status: 200,
                compressed: false,
                value: &stats[..]
            }
        );
    }

    const XUID_DEC: &str = "16140901064495857664";

    #[test]
    fn user_documents_live_in_storage_and_outlive_the_store() {
        use crate::storage::{BlobKind, MemoryStorage, Storage};
        let storage: std::sync::Arc<dyn Storage> = std::sync::Arc::new(MemoryStorage::default());
        let custom = b"<xsor:object name=\"UserStorage\" type=\"table\"><LQ type=\"integer\">7</LQ></xsor:object>";
        let first = Mutex::new(Store::bootstrap().with_storage(std::sync::Arc::clone(&storage)));
        handle(user_set(30, USER_STORAGE, XUID_DEC, custom), &first).unwrap();
        handle(user_set(31, USER_STATS, XUID_DEC, b"<s/>"), &first).unwrap();

        // A restarted server: a fresh store over the same storage serves the same documents.
        let second = Mutex::new(Store::bootstrap().with_storage(std::sync::Arc::clone(&storage)));
        let got = handle(user_get(32, USER_STORAGE, XUID_DEC), &second).unwrap();
        assert_eq!(
            generic_parts(&got),
            GenericParts {
                kind: 0x82,
                cookie: 32,
                status: 200,
                compressed: false,
                value: &custom[..]
            }
        );
        let got = handle(user_get(33, USER_STATS, XUID_DEC), &second).unwrap();
        assert_eq!(
            generic_parts(&got),
            GenericParts {
                kind: 0x82,
                cookie: 33,
                status: 200,
                compressed: false,
                value: &b"<s/>"[..]
            }
        );
        let numeric = crate::ids::Xuid(XUID_DEC.parse().unwrap());
        assert_eq!(
            storage
                .blob(BlobKind::UserStorage, numeric)
                .unwrap()
                .unwrap(),
            custom
        );
        // Unknown players still get the default document / no stats, and a bad tag is rejected.
        let other = handle(user_get(34, USER_STORAGE, "1"), &second).unwrap();
        assert!(generic_parts(&other).value.starts_with(b"<xsor:object"));
        let bad = handle(user_get(35, USER_STORAGE, "not-a-number"), &second).unwrap();
        assert_eq!(generic_parts(&bad).status, 404);
    }

    #[test]
    fn global_stats_sums_accumulate_persist_and_never_block() {
        use crate::storage::{MemoryStorage, Storage};
        let storage: std::sync::Arc<dyn Storage> = std::sync::Arc::new(MemoryStorage::default());
        let key = "v3.Stats.MGS.1vs100.Sum.StatsUpdate.TotalSeen.251317";
        let first = Mutex::new(Store::bootstrap().with_storage(std::sync::Arc::clone(&storage)));

        let started = std::time::Instant::now();
        let missing = handle(named_frame(40, key, "", false, b""), &first).unwrap();
        assert_eq!(generic_parts(&missing).status, 404);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "an unknown stat must not park the GET"
        );

        for cookie in [41, 42] {
            let ack = handle(
                named_frame(cookie, key, "value=1.000000", false, b""),
                &first,
            )
            .unwrap();
            assert_eq!(
                generic_parts(&ack),
                GenericParts {
                    kind: 0x82,
                    cookie,
                    status: 200,
                    compressed: false,
                    value: &b""[..]
                }
            );
        }
        assert!(
            hit_xml(&handle(named_frame(43, key, "", false, b""), &first).unwrap())
                .contains("<Value>2</Value>")
        );

        let second = Mutex::new(Store::bootstrap().with_storage(storage));
        assert!(
            hit_xml(&handle(named_frame(44, key, "", false, b""), &second).unwrap())
                .contains("<Value>2</Value>")
        );
        let bad = handle(named_frame(45, key, "value=abc", false, b""), &second).unwrap();
        assert_eq!(generic_parts(&bad).status, 404);
    }

    #[test]
    fn malformed_generic_body_gets_bounded_error_and_connection_continues() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            serve(
                listener,
                Arc::new(App::new(
                    Settings::new(Protocol::V1317),
                    Default::default(),
                    Store::bootstrap(),
                    Default::default(),
                )),
            )
            .unwrap()
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut malformed = wire::encode(Protocol::V1317, 0x0a, 1, &[1, 2, 3]);
        malformed.extend_from_slice(&wire::encode(Protocol::V1317, 0, 2, &[0; 17]));
        stream.write_all(&malformed).unwrap();

        let mut first = [0; 39];
        stream.read_exact(&mut first).unwrap();
        assert_eq!(
            generic_parts(&first),
            GenericParts {
                kind: 0x82,
                cookie: 1,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );
        let mut second = [0; 26];
        stream.read_exact(&mut second).unwrap();
        assert_eq!(&second[..10], &[3, 0x80, 0, 0, 0, 26, 0, 2, 0, 200]);
    }

    fn serve_on_loopback(protocol: Protocol) -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            serve(
                listener,
                Arc::new(App::new(
                    Settings::new(protocol),
                    Default::default(),
                    Store::bootstrap(),
                    Default::default(),
                )),
            )
            .unwrap()
        });
        let stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
    }

    #[test]
    fn a_1149_server_answers_1149_and_drops_a_1317_frame() {
        let mut stream = serve_on_loopback(Protocol::V1149);
        stream
            .write_all(&wire::fixture_frame(wire::FIXTURE_1149, "register"))
            .unwrap();
        let mut register = [0; 26];
        stream.read_exact(&mut register).unwrap();
        assert_eq!(
            register.to_vec(),
            wire::fixture_frame(wire::FIXTURE_1149, "reply_register")
        );

        let v1317 = get(5, CONFIG);
        stream
            .write_all(&wire::encode(Protocol::V1317, v1317.kind, 5, &v1317.body))
            .unwrap();
        let mut rest = Vec::new();
        assert_eq!(stream.read_to_end(&mut rest).unwrap(), 0);
    }

    #[test]
    fn a_1317_server_drops_a_1149_register_unanswered() {
        let mut stream = serve_on_loopback(Protocol::V1317);
        stream
            .write_all(&wire::fixture_frame(wire::FIXTURE_1149, "register"))
            .unwrap();
        let mut rest = Vec::new();
        assert_eq!(stream.read_to_end(&mut rest).unwrap(), 0);
    }

    #[test]
    fn http_on_the_1149_sbg_port_is_answered_as_ads() {
        // Both builds point the ad client at TCP 1000.
        let mut stream = serve_on_loopback(Protocol::V1149);
        let mut request = b"POST /adsrv/5/locateService HTTP/1.1\r\nHost:\r\nContent-Type: application/massive\r\nContent-Length: 68\r\n\r\n".to_vec();
        request.extend_from_slice(&[0; 68]);
        stream.write_all(&request).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert!(
            response.starts_with(b"HTTP/1.1 200"),
            "{:?}",
            String::from_utf8_lossy(&response)
        );
    }

    /// A v2.0.1149.0 client request frame: controller 0, 8 zero bytes, execute time, key, `tail`.
    fn v1149_frame(
        kind: u8,
        cookie: u16,
        execute_at: SystemTime,
        key: &str,
        tail: &[u8],
    ) -> Vec<u8> {
        let filetime = (execute_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            / 100) as u64
            + 116_444_736_000_000_000;
        let mut body = vec![0; 9];
        body.extend_from_slice(&filetime.to_be_bytes());
        body.extend_from_slice(&(key.len() as u16).to_be_bytes());
        body.extend_from_slice(key.as_bytes());
        body.extend_from_slice(tail);
        let mut frame = wire::encode(Protocol::V1149, kind, cookie, &body);
        frame[0] = 0xFF; // the 1149 client's version byte (`Protocol::from_version`)
        frame
    }

    #[test]
    fn a_get_sent_before_a_sum_but_scheduled_after_it_sees_the_sum() {
        // `Server_LobbyPlayers` (captured 2026-10-19): the bucket GET can reach the server first.
        let mut stream = serve_on_loopback(Protocol::V1149);
        let key = "v1.Session.MGS.1vs100.655a39c6-1a68-5db7-b04f-ad76f3b20018.EXTENDED_PLAY.LobbyPlayerCount.105";
        let now = SystemTime::now();
        stream
            .write_all(&v1149_frame(
                3,
                7,
                now + Duration::from_millis(800),
                key,
                &[],
            ))
            .unwrap();
        stream
            .write_all(&v1149_frame(5, 8, now, key, &1.0f64.to_be_bytes()))
            .unwrap();
        let mut replies = Vec::new();
        while replies.len() < 2 {
            let frame = wire::read_one(&mut stream).unwrap();
            replies.push(frame);
        }
        let get = replies.iter().find(|f| f.cookie == 7).expect("get reply");
        let value = crate::sbg::message::tests::inflate_v1149_value(&get.body[18..]);
        assert!(
            String::from_utf8_lossy(&value).contains("<Value>1</Value>"),
            "{}",
            String::from_utf8_lossy(&value)
        );
    }

    #[test]
    fn season_sums_count_like_session_sums() {
        // EP `Question.<q>.TotalPlayerCount` and StatBreaks live under v3.Season...Sum (both builds).
        let mut stream = serve_on_loopback(Protocol::V1149);
        let key = "v1.Session.MGS.1vs100.655a39c6-1a68-5db7-b04f-ad76f3b20018.EXTENDED_PLAY.Question.0.TotalPlayerCount";
        let now = SystemTime::now();
        stream
            .write_all(&v1149_frame(5, 1, now, key, &1.0f64.to_be_bytes()))
            .unwrap();
        stream
            .write_all(&v1149_frame(
                3,
                2,
                now + Duration::from_millis(300),
                key,
                &[],
            ))
            .unwrap();
        let replies = [
            wire::read_one(&mut stream).unwrap(),
            wire::read_one(&mut stream).unwrap(),
        ];
        let get = replies.iter().find(|f| f.cookie == 2).expect("get reply");
        assert_eq!(u16::from_be_bytes([get.body[0], get.body[1]]), 200);
        let value = crate::sbg::message::tests::inflate_v1149_value(&get.body[18..]);
        assert!(String::from_utf8_lossy(&value).contains("<Value>1</Value>"));
    }

    #[test]
    fn requests_with_an_execute_time_are_answered_at_once_except_a_missing_counter() {
        // The 1149 client queues behind outstanding replies; held requests stall its setup.
        let mut stream = serve_on_loopback(Protocol::V1149);
        let later = SystemTime::now() + Duration::from_secs(5);
        let config = "v1.Setup.MGS.1vs100.Config";
        let sent = std::time::Instant::now();
        stream
            .write_all(&v1149_frame(3, 4, later, config, &[]))
            .unwrap();
        let reply = wire::read_one(&mut stream).unwrap();
        assert_eq!(u16::from_be_bytes([reply.body[0], reply.body[1]]), 200);
        assert!(
            sent.elapsed() < Duration::from_secs(2),
            "{:?}",
            sent.elapsed()
        );
    }

    #[test]
    fn idle_connection_is_closed_after_the_read_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            serve_with_read_timeout(
                listener,
                Arc::new(App::new(
                    Settings::new(Protocol::V1317),
                    Default::default(),
                    Store::bootstrap(),
                    Default::default(),
                )),
                Duration::from_millis(100),
            )
            .unwrap()
        });

        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0; 1];
        assert_eq!(
            stream.read(&mut byte).unwrap(),
            0,
            "server should close idle peer"
        );
    }

    #[test]
    fn question_bank_rejects_malformed_paths_but_accepts_default_qms_keys() {
        let store = on_manual_clock(Store::bootstrap());
        let malformed =
            "v3.Setup.MGS.1vs100.Store.QuestionBank.not-a-locale.Difficulty.1.ID.251317";
        assert_eq!(
            generic_parts(&handle(get(107, malformed), &store).unwrap()).status,
            404
        );

        let default_qms = "v3.Setup.MGS.1vs100.Store.QuestionBank.Difficulty.1.ID.251317";
        assert_eq!(
            generic_parts(&handle(get(108, default_qms), &store).unwrap()).status,
            200
        );
    }

    #[test]
    fn a_question_list_without_a_show_gets_no_bank_list() {
        let store = on_manual_clock(Store::bootstrap());
        let short = "v3.Setup.MGS.1vs100.Store.QuestionBank.Difficulty.1.ResetID";
        let reply = handle(get(109, short), &store).unwrap();
        assert_eq!(generic_parts(&reply).status, 404);
    }

    #[test]
    fn stored_introductions_are_served_as_stored() {
        // The One posts every state.
        let store = Mutex::new(Store::bootstrap());
        let game_state_key = "v3.Session.MGS.1vs100.Store.P.GameState";
        let intro =
            String::from_utf8(crate::gamestate::recorded("Introductions", false).to_xml()).unwrap();
        assert_eq!(
            generic_parts(&handle(set(109, game_state_key, intro.as_bytes()), &store).unwrap())
                .status,
            200
        );
        assert_eq!(
            hit_xml(&handle(get(110, game_state_key), &store).unwrap()),
            intro
        );
    }

    #[test]
    fn stored_qa_state_can_fetch_its_known_question_over_sbg() {
        let store = Mutex::new(Store::bootstrap());
        let game_state_key = "v3.Session.MGS.1vs100.Store.P.GameState";
        let qa_state = b"<XsorObject><name>QA</name><data><RoundNumber type=\"float\">1</RoundNumber><QuestionNumber type=\"float\">1</QuestionNumber><QuestionInfo type=\"table\"><questionID type=\"integer\">251317</questionID><difficulty type=\"integer\">1</difficulty></QuestionInfo></data></XsorObject>";
        assert_eq!(
            generic_parts(&handle(set(103, game_state_key, qa_state), &store).unwrap()).status,
            200
        );
        assert_eq!(
            hit_xml(&handle(get(104, game_state_key), &store).unwrap()).as_bytes(),
            qa_state
        );

        let question_key = "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-us.Difficulty.1.ID.251317";
        let question = hit_xml(&handle(get(105, question_key), &store).unwrap());
        assert!(question.contains("<Question>Which company makes and sells the iPod?</Question>"));
    }

    #[test]
    fn compressed_set_rejects_inflated_values_over_one_mib_without_truncating() {
        let store = on_manual_clock(Store::bootstrap());
        let too_large = vec![b'x'; 0x100000 + 1];
        let compressed = zlib(&too_large).unwrap();
        let reply = handle(
            named_frame(
                16,
                "v3.Session.MGS.1vs100.Store.P.TooLarge",
                "",
                true,
                &compressed,
            ),
            &store,
        )
        .unwrap();
        assert_eq!(
            generic_parts(&reply),
            GenericParts {
                kind: 0x82,
                cookie: 16,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );
        let miss = handle(get(17, "v3.Session.MGS.1vs100.Store.P.TooLarge"), &store).unwrap();
        assert_eq!(
            generic_parts(&miss),
            GenericParts {
                kind: 0x82,
                cookie: 17,
                status: 404,
                compressed: false,
                value: &b""[..]
            }
        );
    }

    #[test]
    fn stored_incompressible_value_get_cannot_emit_oversized_frame() {
        let store = Mutex::new(Store::bootstrap());
        let mut value = vec![0; wire::MAX_FRAME - 27];
        let mut state = 0x1234_5678_9abc_def0u64;
        for byte in &mut value {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state as u8;
        }
        assert!(39 + zlib(&value).unwrap().len() > wire::MAX_FRAME);
        let ack = handle(set(21, "k", &value), &store).unwrap();
        assert_eq!(
            generic_parts(&ack),
            GenericParts {
                kind: 0x82,
                cookie: 21,
                status: 200,
                compressed: false,
                value: &b""[..]
            }
        );

        let reply = handle(get(22, "k"), &store).unwrap();
        assert!(reply.len() <= wire::MAX_FRAME);
        assert_eq!(
            generic_parts(&reply),
            GenericParts {
                kind: 0x82,
                cookie: 22,
                status: 500,
                compressed: false,
                value: &b""[..]
            }
        );
    }

    #[test]
    fn scheduled_sessions_use_their_own_ids_and_a_new_show_resets_the_session_state() {
        use crate::{schedule::Mode, time::Utc};
        use std::time::Duration;
        let storage = Arc::new(crate::storage::MemoryStorage::default());
        crate::storage::Storage::set_programme(
            storage.as_ref(),
            &crate::programme::Programme::default(),
        )
        .unwrap();
        let schedule = Mutex::new(
            Schedule::load(crate::schedule::ScheduleConfig::default(), storage).unwrap(),
        );
        let now = SystemTime::now();
        let at = |ago: u64| Utc::from(now - Duration::from_secs(ago));
        // Show A ran two hours ago for an hour; show B started a second ago.
        let a = lock(&schedule)
            .start_show_now(at(2 * 3600), Mode::Primetime, crate::programme::minutes(60))
            .unwrap();
        let b = lock(&schedule)
            .start_show_now(at(1), Mode::Primetime, crate::programme::minutes(30))
            .unwrap();
        let key = |id: &str| format!("v3.Session.MGS.1vs100.Store.{id}.GameState");
        let merged = |id: &str| format!("v3.Setup.MGS.1vs100.Store.Sessions.{id}.MergedLaunchData");

        let store = Mutex::new(Store::bootstrap());
        // MergedLaunchData answers for the asked show.
        let reply =
            hit_xml(&handle_scheduled(get(1, &merged(&a.session_id)), &store, &schedule).unwrap());
        assert!(reply.contains("<Duration>60</Duration>"), "{reply}");
        assert!(reply.contains(&format!("<EpisodeId>{}</EpisodeId>", a.episode_id)));
        assert!(reply.contains(&format!(
            "<StartTime>{}</StartTime>",
            a.plan.start.trunc_secs().to_xsor()
        )));
        let reply =
            hit_xml(&handle_scheduled(get(2, &merged(&b.session_id)), &store, &schedule).unwrap());
        assert!(reply.contains("<Duration>30</Duration>"), "{reply}");

        // Show state resets with the show.
        let state = include_str!("../../fixtures/native-r12-qa-251317.xml");
        handle_scheduled(
            set(3, &key(&a.session_id), state.as_bytes()),
            &store,
            &schedule,
        )
        .unwrap();
        assert!(
            hit_xml(&handle_scheduled(get(4, &key(&a.session_id)), &store, &schedule).unwrap())
                .contains("<name>QA</name>")
        );
        let reply = handle_scheduled(get(5, &key(&b.session_id)), &store, &schedule).unwrap();
        let GenericParts { kind, .. } = generic_parts(&reply);
        assert_ne!(kind, 0, "show B starts without show A's game state");
        assert!(
            lock(&store)
                .get(&key(crate::schedule::CANONICAL_SESSION))
                .is_err()
        );
    }

    #[test]
    fn a_special_is_served_only_its_categorys_questions() {
        use crate::{programme::ShowKind, questions::QuestionBank, time::Utc};
        let storage = Arc::new(crate::storage::MemoryStorage::default());
        crate::storage::Storage::set_programme(
            storage.as_ref(),
            &crate::programme::Programme::default(),
        )
        .unwrap();
        let mut schedule =
            Schedule::load(crate::schedule::ScheduleConfig::default(), storage).unwrap();
        let now = Utc::from(SystemTime::now());
        let tomorrow = crate::programme::Programme::default()
            .zone
            .date_of(now)
            .next_day()
            .unwrap();
        schedule
            .put_special(
                crate::programme::SpecialEpisode {
                    date: tomorrow,
                    at: crate::time::LocalTime::from_hm(20, 0).unwrap(),
                    title: "Movie Night".into(),
                    category: "otdb:11".into(),
                    length: crate::programme::minutes(60),
                    mode: crate::schedule::Mode::Primetime,
                    artwork: None,
                },
                now,
            )
            .unwrap();
        let shows = schedule.shows(now).to_vec();
        let special = shows
            .iter()
            .find(|show| matches!(show.plan.kind, ShowKind::Special { .. }))
            .unwrap()
            .clone();
        let regular = shows
            .iter()
            .find(|show| show.plan.kind == ShowKind::ExtendedPlay)
            .unwrap()
            .clone();
        let schedule = Mutex::new(schedule);
        let question = |id: u32, difficulty: u32, category: &str| {
            format!(
                r#"{{"id":{id},"difficulty":{difficulty},"text":"Q{id}","choices":["A","B","C"],
                    "correct":1,"categories":["{category}"],"source":"otdb"}}"#
            )
        };
        let file = format!(
            r#"{{"version":1,"sources":[{{"id":"otdb","name":"Open Trivia Database"}}],
                "categories":[{{"id":"otdb:9","name":"General Knowledge","source":"otdb"}},
                              {{"id":"otdb:11","name":"Film","source":"otdb"}}],
                "questions":[{},{},{},{}]}}"#,
            question(1, 1, "otdb:11"),
            question(2, 1, "otdb:9"),
            question(3, 1, "otdb:11"),
            question(4, 3, "otdb:9"),
        );
        let bank = QuestionBank::from_file(serde_json::from_str(&file).unwrap()).unwrap();
        let store = Mutex::new(Store::bootstrap().with_questions(Arc::new(bank)));
        // ResetID by episode, Available by session; upper case as consoles send.
        let reset = |episode: &str, difficulty: u32| {
            format!(
                "v3.Setup.MGS.1vs100.Store.QuestionBank.{}.Locale.en-US.Difficulty.{difficulty}.ResetID",
                episode.to_ascii_uppercase()
            )
        };
        let available = |session: &str, difficulty: u32| {
            format!(
                "v3.Session.MGS.1vs100.Store.{}.PRIMETIME.QuestionBank.Difficulty.{difficulty}.Available",
                session.to_ascii_uppercase()
            )
        };
        let mut cookie = 0;
        let mut ids = |key: String| -> Vec<u32> {
            cookie += 1;
            let xml = hit_xml(&handle_scheduled(get(cookie, &key), &store, &schedule).unwrap());
            // Sorted: each show is served its own order.
            let mut ids: Vec<u32> = xml
                .split("<XsorItem")
                .skip(1)
                .map(|item| item.split(['>', '<']).nth(1).unwrap().parse().unwrap())
                .collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(ids(reset(&special.episode_id, 1)), [1, 3]);
        assert_eq!(ids(available(&special.session_id, 1)), [1, 3]);
        assert_eq!(
            ids(reset(&special.episode_id, 3)),
            [4],
            "none of the category: the whole difficulty"
        );
        assert_eq!(ids(reset(&regular.episode_id, 1)), [1, 2, 3]);
        assert_eq!(ids(available(&regular.session_id, 1)), [1, 2, 3]);
        assert_eq!(
            ids(reset("0a1b2c3d-1111-4222-8333-444455556666", 1)),
            [1, 2, 3],
            "an episode not in the schedule"
        );
    }
}

#[cfg(test)]
mod lobby_sum_tests {
    use super::key::{Key, ParsedKey};
    use crate::ids::Xuid;

    #[test]
    fn detects_lobby_player_count_sum_keys() {
        let lobby = |key: &str| matches!(ParsedKey::parse(key).kind, Key::LobbyPlayerCount { .. });
        assert!(lobby(
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.LobbyPlayerCount.285"
        ));
        assert!(!lobby(
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.30.TOTAL_PLAYER_COUNT"
        ));
    }

    #[test]
    fn the_one_may_be_any_local_player_of_the_posting_console() {
        let players = [Xuid(100), Xuid(200)];
        let store = crate::store::Store::bootstrap();
        let key = "v3.Session.MGS.1vs100.Store.P.GameState";
        let posted = |ones: u64| {
            let mut doc = crate::gamestate::recorded("QA", true);
            doc.set_field("OnesXuid", ones);
            doc.to_xml()
        };
        assert!(crate::sbg::handlers::is_ones_console(
            &players,
            &posted(100),
            &store,
            key
        ));
        assert!(
            crate::sbg::handlers::is_ones_console(&players, &posted(200), &store, key),
            "user 1 is the One"
        );
        assert!(!crate::sbg::handlers::is_ones_console(
            &players,
            &posted(300),
            &store,
            key
        ));
    }
}
