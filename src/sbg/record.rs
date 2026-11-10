//! SBG session recordings: every frame in and out, with time and caller, written by
//! `sbg::capture`. An event's time is milliseconds after `started_unix_ms` on the wall clock the
//! logs use.
//! Text, one event per line after a `#` header; replayed by `sbg::replay`.
//! Missing header options replay as served (`xsor::ServedConfig`).

use std::{
    fmt::{self, Write as _},
    net::IpAddr,
};

use crate::{
    ids::{ConnId, Xuid},
    storage::Dump,
    store::SimulationOptions,
    time::Utc,
    wire::Protocol,
    xsor::ServedConfig,
};

const MAGIC: &str = "# onevs100-gameserver SBG recording v1";

/// What the server ran with; a replay needs the same behaviour switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub protocol: Protocol,
    pub options: SimulationOptions,
    pub server_flow: bool,
    pub schedule: bool,
    pub served: ServedConfig,
}

/// Assumed `joiningPercentage` when the header does not name it.
const UNRECORDED_JOINING_PERCENTAGE: u32 = 50;
/// Assumed `hostingPercentage` when the header does not name it.
const UNRECORDED_HOSTING_PERCENTAGE: u32 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Open,
    Close,
    In,
    Out,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Close => "close",
            Self::In => "in",
            Self::Out => "out",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub at_ms: u64,
    pub conn: ConnId,
    pub peer: Option<IpAddr>,
    pub xuid: Option<Xuid>,
    pub kind: Kind,
    /// The raw frame for `In`/`Out`; empty for `Open`/`Close`.
    pub frame: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    pub settings: Settings,
    pub started: Utc,
    pub service_host: Option<String>,
    pub storage: Option<Dump>,
    pub events: Vec<Event>,
    /// Events the recorder dropped while its queue was full; a replay of these may differ.
    pub dropped: u64,
}

/// A host clock before 1970 writes a negative start, which [`parse`] refuses.
pub(super) fn header(settings: &Settings, started: Utc) -> String {
    let flag = |on: bool| u8::from(on);
    let o = settings.options;
    // Absent shutdown option replays without the key.
    let shutdown = settings
        .served
        .matchmake_end_game_shutdown
        .map_or(String::new(), |v| {
            format!(" matchmake_end_game_shutdown={v}")
        });
    format!(
        "{MAGIC}\n# protocol {}\n# started_unix_ms {}\n\
         # options synthetic_mob={} synthetic_crowd={} server_flow={} schedule={} \
         joining_percentage={} hosting_percentage={}{shutdown}\n",
        settings.protocol.build(),
        started.unix_ms(),
        flag(o.synthetic_mob),
        o.synthetic_crowd,
        flag(settings.server_flow),
        flag(settings.schedule),
        settings.served.joining_percentage,
        settings.served.hosting_percentage,
    )
}

pub(super) fn format_event(event: &Event) -> String {
    let mut line = format!(
        "{} {} {} {} {}",
        event.at_ms,
        event.conn,
        event.peer.map_or_else(|| "-".to_owned(), |p| p.to_string()),
        event
            .xuid
            .map_or_else(|| "-".to_owned(), |x| x.0.to_string()),
        event.kind.as_str(),
    );
    if !event.frame.is_empty() {
        line.push(' ');
        for byte in &event.frame {
            let _ = write!(line, "{byte:02x}");
        }
    }
    line
}

/// A recording that does not parse: the line it was read to, and what is wrong there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line of the recording. A header the recording ends without is reported at the
    /// last line it has.
    pub line: usize,
    pub kind: ParseErrorKind,
}

/// What is wrong with the line a recording fails on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// The first line is not a recording's header.
    NotARecording,
    /// The `# protocol` header names a build this server does not read.
    UnknownProtocol,
    /// No usable `# started_unix_ms` header: absent, not a number, or a time a replay cannot
    /// start at.
    StartTime,
    /// The `# options` header names a switch or a value this server does not read.
    BadOptions,
    /// The `# storage` header is not the JSON of a storage dump; what the JSON reader says.
    Storage { detail: String },
    /// An event line does not read.
    BadEvent,
    /// The recording ends without a `# protocol` header.
    MissingProtocol,
    /// The recording ends without an `# options` header.
    MissingOptions,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: ", self.line)?;
        match &self.kind {
            ParseErrorKind::NotARecording => f.write_str("not an SBG recording (missing header)"),
            ParseErrorKind::UnknownProtocol => f.write_str("unknown protocol"),
            ParseErrorKind::StartTime => f.write_str("missing or bad started_unix_ms"),
            ParseErrorKind::BadOptions => f.write_str("bad options"),
            ParseErrorKind::Storage { detail } => f.write_str(detail),
            ParseErrorKind::BadEvent => f.write_str("bad event"),
            ParseErrorKind::MissingProtocol => f.write_str("missing protocol header"),
            ParseErrorKind::MissingOptions => f.write_str("missing options header"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse a recording; the error names the line and what is wrong there.
pub fn parse(text: &str) -> Result<Recording, ParseError> {
    let mut lines = text.lines().enumerate();
    if lines.next().map(|(_, line)| line) != Some(MAGIC) {
        return Err(ParseError {
            line: 1,
            kind: ParseErrorKind::NotARecording,
        });
    }
    let mut protocol = None;
    let mut started = None;
    let mut options = None;
    let mut service_host = None;
    let mut storage = None;
    let mut events = Vec::new();
    let mut dropped: u64 = 0;
    for (index, line) in lines {
        let fail = |kind| ParseError {
            line: index + 1,
            kind,
        };
        if let Some(meta) = line.strip_prefix("# ") {
            match meta.split_once(' ') {
                Some(("protocol", build)) => {
                    protocol = Some(
                        Protocol::parse(build)
                            .ok_or_else(|| fail(ParseErrorKind::UnknownProtocol))?,
                    )
                }
                Some(("started_unix_ms", ms)) => {
                    let ms: i64 = ms
                        .parse()
                        .ok()
                        .filter(|ms| *ms >= 0)
                        .ok_or_else(|| fail(ParseErrorKind::StartTime))?;
                    started = Some(Utc::from_unix_ms(ms));
                }
                Some(("options", list)) => {
                    options =
                        Some(parse_options(list).ok_or_else(|| fail(ParseErrorKind::BadOptions))?)
                }
                Some(("service_host", host)) => service_host = Some(host.to_owned()),
                Some(("dropped", count)) => {
                    let count: u64 = count.parse().map_err(|_| fail(ParseErrorKind::BadEvent))?;
                    dropped = dropped.saturating_add(count);
                }
                Some(("storage", json)) => {
                    storage = Some(serde_json::from_str(json).map_err(|error| {
                        fail(ParseErrorKind::Storage {
                            detail: error.to_string(),
                        })
                    })?)
                }
                _ => {}
            }
            continue;
        }
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        events.push(parse_event(line).ok_or_else(|| fail(ParseErrorKind::BadEvent))?);
    }
    // A header the recording ends without is reported at the last line it has.
    let ended = |kind| ParseError {
        line: text.lines().count().max(1),
        kind,
    };
    let header = options.ok_or_else(|| ended(ParseErrorKind::MissingOptions))?;
    let settings = Settings {
        protocol: protocol.ok_or_else(|| ended(ParseErrorKind::MissingProtocol))?,
        options: header.options,
        server_flow: header.server_flow,
        schedule: header.schedule,
        served: header.served,
    };
    Ok(Recording {
        settings,
        started: started.ok_or_else(|| ended(ParseErrorKind::StartTime))?,
        service_host,
        storage,
        events,
        dropped,
    })
}

/// The switches of a recording's `# options` header; named, so no two flags can be swapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OptionsHeader {
    options: SimulationOptions,
    server_flow: bool,
    schedule: bool,
    served: ServedConfig,
}

fn parse_options(list: &str) -> Option<OptionsHeader> {
    let mut header = OptionsHeader {
        options: SimulationOptions::default(),
        server_flow: false,
        schedule: false,
        served: ServedConfig {
            joining_percentage: UNRECORDED_JOINING_PERCENTAGE,
            hosting_percentage: UNRECORDED_HOSTING_PERCENTAGE,
            // Absent shutdown option: no key was served.
            matchmake_end_game_shutdown: None,
        },
    };
    for pair in list.split_whitespace() {
        let (name, value) = pair.split_once('=')?;
        let flag = || match value {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        };
        match name {
            "synthetic_mob" => header.options.synthetic_mob = flag()?,
            // Option the server does not serve: recordings with it on are refused.
            "qa_end_plus_5s" if flag()? => return None,
            "qa_end_plus_5s" => {}
            "synthetic_crowd" => header.options.synthetic_crowd = value.parse().ok()?,
            "server_flow" => header.server_flow = flag()?,
            "schedule" => header.schedule = flag()?,
            "joining_percentage" => header.served.joining_percentage = value.parse().ok()?,
            "hosting_percentage" => header.served.hosting_percentage = value.parse().ok()?,
            "matchmake_end_game_shutdown" => {
                header.served.matchmake_end_game_shutdown = Some(value.parse().ok()?)
            }
            _ => return None,
        }
    }
    Some(header)
}

fn parse_event(line: &str) -> Option<Event> {
    let mut fields = line.split(' ');
    let at_ms = fields.next()?.parse().ok()?;
    let conn = ConnId(fields.next()?.parse().ok()?);
    let peer = match fields.next()? {
        "-" => None,
        peer => Some(peer.parse().ok()?),
    };
    let xuid = match fields.next()? {
        "-" => None,
        xuid => Some(Xuid(xuid.parse().ok()?)),
    };
    let kind = match fields.next()? {
        "open" => Kind::Open,
        "close" => Kind::Close,
        "in" => Kind::In,
        "out" => Kind::Out,
        _ => return None,
    };
    let frame = match fields.next() {
        Some(hex) if hex.len() % 2 == 0 => (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
            .collect::<Option<Vec<u8>>>()?,
        Some(_) => return None,
        None => Vec::new(),
    };
    let has_frame = matches!(kind, Kind::In | Kind::Out);
    (fields.next().is_none() && has_frame != frame.is_empty()).then_some(Event {
        at_ms,
        conn,
        peer,
        xuid,
        kind,
        frame,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            protocol: Protocol::V1149,
            options: SimulationOptions {
                synthetic_mob: true,
                synthetic_crowd: 60,
            },
            server_flow: true,
            schedule: false,
            served: ServedConfig::LIVE,
        }
    }

    #[test]
    fn malformed_recordings_name_the_line() {
        let no_header = parse("hello").unwrap_err();
        assert_eq!(no_header.line, 1);
        assert_eq!(no_header.kind, ParseErrorKind::NotARecording);
        assert_eq!(
            no_header.to_string(),
            "line 1: not an SBG recording (missing header)"
        );
        let head = header(&settings(), Utc::from_unix_ms(1));
        let error = parse(&format!("{head}0 1 - - in zz\n")).unwrap_err();
        assert_eq!(error.line, 5);
        assert_eq!(error.kind, ParseErrorKind::BadEvent);
        assert!(error.to_string().starts_with("line 5"), "{error}");
        assert!(parse(&format!("{head}0 1 - - open 00\n")).is_err());
        assert!(parse(&format!("{head}0 1 - - in\n")).is_err());
        assert!(parse(&format!("{MAGIC}\n# protocol 1317\n")).is_err());
    }

    #[test]
    fn a_recording_without_a_usable_start_time_does_not_parse() {
        let head = |start: &str| {
            format!(
                "{MAGIC}\n# protocol 1149\n{start}# options synthetic_mob=1 synthetic_crowd=60 \
                 server_flow=1 schedule=0\n"
            )
        };
        let missing = parse(&head("")).unwrap_err();
        assert_eq!(missing.line, 3);
        assert_eq!(missing.kind, ParseErrorKind::StartTime);
        let too_late = parse(&head(&format!("# started_unix_ms {}\n", u64::MAX))).unwrap_err();
        assert_eq!(too_late.line, 3);
        assert_eq!(too_late.kind, ParseErrorKind::StartTime);
        let before_1970 = parse(&head("# started_unix_ms -1\n")).unwrap_err();
        assert_eq!(before_1970.line, 3);
        assert_eq!(before_1970.kind, ParseErrorKind::StartTime);
        assert!(parse(&head("# started_unix_ms 1792715141159\n")).is_ok());
    }

    /// A header in each form a recording holds, and the settings it parses to.
    #[test]
    fn headers_parse_to_their_settings() {
        let head = |options: &str| {
            format!("{MAGIC}\n# protocol 1149\n# started_unix_ms 1\n# options {options}\n")
        };
        let mut recorded = settings();
        recorded.served = ServedConfig {
            joining_percentage: 7,
            hosting_percentage: 9,
            matchmake_end_game_shutdown: Some(11),
        };
        let cases = [
            (
                "the settings this server records",
                header(&settings(), Utc::from_unix_ms(1)),
            ),
            (
                "the two switches apart",
                head("synthetic_mob=0 synthetic_crowd=60 server_flow=0 schedule=1"),
            ),
            (
                "the unserved option off",
                head(
                    "synthetic_mob=1 qa_end_plus_5s=0 synthetic_crowd=60 server_flow=1 schedule=0",
                ),
            ),
            (
                "before the percentages were recorded",
                head("synthetic_mob=1 synthetic_crowd=60 server_flow=1 schedule=0"),
            ),
            (
                "joining recorded, hosting not",
                head(
                    "synthetic_mob=1 synthetic_crowd=60 server_flow=1 schedule=0 \
                     joining_percentage=0",
                ),
            ),
            (
                "no shutdown recorded",
                head(
                    "synthetic_mob=1 synthetic_crowd=60 server_flow=1 schedule=0 \
                     joining_percentage=0 hosting_percentage=100",
                ),
            ),
            (
                "other served percentages",
                header(&recorded, Utc::from_unix_ms(1)),
            ),
        ];
        let shown: Vec<String> = cases
            .into_iter()
            .map(|(what, text)| format!("{what}: {:?}", parse(&text).unwrap().settings))
            .collect();
        insta::assert_snapshot!(shown.join("\n"));
    }

    #[test]
    fn an_option_this_server_does_not_read_does_not_parse() {
        let text = format!(
            "{MAGIC}\n# protocol 1149\n# started_unix_ms 1\n# options synthetic_mob=1 \
             qa_end_plus_5s=1 synthetic_crowd=60 server_flow=1 schedule=0\n"
        );
        assert!(parse(&text).is_err());
    }

    /// One recording of every event shape; its header holds no storage to mask.
    #[test]
    fn a_recording_parses_to_its_events() {
        let text = format!(
            "{}# service_host 192.0.2.10\n\
             0 1 192.0.2.21 - open\n\
             12 1 192.0.2.21 16140909857506937655 in ff0001\n\
             40 1 - - out 0382\n\
             41 1 192.0.2.21 - close\n\
             42 2 2001:db8::21 16140909857506937655 in 03\n",
            header(&settings(), Utc::from_unix_ms(1_792_715_141_159))
        );
        insta::assert_debug_snapshot!(parse(&text).unwrap());
    }

    /// The header, the service host and the first event of every recorded session.
    #[test]
    fn the_recorded_sessions_parse() {
        let mut shown = Vec::new();
        for dir in [
            "fixtures/sessions",
            "fixtures/sessions/first-round-opens-twice",
        ] {
            let mut paths: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "sbg"))
                .collect();
            paths.sort();
            for path in paths {
                let recording = parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
                shown.push(format!(
                    "{}: {:?}, service host {:?}, {} player records, {} events, first {:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    recording.settings,
                    recording.service_host,
                    recording
                        .storage
                        .as_ref()
                        .map_or(0, |dump| dump.players.len()),
                    recording.events.len(),
                    recording.events.first(),
                ));
            }
        }
        insta::assert_snapshot!(shown.join("\n"));
    }
}
