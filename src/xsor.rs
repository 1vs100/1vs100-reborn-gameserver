use std::time::Duration;

use crate::{
    ids::QuestionId,
    programme::{self, ShowKind},
    questions::{Difficulty, QuestionBank, ShowSeed},
    schedule::Showing,
    time::Utc,
};

pub mod table;

use table::{ITEM, Node, native, object};

fn int_items(name: &str, values: &[u32]) -> Node {
    Node::table(name, values.iter().map(|value| Node::int(ITEM, value)))
}

/// Config values a replay must reproduce; recordings carry the served ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServedConfig {
    /// Share of matchmaking attempts that leave the QNet session to search.
    pub joining_percentage: u32,
    /// Share of matchmaking attempts that host a session of their own.
    pub hosting_percentage: u32,
    /// Seconds before program end that matchmaking stops; `None` serves no key.
    pub matchmake_end_game_shutdown: Option<u32>,
}

impl ServedConfig {
    /// Live values. 0 joining: session search finds nobody without Live, but the key must stay.
    /// 100 hosting: every console hosts its own session; the lobby never hosts by itself.
    /// 0 shutdown: matchmaking never stops early; the title reads this one with `or`, so 0 overrides.
    pub const LIVE: Self = Self {
        joining_percentage: 0,
        hosting_percentage: 100,
        matchmake_end_game_shutdown: Some(0),
    };
}

impl Default for ServedConfig {
    fn default() -> Self {
        Self::LIVE
    }
}

/// `scheduled_cutoff`: with a schedule, its `PT_MINIMUM_TIME_TILL_EVENT_END` in minutes.
pub fn config_fixture(served: ServedConfig, scheduled_cutoff: Option<Duration>) -> Vec<u8> {
    let mut fields = vec![
        Node::float("MinGameVersionNumber", "2.02"),
        Node::string("SeasonStartDate", "2026-09-16T01:00:00Z"),
        Node::boolean("SeasonFinale", false),
        Node::string("LegalURL", "http://www.1vs100.com/legal"),
    ];
    for (name, value) in [
        ("hostingPercentage", served.hosting_percentage),
        ("joiningPercentage", served.joining_percentage),
        ("matchmakeDelayMin", 1),
        ("matchmakeDelayMax", 5),
        ("maxAttemptsPerTry", 3),
        ("MATCHMAKE_MAXQUALIFICATION", 100),
        // Minutes (ProgramTime.lua): rounds stop when less than this is left of the show's
        // `Duration`. With a schedule, 5 keeps a round running through the 45:35 sponsor ad of a
        // 50-minute show - the ad is what loads the next show; the title's own
        // default is 10.  The unscheduled fixture (Duration 600) pairs it with 300.
        (
            "PT_MINIMUM_TIME_TILL_EVENT_END",
            scheduled_cutoff.map_or(300, programme::whole_minutes),
        ),
        (
            "QALoopStatTime",
            u32::try_from(crate::pt_timings::QA_LOOP_STAT_TIME.as_secs())
                .expect("QALoopStatTime is whole seconds"),
        ),
        ("ACHIEVEMENT_TPS_TUNING_DIAL", 1),
        ("oneEligibleRTT", 200),
        ("mobEligibleRTT", 200),
        ("oneEligibleBWUp", 80000),
        ("mobEligibleBWUp", 80000),
        ("oneEligibleBWDown", 80000),
        ("mobEligibleBWDown", 80000),
        ("cutHostAudioBWDown", 60000),
        ("warnRTT", 350),
        ("warnBWDown", 80000),
        ("warnBWUp", 64000),
        ("maxJitterMs", 999),
        ("jitterPingThreshold", 1199),
    ] {
        fields.push(Node::int(name, value));
    }
    if let Some(shutdown) = served.matchmake_end_game_shutdown {
        fields.push(Node::int("matchmakeEndGameShutdown", shutdown));
    }
    fields.extend([
        Node::table(
            "msgBuckets",
            [
                int_items("BWUp", &[20000, 40000, 120000]),
                int_items("BWDown", &[32000, 40000, 120000]),
                int_items("RTT", &[150, 300, 400]),
            ],
        ),
        int_items("PTQuestionProgression", &PT_QUESTION_PROGRESSION),
        int_items("EPQuestionProgression", &EP_QUESTION_PROGRESSION),
    ]);
    object("Config", fields)
}

pub fn prizing_matrix_fixture() -> Vec<u8> {
    let mut fields = Vec::new();
    let rows = [
        (0, 0, 0, "n0g4m3"),
        (1, 160, 0, "n0g4m3"),
        (2, 300, 0, "n0g4m3"),
        (3, 600, 0, "n0g4m3"),
        (4, 800, 0, "SomeGame"),
        (5, 1200, 0, "SomeGame"),
        (6, 2000, 80, "SomeGame"),
        (7, 3000, 160, "SomeGame"),
        (8, 5000, 400, "SomeGame"),
        (9, 6000, 800, "SomeGame"),
        (10, 10000, 0, "n0g4m3"),
    ];
    for (tier, one, mob_pt, mob_gt) in rows {
        fields.extend([
            Node::int(format!("PTOneT{tier}"), one),
            Node::int(format!("PTMobPT{tier}"), mob_pt),
            Node::string(format!("PTMobGT{tier}"), mob_gt),
        ]);
    }
    for top in ["PTTop1", "PTTop2", "PTTop3"] {
        fields.push(Node::string(top, "SomeGame"));
    }
    object("PrizingMatrix", fields)
}

/// `Duration` of a show without a schedule, in minutes, although its window is two hours; not
/// yet checked against a recording.
const UNSCHEDULED_DURATION_MINUTES: u64 = 600;

/// `Theme.Name` of a special without artwork; it only needs to be non-empty, as the title shows
/// the theme text only then (`jumbotronscreens.lua`, `InitializeThemeImages`).
const SPECIAL_THEME_NAME: &str = "Special";

/// Keys a special's theme name is sent under: the lobby marquee looks it up by the console
/// locale (`en-us`), the Extended Play screens by the episode locale, the served
/// `AvailableLanguages` entry (`en-US`).
const THEME_LOCALES: [&str; 2] = ["en-us", AVAILABLE_LANGUAGE];

/// The one language served in `AvailableLanguages`.
const AVAILABLE_LANGUAGE: &str = "en-US";

/// A special's theme; other shows have none, which the title shows as its general theme.
fn theme(showing: &Showing) -> Node {
    let (name, localized) = match showing {
        Showing::Scheduled(show) => match &show.plan.kind {
            ShowKind::Special { title, artwork, .. } => (
                artwork
                    .as_ref()
                    .map_or(SPECIAL_THEME_NAME, |art| art.as_str())
                    .to_owned(),
                THEME_LOCALES
                    .iter()
                    .map(|locale| Node::string(*locale, title.as_str()))
                    .collect(),
            ),
            ShowKind::Primetime | ShowKind::ExtendedPlay => (String::new(), Vec::new()),
        },
        Showing::Unscheduled { .. } => (String::new(), Vec::new()),
    };
    Node::table(
        "Theme",
        [
            Node::string("Name", name),
            Node::table("LocalizedNames", localized),
        ],
    )
}

/// `MergedLaunchData` for `showing`: the show whose id was asked for, or the show running now
/// (`Schedule::showing_for_session`); without a schedule the unscheduled window and constant ids.
/// `numeric` types Duration/GameType as integers (v2.0.1149.0 `VerifyGridLaunchData` checks
/// `type(...) == "number"`; v2.0.1317.0 checks `"string"`).
pub fn merged_launch_data_fixture(showing: &Showing, numeric: bool, service_host: &str) -> Vec<u8> {
    let game_type = showing.mode().game_type();
    let (duration, episode_id) = match showing {
        Showing::Scheduled(show) => (show.plan.minutes(), show.episode_id.clone()),
        Showing::Unscheduled { .. } => (
            UNSCHEDULED_DURATION_MINUTES,
            crate::schedule::CANONICAL_EPISODE.to_owned(),
        ),
    };
    // With the title's millisecond field (`...:00.000Z`): it compares a program's `StartTime`
    // string with `tostring(UTCTime(StartTime))` to tell "the program I am in" from "another one"
    // (`program.lua::GetNextProgram`); without the fraction the two never match and the next
    // show is never picked up (no transition).
    let start = Utc::from(showing.start()).trunc_secs().to_xsor();
    let end = Utc::from(showing.end()).trunc_secs().to_xsor();
    let number = |name: &str, value: u64| {
        if numeric {
            Node::int(name, value)
        } else {
            Node::string(name, value)
        }
    };
    let en_us = |name: &str, value: String| Node::table(name, [Node::string("en-us", value)]);
    object(
        "MergedLaunchData",
        [
            Node::string("EpisodeId", episode_id),
            en_us(
                "LiveStreamPathURI",
                format!("http://{service_host}/livehost.mp3"),
            ),
            Node::string("StartTime", start),
            Node::string("EndTime", end),
            number("Duration", duration),
            number("GameType", game_type.into()),
            theme(showing),
            Node::string("EpisodeGroup", ""),
            Node::int("DesignerKnob", 0),
            Node::int("MobBucketSize", 5),
            en_us("SeasonSponsor", "Microsoft".into()),
            en_us("EpisodeSponsor", "Microsoft".into()),
        ],
    )
}

pub fn available_languages_fixture() -> Vec<u8> {
    object(
        "AvailableLanguages",
        [Node::table(
            "Languages",
            [Node::string(ITEM, AVAILABLE_LANGUAGE)],
        )],
    )
}

pub fn sum_value(total: f64) -> Vec<u8> {
    object("Sum", [Node::string("Value", total)])
}

pub fn top_n_values(entries: &[(f64, Vec<u8>)]) -> Vec<u8> {
    // f64 Display round-trips every finite stored score as valid XML numeric text.
    let values = entries.iter().map(|(score, metadata)| {
        Node::table(
            ITEM,
            [
                Node::string("Value", score),
                Node::raw("Metadata", metadata),
            ],
        )
    });
    object("TopN", [Node::table("Values", values)])
}

pub fn user_storage_default_fixture() -> Vec<u8> {
    let mut fields: Vec<Node> = [
        "LQ", "LC", "LS", "LRS", "LME", "LCS", "LLS", "LGP", "CW", "CWQ", "CWC",
    ]
    .into_iter()
    .map(|name| Node::int(name, 0))
    .collect();
    for name in ["CWA", "RH", "Sweepstakes"] {
        fields.push(Node::table(name, []));
    }
    let mut ps = vec![Node::boolean("bValid", false), Node::int("optionNum", 2)];
    for (name, value) in [
        ("Prizes", false),
        ("Callout", false),
        ("TheOne", false),
        ("Mob", false),
        ("ExMob", false),
        ("Crowd", true),
    ] {
        ps.push(Node::boolean(name, value));
    }
    fields.push(Node::table("PS", ps));
    object("UserStorage", fields)
}
/// Difficulty of each Prime Time question (Config `PTQuestionProgression`); the One picks its
/// questions by it.
pub const PT_QUESTION_PROGRESSION: [u32; 21] = [
    1, 1, 2, 2, 2, 3, 3, 3, 3, 4, 3, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5,
];

/// Difficulty of each Extended Play question (Config `EPQuestionProgression`, overriding the
/// title's `data.lua` default). An EP show takes one id per entry from the ResetID lists up
/// front, so the bank must hold at least this many per difficulty (2/5/19/8/3).
pub const EP_QUESTION_PROGRESSION: [u32; 37] = [
    1, 1, 2, 2, 2, 3, 2, 2, 3, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 5, 3, 4, 3, 4, 3, 4, 3, 3, 4, 5, 4, 3,
    4, 3, 4, 5, 3,
];

pub fn question_payload(bank: &QuestionBank, question_id: QuestionId) -> Option<Vec<u8>> {
    let question = bank.get(question_id)?;
    let choices = question
        .choices
        .iter()
        .map(|choice| Node::string(ITEM, choice));
    Some(object(
        "question",
        [
            Node::string("Question", &question.text),
            Node::int("CorrectAnswer", question.correct.get()),
            Node::table("ChoiceList", choices),
        ],
    ))
}

fn ids_at(
    bank: &QuestionBank,
    difficulty: Difficulty,
    category: Option<&str>,
    show: ShowSeed,
    item: impl Fn(QuestionId) -> Node,
) -> Vec<Node> {
    bank.listed_ids(difficulty, category, show)
        .into_iter()
        .map(item)
        .collect()
}

/// The ResetID list at `difficulty`; `None` for a number that names no difficulty.
pub fn question_reset_ids(
    bank: &QuestionBank,
    difficulty: u32,
    category: Option<&str>,
    show: ShowSeed,
) -> Option<Vec<u8>> {
    Difficulty::from_number(difficulty).map(|difficulty| {
        object(
            "ResetID",
            ids_at(bank, difficulty, category, show, |id| Node::int(ITEM, id)),
        )
    })
}

/// The Available list at `difficulty`; `None` for a number that names no difficulty.
pub fn question_bank_available(
    bank: &QuestionBank,
    difficulty: u32,
    category: Option<&str>,
    show: ShowSeed,
) -> Option<Vec<u8>> {
    Difficulty::from_number(difficulty).map(|difficulty| {
        native(ids_at(bank, difficulty, category, show, |id| {
            Node::float(ITEM, id)
        }))
    })
}

/// A question list the title stored (Available or Used), served as the bank's lists are: the
/// same ids in the same order. `None` if it is not a list of ids.
pub fn stored_question_list(stored: &[u8]) -> Option<Vec<u8>> {
    let root = table::parse(stored)?;
    let ids = root
        .children()
        .iter()
        .map(|item| {
            let id: f64 = item.text_value()?.trim().parse().ok()?;
            (id >= 0.0 && id.fract() == 0.0 && id <= f64::from(u32::MAX)).then_some(id as u32)
        })
        .collect::<Option<Vec<u32>>>()?;
    Some(native(ids.into_iter().map(|id| Node::float(ITEM, id))))
}

pub fn question_bank_used(difficulty: u32) -> Option<Vec<u8>> {
    Difficulty::from_number(difficulty).map(|_| native([]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::programme::{PlannedShow, Show, ShowKind};

    fn scheduled(kind: ShowKind, minutes: u64) -> Showing {
        let start = Utc::parse("2026-10-23T20:00:00Z").unwrap();
        Showing::Scheduled(Show {
            session_id: "session".into(),
            episode_id: "episode".into(),
            plan: PlannedShow {
                start,
                end: start + Duration::from_secs(minutes * 60),
                kind,
            },
            manual: false,
        })
    }

    fn special(mode: crate::schedule::Mode, artwork: Option<&str>) -> ShowKind {
        ShowKind::Special {
            title: "Movie Night".into(),
            category: "otdb:11".into(),
            mode,
            artwork: artwork.map(|name| crate::programme::ThemeArtwork::new(name).unwrap()),
        }
    }

    /// A document as its elements, one per line.
    fn lines(xml: Vec<u8>) -> String {
        String::from_utf8(xml)
            .expect("a served document is UTF-8")
            .replace("><", ">\n<")
    }

    /// Every document the server serves, each over representative input.
    #[test]
    fn the_served_documents() {
        use crate::schedule::Mode;
        let now = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_510_400);
        let uncut = ServedConfig {
            joining_percentage: 7,
            hosting_percentage: 9,
            matchmake_end_game_shutdown: None,
        };
        let bank = &QuestionBank::seed();
        let seed = ShowSeed::of("f605b5d4-7f6b-42b0-95ab-16aee9b51db4");
        let shown = |xml: Option<Vec<u8>>| lines(xml.expect("a served document"));
        let documents = [
            ("config", lines(config_fixture(ServedConfig::LIVE, None))),
            (
                "a config cut off 45 minutes before the show ends",
                lines(config_fixture(uncut, Some(Duration::from_secs(45 * 60)))),
            ),
            ("prizing matrix", lines(prizing_matrix_fixture())),
            (
                "launch data, Primetime, numbers",
                lines(merged_launch_data_fixture(
                    &scheduled(ShowKind::Primetime, 60),
                    true,
                    "192.0.2.10",
                )),
            ),
            (
                "launch data, Extended Play, strings",
                lines(merged_launch_data_fixture(
                    &scheduled(ShowKind::ExtendedPlay, 120),
                    false,
                    "192.0.2.10",
                )),
            ),
            (
                "launch data, a special",
                lines(merged_launch_data_fixture(
                    &scheduled(special(Mode::Primetime, None), 90),
                    true,
                    "192.0.2.10",
                )),
            ),
            (
                "launch data, a special with artwork",
                lines(merged_launch_data_fixture(
                    &scheduled(special(Mode::ExtendedPlay, Some("Community")), 30),
                    true,
                    "192.0.2.10",
                )),
            ),
            (
                "launch data without a schedule",
                lines(merged_launch_data_fixture(
                    &crate::schedule::Schedule::default().showing_for_session(now, None),
                    true,
                    "192.0.2.10",
                )),
            ),
            ("available languages", lines(available_languages_fixture())),
            ("a sum", lines(sum_value(102.0))),
            (
                "a board",
                lines(top_n_values(&[(
                    0.72,
                    b"<Metadata>16140909857506937655</Metadata>".to_vec(),
                )])),
            ),
            ("a player's documents", lines(user_storage_default_fixture())),
            (
                "a question",
                shown(question_payload(bank, QuestionId(251_317))),
            ),
            ("the reset ids", shown(question_reset_ids(bank, 1, None, seed))),
            (
                "the available ids",
                shown(question_bank_available(bank, 1, None, seed)),
            ),
            ("the used ids", shown(question_bank_used(1))),
            (
                "a list the title stored",
                shown(stored_question_list(
                    b"<XsorObject><XsorItem type=\"float\">251317</XsorItem><XsorItem type=\"float\">251401</XsorItem></XsorObject>",
                )),
            ),
        ];
        let shown: Vec<String> = documents
            .into_iter()
            .map(|(what, document)| format!("{what}:\n{document}"))
            .collect();
        insta::assert_snapshot!(shown.join("\n"));
    }

    #[test]
    fn question_bank_covers_an_extended_play_show() {
        // An EP show takes one id per entry of the served progression, up front.
        for difficulty in Difficulty::ALL {
            let needed = EP_QUESTION_PROGRESSION
                .iter()
                .filter(|&&d| d == difficulty.number())
                .count();
            let have = QuestionBank::seed().ids(difficulty, None).len();
            assert!(have >= needed, "difficulty {difficulty}: {have} < {needed}");
        }
    }

    #[test]
    fn question_bank_is_well_formed() {
        let seed = QuestionBank::seed();
        assert_eq!(
            QuestionBank::from_file(seed.to_file()).as_ref(),
            Ok(&seed),
            "valid"
        );
        for question in seed.questions() {
            assert!(question_payload(&seed, question.id).is_some());
        }
        for difficulty in Difficulty::ALL {
            assert!(!seed.ids(difficulty, None).is_empty());
        }
    }
}
