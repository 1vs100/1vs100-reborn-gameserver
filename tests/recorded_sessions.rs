//! Recorded sessions: every recording directly under `fixtures/sessions/` (made with
//! `GAMESERVER_RECORD_SBG` on a LAN game) replays without a single differing reply, holds and
//! state progression included (the replay runs on a virtual clock, see `sbg::replay`).
//!
//! The replay sets process-wide SBG state (the service host, server flow) from the recording, so
//! it lives in its own test binary, and every recording here must share those settings.
//!
//! `fixtures/sessions/first-round-opens-twice/` holds the recordings where the first round opens
//! twice; they are evidence for the title model (`tests/title_state_manager.rs`) and are not
//! replayed here.

use std::path::Path;

use onevs100_gameserver::{
    ids::ConnId,
    sbg::{record, replay},
};

/// Replies where a recording and the server disagree, each with the reason the server answers
/// differently (every replay diff is explained). Anything else that differs fails the test, and
/// so does an entry whose replies match again.
struct KnownDifference {
    fixture: &'static str,
    /// (connection, cookie) of each reply.
    replies: &'static [(ConnId, u16)],
    /// Why the server answers differently.
    reason: &'static str,
}

const KNOWN_DIFFERENCES: &[KnownDifference] = &[
    KnownDifference {
        fixture: "2026-10-23-1149-money-or-mob.sbg",
        replies: &[
            (ConnId(1), 129),
            (ConnId(1), 131),
            (ConnId(1), 132),
            (ConnId(1), 133),
            (ConnId(1), 134),
            (ConnId(1), 135),
            (ConnId(1), 136),
            (ConnId(1), 138),
            (ConnId(1), 142),
            (ConnId(1), 143),
        ],
        reason: QUESTION_LISTS_ARE_SAMPLED_PER_SHOW,
    },
    KnownDifference {
        fixture: "2026-10-23-1149-all-helps-two-players.sbg",
        replies: &[
            (ConnId(1), 138),
            (ConnId(1), 140),
            (ConnId(1), 141),
            (ConnId(1), 143),
            (ConnId(1), 145),
            (ConnId(1), 146),
            (ConnId(1), 147),
            (ConnId(1), 150),
            (ConnId(1), 151),
            (ConnId(1), 152),
            (ConnId(1), 547),
            (ConnId(1), 549),
            (ConnId(1), 551),
            (ConnId(1), 554),
            (ConnId(1), 555),
            (ConnId(1), 556),
            (ConnId(1), 558),
        ],
        reason: QUESTION_LISTS_ARE_SAMPLED_PER_SHOW,
    },
    KnownDifference {
        fixture: "2026-10-22-1317-two-consoles.sbg",
        replies: &[
            (ConnId(2), 31),
            (ConnId(2), 32),
            (ConnId(2), 34),
            (ConnId(2), 35),
            (ConnId(2), 37),
            (ConnId(2), 38),
            (ConnId(2), 40),
            (ConnId(2), 41),
            (ConnId(2), 43),
            (ConnId(2), 44),
            (ConnId(2), 158),
            (ConnId(2), 161),
            (ConnId(2), 162),
            (ConnId(2), 164),
            (ConnId(2), 165),
            (ConnId(2), 167),
            (ConnId(2), 168),
            (ConnId(2), 170),
            (ConnId(2), 171),
        ],
        reason: QUESTION_LISTS_ARE_SAMPLED_PER_SHOW,
    },
    KnownDifference {
        fixture: "2026-10-22-1149-two-consoles.sbg",
        replies: &[
            (ConnId(2), 133),
            (ConnId(2), 135),
            (ConnId(2), 136),
            (ConnId(2), 137),
            (ConnId(2), 139),
            (ConnId(2), 141),
            (ConnId(2), 142),
            (ConnId(2), 143),
            (ConnId(2), 144),
            (ConnId(2), 146),
            (ConnId(2), 286),
            (ConnId(2), 288),
            (ConnId(2), 292),
            (ConnId(2), 293),
            (ConnId(2), 294),
            (ConnId(2), 296),
            (ConnId(2), 297),
            (ConnId(2), 299),
        ],
        reason: QUESTION_LISTS_ARE_SAMPLED_PER_SHOW,
    },
    KnownDifference {
        fixture: "2026-10-22-1149-two-consoles.sbg",
        replies: &[
            (ConnId(1), 184),
            (ConnId(1), 204),
            (ConnId(1), 206),
            (ConnId(1), 207),
            (ConnId(1), 208),
            (ConnId(2), 219),
            (ConnId(2), 246),
            (ConnId(2), 250),
            (ConnId(2), 251),
            (ConnId(2), 253),
        ],
        reason: MOB_ANSWERS_THE_ASKED_QUESTION,
    },
    KnownDifference {
        fixture: "2026-10-22-1317-two-consoles.sbg",
        replies: &[
            (ConnId(1), 85),
            (ConnId(1), 88),
            (ConnId(1), 89),
            (ConnId(2), 120),
            (ConnId(2), 125),
            (ConnId(2), 126),
        ],
        reason: MOB_ANSWERS_THE_ASKED_QUESTION,
    },
    KnownDifference {
        fixture: "2026-10-23-1149-money-or-mob.sbg",
        replies: &[(ConnId(1), 211), (ConnId(1), 237)],
        reason: MOB_ANSWERS_THE_ASKED_QUESTION,
    },
    KnownDifference {
        fixture: "2026-10-22-1149-two-consoles.sbg",
        replies: &[
            (ConnId(1), 214),
            (ConnId(1), 215),
            (ConnId(1), 216),
            (ConnId(1), 218),
            (ConnId(2), 258),
            (ConnId(2), 259),
            (ConnId(2), 263),
            (ConnId(2), 265),
        ],
        reason: STAT_BREAKS_COUNT_THE_SIMULATED_MOB,
    },
    KnownDifference {
        fixture: "2026-10-22-1317-two-consoles.sbg",
        replies: &[
            (ConnId(1), 97),
            (ConnId(1), 99),
            (ConnId(1), 101),
            (ConnId(2), 134),
            (ConnId(2), 136),
            (ConnId(2), 141),
        ],
        reason: STAT_BREAKS_COUNT_THE_SIMULATED_MOB,
    },
    KnownDifference {
        fixture: "2026-10-22-1149-two-consoles.sbg",
        replies: &[(ConnId(1), 209), (ConnId(2), 254)],
        reason: THE_SIMULATED_MOB_POSTS_A_BRAIN,
    },
    KnownDifference {
        fixture: "2026-10-22-1317-two-consoles.sbg",
        replies: &[(ConnId(1), 90), (ConnId(2), 127)],
        reason: THE_SIMULATED_MOB_POSTS_A_BRAIN,
    },
    KnownDifference {
        fixture: "2026-10-22-1149-two-consoles.sbg",
        replies: &[
            (ConnId(2), 289),
            (ConnId(2), 290),
            (ConnId(2), 291),
            (ConnId(2), 298),
        ],
        reason: THE_TITLES_QUESTION_TRACKING_IS_KEPT,
    },
    KnownDifference {
        fixture: "2026-10-22-1317-two-consoles.sbg",
        replies: &[(ConnId(2), 159), (ConnId(2), 160)],
        reason: THE_TITLES_QUESTION_TRACKING_IS_KEPT,
    },
    KnownDifference {
        fixture: "2026-10-23-1149-all-helps-two-players.sbg",
        replies: &[
            (ConnId(1), 544),
            (ConnId(1), 545),
            (ConnId(1), 550),
            (ConnId(1), 552),
            (ConnId(1), 553),
            (ConnId(1), 557),
        ],
        reason: THE_TITLES_QUESTION_TRACKING_IS_KEPT,
    },
    KnownDifference {
        fixture: "2026-10-22-1149-two-consoles.sbg",
        replies: &[
            (ConnId(1), 106),
            (ConnId(1), 108),
            (ConnId(1), 122),
            (ConnId(1), 124),
            (ConnId(2), 107),
            (ConnId(2), 108),
            (ConnId(2), 122),
            (ConnId(2), 124),
        ],
        reason: ONLY_A_SPECIAL_HAS_A_THEME,
    },
    KnownDifference {
        fixture: "2026-10-22-1317-two-consoles.sbg",
        replies: &[
            (ConnId(1), 6),
            (ConnId(1), 7),
            (ConnId(1), 22),
            (ConnId(1), 24),
            (ConnId(2), 6),
            (ConnId(2), 7),
            (ConnId(2), 21),
            (ConnId(2), 23),
        ],
        reason: ONLY_A_SPECIAL_HAS_A_THEME,
    },
    KnownDifference {
        fixture: "2026-10-23-1149-all-helps-two-players.sbg",
        replies: &[
            (ConnId(1), 107),
            (ConnId(1), 108),
            (ConnId(1), 126),
            (ConnId(1), 128),
        ],
        reason: ONLY_A_SPECIAL_HAS_A_THEME,
    },
    KnownDifference {
        fixture: "2026-10-23-1149-money-or-mob.sbg",
        replies: &[
            (ConnId(1), 107),
            (ConnId(1), 108),
            (ConnId(1), 111),
            (ConnId(1), 112),
        ],
        reason: ONLY_A_SPECIAL_HAS_A_THEME,
    },
];

/// A show's `Theme.Name` is `Its_you_versus_everyone` with no `LocalizedNames` in the recording,
/// which the title renders as the theme text `NoTheme`. Only a special has a theme; other shows
/// send an empty name. These are the MergedLaunchData reads.
const ONLY_A_SPECIAL_HAS_A_THEME: &str = "only a special has a theme";

/// The recording holds the synthetic Mob and Crowd answering the question of whatever state was
/// stored, so after question 1 (251317) they answer 251317 rather than the question asked. These
/// are the MobChoices and Crowd.ChoiceN reads of questions 2 and 3.
const MOB_ANSWERS_THE_ASKED_QUESTION: &str = "the mob answers the question asked";

/// The Brain board (`Round.R.Question.Q.Brain`) in the recording holds only real posts, the Mob
/// console's own entry alone; the server adds the simulated Mob's ten highest. These are the
/// reads of it.
const THE_SIMULATED_MOB_POSTS_A_BRAIN: &str = "the simulated mob posts a brain";

/// The title writes the show's `QuestionBank.Difficulty.N.Available` and `...Used` lists back as
/// it picks questions, and the recording holds the bank's full list and an empty Used where the
/// server answers what the title wrote. These are its reads after its first writes: Used holds
/// the ids already asked and Available leaves them out (e.g. difficulty 1: Available 251317,
/// 251401, 251402 in the recording, 251402 in the reply, and Used 251317, 251401), so no
/// question is asked twice.
const THE_TITLES_QUESTION_TRACKING_IS_KEPT: &str = "the titles question tracking is kept";

/// Each show is served a seeded sample of its question lists (`questions::MAX_LISTED_IDS`), so a
/// list fits the title's inflate buffer (`wire::v1149::INFLATE_CAPACITY`). These are the ResetID
/// and Available reads: the recorded ids, in the show's order.
const QUESTION_LISTS_ARE_SAMPLED_PER_SHOW: &str = "the question lists are sampled per show";

/// The recording's stat-break buckets (`StatBreaks.NumCorrectAnswers.BucketN`) count only the
/// consoles' own players; the server adds the simulated Mob. These are the bucket reads after
/// question 2 or 3 that the Mob's 100 members land in.
const STAT_BREAKS_COUNT_THE_SIMULATED_MOB: &str = "the stat breaks count the simulated mob";

#[test]
fn recorded_sessions_replay_to_their_recorded_replies() {
    let mut paths: Vec<_> = std::fs::read_dir(Path::new("fixtures/sessions"))
        .expect("fixtures/sessions")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sbg"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no recorded sessions");
    for path in paths {
        let name = path.display().to_string();
        let recording = record::parse(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let mismatches =
            replay::replay(&recording).unwrap_or_else(|error| panic!("{name}: {error}"));
        let file = path.file_name().unwrap().to_str().unwrap();
        let known: Vec<(ConnId, u16)> = KNOWN_DIFFERENCES
            .iter()
            .filter(|known| known.fixture == file)
            .flat_map(|known| known.replies.iter().copied())
            .collect();
        let unexplained: Vec<_> = mismatches
            .iter()
            .filter(|m| {
                !m.cookie
                    .is_some_and(|cookie| known.contains(&(m.conn, cookie)))
            })
            .collect();
        let reasons: Vec<_> = KNOWN_DIFFERENCES
            .iter()
            .filter(|known| known.fixture == file)
            .map(|known| known.reason)
            .collect();
        assert!(
            unexplained.is_empty(),
            "{name}: {} replies differ:\n{unexplained:#?}",
            unexplained.len()
        );
        let stale: Vec<_> = known
            .iter()
            .filter(|reply| {
                !mismatches
                    .iter()
                    .any(|m| m.cookie.is_some_and(|cookie| (m.conn, cookie) == **reply))
            })
            .collect();
        assert!(
            stale.is_empty(),
            "{name}: known differences ({reasons:?}) whose replies match: {stale:?}"
        );
    }
}
