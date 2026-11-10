//! SBG store key parsing: what a key addresses and where its parts are.
//! Segments match case-insensitively; [`ParsedKey::raw`] keeps the key as sent.

use crate::{
    ids::{QuestionId, QuestionNumber, Round},
    primetime::{Bucket, StatBreak},
    questions::Choice,
};

/// Parsed store key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedKey {
    /// Key as sent; values, sums, and boards store under it.
    pub raw: String,
    /// Second segment.
    pub namespace: Option<Namespace>,
    /// Store, Sum, or TopN; unknown Sum keys are still sums.
    pub collection: Option<Collection>,
    pub kind: Key,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    Session,
    Season,
    Setup,
    Stats,
    Log,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collection {
    Store,
    Sum,
    TopN,
}

/// Show id: lowercased session segment of a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionId(String);

impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Folded session; the simulation answers only for it.
    pub fn is_canonical(&self) -> bool {
        self.0 == crate::schedule::CANONICAL_SESSION
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// `Session.Store.<S>.GameState`: the show's state.
    GameState { session: SessionId },
    /// The One's per-state copy for data collection.
    DataCollectionState {
        season: bool,
        session: SessionId,
        round: Round,
        question: Option<QuestionNumber>,
        /// State name after the `GameState` prefix, as sent.
        name: String,
    },
    /// `...<S>.PRIMETIME.<id>.<item>`: a role election window.
    Election {
        session: SessionId,
        id: u32,
        item: ElectionItem,
    },
    /// `Session.TopN.<S>.PRIMETIME.Round.R.Question.Q.<board>`.
    QuestionBoard {
        session: SessionId,
        round: Round,
        question: Option<QuestionNumber>,
        board: Board,
    },
    /// `Session.Sum.<S>.PRIMETIME.Round.R.Question.Q.<sum>`.
    QuestionSum {
        session: SessionId,
        round: Round,
        question: Option<QuestionNumber>,
        sum: QuestionSum,
    },
    /// `Session.Sum.<S>.PRIMETIME.LobbyPlayerCount.<secs>`.
    LobbyPlayerCount { session: SessionId, secs: u32 },
    /// `Session.Sum.Program.<S>.QOS.<id>....`: connection quality reports.
    Qos { session: SessionId },
    /// `Session.Store.<S>.PRIMETIME.QuestionBank.Difficulty.<D>.<Available|Used>`.
    SessionQuestionList {
        session: SessionId,
        difficulty: u8,
        list: QuestionList,
    },
    /// `Setup.Store.QuestionBank.<S>.Locale.<L>.Difficulty.<D>.<ResetID|ID.<qid>>`, or the short
    /// `Setup.Store.QuestionBank.Difficulty.<D>....` without bank and locale.
    QuestionBank {
        /// Lowercased; `None` for the short form.
        locale: Option<String>,
        difficulty: u8,
        entry: BankEntry,
    },
    /// `Setup.Store.Config`.
    Config,
    /// `Setup.Store.Config.Week<W>`.
    ConfigWeek(u8),
    /// `Setup.Store.Pangolin.Config`.
    PangolinConfig,
    /// `Setup.Store.<S>.AvailableLanguages`.
    AvailableLanguages,
    /// `Setup.Store.Sessions.<S>.MergedLaunchData`.
    MergedLaunchData { session: SessionId },
    /// `Season.UserStore.<UserStorage|UserStats>`: a player's documents.
    User(UserDoc),
    /// `Session.Log.UserError.<code>`: the title's error reports (`sbg::user_error`).
    UserError,
    /// `Session.Log.URLogging` and `Log.Store.URLogging`: the title's telemetry.
    UrLogging,
    /// `Season.Store.Prizing.Session.<S>.Xuid.<X>.Round.R.One`: the One's prize record.
    PrizingOne { session: SessionId, round: Round },
    /// `Season.TopN.Prizing.Session.<S>.Round.R.RoundEndTopScores`: the top-3 call-out.
    RoundEndTopScores { session: SessionId, round: Round },
    /// `Season.TopN.<S>.<game>.Question.Q.RoundScores`: Extended Play's leaderboard, for the
    /// whole show (`STAT_EP_ROUND_TOP_SCORE`).
    ExtendedPlayRoundScores {
        session: SessionId,
        question: Option<QuestionNumber>,
    },
    /// `Stats.Sum....`: global counters (persisted).
    StatsSum,
    /// `Stats.TopN....`: the host tool's boards.
    StatsTopN,
    /// Any other key: a plain store key.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElectionItem {
    /// TopN: candidacies for the One.
    TheOne,
    /// TopN: candidacies for the Mob.
    MobSelection,
    /// Store: the elected Mob.
    TheMob,
    /// TopN: Mob players' avatars, bucket 1-13.
    RealAvatars(u8),
    /// Sum: players in the round.
    RsTotalPlayerCount,
    /// Anything else under an election id: still folded into the round's election.
    Other,
}

/// Which show id a key carries (see [`ParsedKey::show_id`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowIdKind {
    Session,
    Episode,
}

/// Where a key carries a show id: the raw key segment holding it, its kind, and the id as sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShowIdRef<'k> {
    /// Index of the segment in the raw key.
    pub index: usize,
    pub kind: ShowIdKind,
    pub id: &'k str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Board {
    MobChoices,
    Brain,
    RoundScores,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestionSum {
    /// The Crowd's answers.
    Crowd(Choice),
    TotalPlayerCount,
    MatchMake(MatchMake),
    StatBreak(Bucket),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchMake {
    Host,
    Join,
    Leave,
    Wait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestionList {
    Available,
    Used,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BankEntry {
    ResetId,
    Id(QuestionId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserDoc {
    Storage,
    Stats,
}

impl Key {
    /// A question id list (Available, Used, ResetID), as opposed to one question.
    pub fn is_question_list(&self) -> bool {
        matches!(
            self,
            Self::SessionQuestionList { .. }
                | Self::QuestionBank {
                    entry: BankEntry::ResetId,
                    ..
                }
        )
    }
}

impl ParsedKey {
    /// Counter; GETs read, posts add.
    pub fn is_sum(&self) -> bool {
        self.collection == Some(Collection::Sum)
            && matches!(
                self.namespace,
                Some(Namespace::Session | Namespace::Season | Namespace::Stats)
            )
    }

    /// TopN board; Stats boards serve as plain keys.
    pub fn is_top_n(&self) -> bool {
        self.collection == Some(Collection::TopN)
            && matches!(self.namespace, Some(Namespace::Session | Namespace::Season))
    }

    /// Where the key names its show, if it does.
    pub fn show_id(&self) -> Option<ShowIdRef<'_>> {
        let parts: Vec<&str> = self.raw.split('.').collect();
        if parts.len() < 6 {
            return None;
        }
        let is = |index: usize, word: &str| {
            parts
                .get(index)
                .is_some_and(|p| p.eq_ignore_ascii_case(word))
        };
        let (index, kind) = match (self.namespace, self.collection) {
            (Some(Namespace::Session | Namespace::Season), Some(_)) => {
                if is(5, "prizing") && is(6, "session") && parts.len() > 7 {
                    (7, ShowIdKind::Session)
                } else {
                    (5, ShowIdKind::Session)
                }
            }
            (Some(Namespace::Setup), Some(Collection::Store)) => {
                if is(5, "sessions") && is(7, "mergedlaunchdata") {
                    (6, ShowIdKind::Session)
                } else if is(6, "availablelanguages") {
                    (5, ShowIdKind::Episode)
                } else if is(5, "questionbank") && is(7, "locale") {
                    // `QuestionBank.<episode>.Locale....` (`messages.lua`: `~E` is the EpisodeID).
                    (6, ShowIdKind::Episode)
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        let segment = self.raw.split('.').nth(index)?;
        Some(ShowIdRef {
            index,
            kind,
            id: segment,
        })
    }

    /// The same key with segment `index` replaced by `value`, parsed again.
    pub fn with_segment(&self, index: usize, value: &str) -> ParsedKey {
        let mut parts: Vec<&str> = self.raw.split('.').collect();
        if let Some(part) = parts.get_mut(index) {
            *part = value;
        }
        ParsedKey::parse(&parts.join("."))
    }

    /// An election key with its election id replaced (`...PRIMETIME.<id>...`).
    pub fn with_election_id(&self, id: u32) -> ParsedKey {
        self.with_segment(7, &id.to_string())
    }

    pub fn parse(raw: &str) -> ParsedKey {
        let lower = raw.to_ascii_lowercase();
        let parts: Vec<&str> = lower.split('.').collect();
        let original: Vec<&str> = raw.split('.').collect();
        let namespace = match parts.as_slice() {
            ["v3", namespace, "mgs", "1vs100", ..] => match *namespace {
                "session" => Some(Namespace::Session),
                "season" => Some(Namespace::Season),
                "setup" => Some(Namespace::Setup),
                "stats" => Some(Namespace::Stats),
                "log" => Some(Namespace::Log),
                _ => None,
            },
            _ => None,
        };
        // A collection names something: `...1vs100.Sum` alone is not a sum (`...Sum.` is).
        let collection = match parts.as_slice() {
            [
                "v3",
                "session" | "season" | "stats" | "setup" | "log",
                "mgs",
                "1vs100",
                c,
                _,
                ..,
            ] => match *c {
                "store" => Some(Collection::Store),
                "sum" => Some(Collection::Sum),
                "topn" => Some(Collection::TopN),
                _ => None,
            },
            _ => None,
        };
        let kind = match parts.as_slice() {
            ["v3", "session", "mgs", "1vs100", rest @ ..] => session_key(rest, &original[4..]),
            ["v3", "season", "mgs", "1vs100", rest @ ..] => season_key(rest, &original[4..]),
            ["v3", "setup", "mgs", "1vs100", "store", rest @ ..] => setup_key(rest),
            ["v3", "stats", "mgs", "1vs100", "sum", ..] => Some(Key::StatsSum),
            ["v3", "stats", "mgs", "1vs100", "topn", ..] => Some(Key::StatsTopN),
            ["v3", "log", "mgs", "1vs100", "store", "urlogging"] => Some(Key::UrLogging),
            _ => None,
        };
        ParsedKey {
            raw: raw.to_owned(),
            namespace,
            collection,
            kind: kind.unwrap_or(Key::Other),
        }
    }
}

/// The key of `session`'s GameState (`v3.Session.MGS.1vs100.Store.<session>.GameState`).
pub fn game_state_key(session: &str) -> String {
    format!("v3.Session.MGS.1vs100.Store.{session}.GameState")
}

fn session(segment: &str) -> Option<SessionId> {
    (!segment.is_empty()).then(|| SessionId(segment.to_owned()))
}

fn number<T: std::str::FromStr>(segment: &str) -> Option<T> {
    segment.parse().ok()
}

/// `segment` without `prefix`, as a number (`bucket5` -> 5).
fn numbered<T: std::str::FromStr>(segment: &str, prefix: &str) -> Option<T> {
    segment.strip_prefix(prefix)?.parse().ok()
}

/// A round segment.
fn round(segment: &str) -> Option<Round> {
    number(segment).map(Round)
}

/// A question segment: the question is absent for the title's 0, the states before a round's
/// first question. A segment that is no number at all is no question key.
fn question(segment: &str) -> Option<Option<QuestionNumber>> {
    Some(QuestionNumber::new(number(segment)?))
}

/// `GameState<Name>` of a data-collection key: the name as sent.
fn data_collection_state(lower: &str, original: &str) -> Option<String> {
    let name = lower.strip_prefix("gamestate")?;
    (!name.is_empty()).then(|| original[original.len() - name.len()..].to_owned())
}

/// After `v3.Session.MGS.1vs100.`.
fn session_key(parts: &[&str], original: &[&str]) -> Option<Key> {
    Some(match parts {
        ["store", s, "gamestate"] => Key::GameState {
            session: session(s)?,
        },
        // Every key under an election id, known item or not (the election fold covers them all).
        [
            collection @ ("store" | "topn" | "sum"),
            s,
            "primetime",
            id,
            item @ ..,
        ] if !item.is_empty() && number::<u32>(id).is_some() => Key::Election {
            session: session(s)?,
            id: number(id)?,
            item: election_item(collection, item),
        },
        [
            "store",
            s,
            "primetime",
            "questionbank",
            "difficulty",
            d,
            list,
        ] => Key::SessionQuestionList {
            session: session(s)?,
            difficulty: number(d)?,
            list: match *list {
                "available" => QuestionList::Available,
                "used" => QuestionList::Used,
                _ => return None,
            },
        },
        ["store", s, "primetime", "round", r, "question", q, name] => Key::DataCollectionState {
            season: false,
            session: session(s)?,
            round: round(r)?,
            question: question(q)?,
            name: data_collection_state(name, original[7])?,
        },
        ["topn", s, "primetime", "round", r, "question", q, board] => Key::QuestionBoard {
            session: session(s)?,
            round: round(r)?,
            question: question(q)?,
            board: match *board {
                "mobchoices" => Board::MobChoices,
                "brain" => Board::Brain,
                "roundscores" => Board::RoundScores,
                _ => return None,
            },
        },
        ["sum", s, "primetime", "lobbyplayercount", secs] => Key::LobbyPlayerCount {
            session: session(s)?,
            secs: number(secs)?,
        },
        ["sum", s, "primetime", "round", r, "question", q, sum @ ..] => Key::QuestionSum {
            session: session(s)?,
            round: round(r)?,
            question: question(q)?,
            sum: match sum {
                ["crowd", choice] => QuestionSum::Crowd(Choice::new(numbered(choice, "choice")?)?),
                ["totalplayercount"] => QuestionSum::TotalPlayerCount,
                ["matchmakehost"] => QuestionSum::MatchMake(MatchMake::Host),
                ["matchmakejoin"] => QuestionSum::MatchMake(MatchMake::Join),
                ["matchmakeleave"] => QuestionSum::MatchMake(MatchMake::Leave),
                ["matchmakewait"] => QuestionSum::MatchMake(MatchMake::Wait),
                ["statbreaks", stat, bucket] => {
                    let stat = match *stat {
                        "numcorrectanswers" => StatBreak::NumCorrectAnswers,
                        "streak" => StatBreak::Streak,
                        _ => return None,
                    };
                    QuestionSum::StatBreak(stat.bucket(numbered(bucket, "bucket")?)?)
                }
                _ => return None,
            },
        },
        ["sum", "program", s, "qos", _, ..] => Key::Qos {
            session: session(s)?,
        },
        ["log", "urlogging"] => Key::UrLogging,
        ["log", "usererror", _, ..] => Key::UserError,
        _ => return None,
    })
}

fn election_item(collection: &str, item: &[&str]) -> ElectionItem {
    match (collection, item) {
        ("store", ["themob"]) => ElectionItem::TheMob,
        ("topn", ["theone"]) => ElectionItem::TheOne,
        ("topn", ["mobselection"]) => ElectionItem::MobSelection,
        ("topn", ["realavatars", bucket]) => {
            numbered(bucket, "bucket").map_or(ElectionItem::Other, ElectionItem::RealAvatars)
        }
        ("sum", ["rstotalplayercount"]) => ElectionItem::RsTotalPlayerCount,
        _ => ElectionItem::Other,
    }
}

/// After `v3.Season.MGS.1vs100.`.
fn season_key(parts: &[&str], original: &[&str]) -> Option<Key> {
    Some(match parts {
        ["userstore", "userstorage"] => Key::User(UserDoc::Storage),
        ["userstore", "userstats"] => Key::User(UserDoc::Stats),
        [
            "store",
            "prizing",
            "session",
            s,
            "xuid",
            _,
            "round",
            r,
            "one",
        ] => Key::PrizingOne {
            session: session(s)?,
            round: round(r)?,
        },
        [
            "topn",
            "prizing",
            "session",
            s,
            "round",
            r,
            "roundendtopscores",
        ] => Key::RoundEndTopScores {
            session: session(s)?,
            round: round(r)?,
        },
        ["topn", s, _game, "question", q, "roundscores"] => Key::ExtendedPlayRoundScores {
            session: session(s)?,
            question: question(q)?,
        },
        ["store", s, "primetime", "round", r, "question", q, name] => Key::DataCollectionState {
            season: true,
            session: session(s)?,
            round: round(r)?,
            question: question(q)?,
            name: data_collection_state(name, original[7])?,
        },
        _ => return None,
    })
}

/// After `v3.Setup.MGS.1vs100.Store.`.
fn setup_key(parts: &[&str]) -> Option<Key> {
    Some(match parts {
        ["config"] => Key::Config,
        ["config", week] => Key::ConfigWeek(numbered(week, "week")?),
        ["pangolin", "config"] => Key::PangolinConfig,
        [_, "availablelanguages"] => Key::AvailableLanguages,
        ["sessions", s, "mergedlaunchdata"] => Key::MergedLaunchData {
            session: session(s)?,
        },
        ["questionbank", "difficulty", d, entry @ ..] => question_bank(None, d, entry)?,
        [
            "questionbank",
            _,
            "locale",
            locale,
            "difficulty",
            d,
            entry @ ..,
        ] => question_bank(Some(locale), d, entry)?,
        _ => return None,
    })
}

fn question_bank(locale: Option<&str>, difficulty: &str, entry: &[&str]) -> Option<Key> {
    Some(Key::QuestionBank {
        locale: locale.map(str::to_owned),
        difficulty: number(difficulty)?,
        entry: match entry {
            ["resetid"] => BankEntry::ResetId,
            ["id", id] => BankEntry::Id(QuestionId(number(id)?)),
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "00000000-0000-0000-0000-000000000001";

    fn s() -> SessionId {
        SessionId(S.to_owned())
    }

    /// One real key of every shape in the LAN recordings of 2026-10-21..23.
    fn recorded() -> Vec<&'static str> {
        vec![
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.GameState",
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.132.TheMob",
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.QuestionBank.Difficulty.1.Available",
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.QuestionBank.Difficulty.1.Used",
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.0.GameStateIntroductions",
            "v3.Season.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.0.GameStateIntroductions",
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.132.TheOne",
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.132.MobSelection",
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.132.RealAvatars.Bucket1",
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.MobChoices",
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.Brain",
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.0.RoundScores",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.132.RSTotalPlayerCount",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.LobbyPlayerCount.15",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.Crowd.Choice3",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.TotalPlayerCount",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.MatchMakeHost",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.MatchMakeJoin",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.MatchMakeLeave",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.MatchMakeWait",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.StatBreaks.NumCorrectAnswers.Bucket5",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.7.StatBreaks.Streak.Bucket1",
            "v3.Session.MGS.1vs100.Sum.Program.00000000-0000-0000-0000-000000000001.QOS.132.Qualified",
            "v3.Session.MGS.1vs100.Sum.Program.00000000-0000-0000-0000-000000000001.QOS.132.RTT.1",
            "v3.Session.MGS.1vs100.Sum.Program.00000000-0000-0000-0000-000000000001.QOS.132.BandwidthUpstream.4",
            "v3.Session.MGS.1vs100.Sum.Program.00000000-0000-0000-0000-000000000001.QOS.132.BandwidthDownstream.4",
            "v3.Session.MGS.1vs100.Log.URLogging",
            "v3.Session.MGS.1vs100.Log.UserError.0",
            "v3.Session.MGS.1vs100.Log.UserError.Live_Host_Delay_1.25_ms",
            "v3.Log.MGS.1vs100.Store.URLogging",
            "v3.Season.MGS.1vs100.UserStore.UserStorage",
            "v3.Season.MGS.1vs100.UserStore.UserStats",
            "v3.Season.MGS.1vs100.Store.Prizing.Session.00000000-0000-0000-0000-000000000001.Xuid.16140907667073616695.Round.1.One",
            "v3.Season.MGS.1vs100.TopN.Prizing.Session.00000000-0000-0000-0000-000000000001.Round.1.RoundEndTopScores",
            "v3.Setup.MGS.1vs100.Store.Config",
            "v3.Setup.MGS.1vs100.Store.Config.Week6",
            "v3.Setup.MGS.1vs100.Store.Pangolin.Config",
            "v3.Setup.MGS.1vs100.Store.00000000-0000-0000-0000-000000000000.AvailableLanguages",
            "v3.Setup.MGS.1vs100.Store.Sessions.00000000-0000-0000-0000-000000000001.MergedLaunchData",
            "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-US.Difficulty.1.ResetID",
            "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale.en-US.Difficulty.1.ID.251317",
            "v3.Stats.MGS.1vs100.Sum.StatsUpdate.TotalSeen.251317",
            "v3.Stats.MGS.1vs100.Sum.StatsUpdate.TotalCorrect.251317",
            "v3.Stats.MGS.1vs100.Sum.StatsUpdate.TotalIncorrect.251401",
            "v3.Stats.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.HostTool.ArcadeGames",
            "v3.Stats.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.HostTool.MSPoints",
            "v3.Stats.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.HostTool.TopMobScore",
        ]
    }

    /// Every other letter flipped: the title is not consistent about case.
    fn scrambled(key: &str) -> String {
        key.chars()
            .enumerate()
            .map(|(i, c)| {
                if i % 2 == 0 {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect()
    }

    /// The kind with its as-sent parts lowercased: a data-collection name keeps its own case.
    fn ignoring_as_sent_name(kind: &Key) -> Key {
        let mut kind = kind.clone();
        if let Key::DataCollectionState { name, .. } = &mut kind {
            name.make_ascii_lowercase();
        }
        kind
    }

    /// Each key and what it parses to, one per line.
    fn shown(keys: &[&str]) -> String {
        keys.iter()
            .map(|raw| format!("{raw} = {:?}", ParsedKey::parse(raw)))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn recorded_keys_parse_to_their_kinds() {
        insta::assert_snapshot!(shown(&recorded()));
    }

    /// `STAT_EP_ROUND_TOP_SCORE` of `messages.lua`, the short question-bank form, and keys of
    /// other titles, versions or shapes that do not read.
    #[test]
    fn unrecorded_and_malformed_keys_parse_to_their_kinds() {
        let keys = [
            "v3.Season.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.EXTENDEDPLAY.Question.12.RoundScores",
            "v3.Setup.MGS.1vs100.Store.QuestionBank.Difficulty.1.ID.251317",
            "v3.Session.MGS.1vs200.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.693.MobSelection",
            "v1.Session.MGS.1vs100.00000000-0000-0000-0000-000000000001.GameState",
            "v3.Session.MGS.1vs100.Store..GameState",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.x.Question.1.Crowd.Choice1",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.Crowd.Choice4",
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.StatBreaks.Streak.Bucket5",
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.Unknown",
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.GameState",
            "",
        ];
        insta::assert_snapshot!(shown(&keys));
    }

    #[test]
    fn every_recorded_shape_parses_in_any_case() {
        for raw in recorded() {
            let lower = ParsedKey::parse(&raw.to_ascii_lowercase());
            for key in [scrambled(raw), raw.to_ascii_uppercase()] {
                let parsed = ParsedKey::parse(&key);
                assert_eq!(parsed.raw, key);
                assert_eq!(parsed.namespace, lower.namespace, "{key}");
                assert_eq!(parsed.collection, lower.collection, "{key}");
                assert_eq!(
                    ignoring_as_sent_name(&parsed.kind),
                    ignoring_as_sent_name(&lower.kind),
                    "{key}"
                );
            }
        }
    }

    #[test]
    fn every_key_of_every_recorded_session_has_a_kind() {
        let mut keys = 0;
        let mut unknown = std::collections::BTreeSet::new();
        for dir in [
            "fixtures/sessions",
            "fixtures/sessions/first-round-opens-twice",
        ] {
            for path in std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
            {
                if path.extension().is_none_or(|ext| ext != "sbg") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let recording = crate::sbg::record::parse(&text).unwrap();
                for event in &recording.events {
                    if event.kind != crate::sbg::record::Kind::In {
                        continue;
                    }
                    let Ok(Some((frame, _))) = crate::wire::decode(&event.frame) else {
                        continue;
                    };
                    if let Ok(super::super::message::Message::Request { key, .. }) =
                        super::super::message::decode(&frame)
                    {
                        keys += 1;
                        if ParsedKey::parse(&key).kind == Key::Other {
                            unknown.insert(key);
                        }
                    }
                }
            }
        }
        assert!(keys > 1000, "{keys} keys");
        assert!(unknown.is_empty(), "{unknown:#?}");
    }

    #[test]
    fn the_router_keeps_sums_and_boards_apart() {
        let parsed = |key: &str| ParsedKey::parse(key);
        assert!(parsed("v3.Season.MGS.1vs100.Sum.x").is_sum());
        assert!(parsed("v3.Stats.MGS.1vs100.Sum.StatsUpdate.TotalSeen.1").is_sum());
        assert!(!parsed("v3.Setup.MGS.1vs100.Sum.x").is_sum());
        assert!(
            parsed("v3.Season.MGS.1vs100.TopN.Prizing.Session.x.Round.1.RoundEndTopScores")
                .is_top_n()
        );
        assert!(
            !parsed("v3.Stats.MGS.1vs100.TopN.x.PRIMETIME.Round.1.Question.1.HostTool.TopMobScore")
                .is_top_n()
        );
        // An unknown key still names its collection: the router keeps sums and boards apart.
        let unknown = parsed(
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.Unknown",
        );
        assert_eq!(unknown.collection, Some(Collection::TopN));
    }

    #[test]
    fn any_item_under_an_election_id_is_an_election_and_keeps_its_id_on_rewrite() {
        let key = ParsedKey::parse(
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.387.Something.Else",
        );
        assert_eq!(
            key.kind,
            Key::Election {
                session: s(),
                id: 387,
                item: ElectionItem::Other
            }
        );
        let folded = key.with_election_id(381);
        assert_eq!(
            folded.raw,
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.381.Something.Else"
        );
        assert!(matches!(folded.kind, Key::Election { id: 381, .. }));
        // Not an election: a non-numeric id, or nothing after the id.
        for key in [
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.QuestionBank.Difficulty.1.Available",
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.387",
        ] {
            assert!(
                !matches!(ParsedKey::parse(key).kind, Key::Election { .. }),
                "{key}"
            );
        }
    }

    #[test]
    fn the_canonical_session_is_recognised_in_any_case() {
        let key = |session: &str| {
            ParsedKey::parse(&format!("v3.Session.MGS.1vs100.Store.{session}.GameState"))
        };
        let Key::GameState { session } = key(S).kind else {
            panic!()
        };
        assert!(session.is_canonical());
        let Key::GameState { session } = key("00000000-0000-0000-0000-000000000002").kind else {
            panic!()
        };
        assert!(!session.is_canonical());
        let Key::GameState { session } = key("ABCDEF00-0000-0000-0000-000000000001").kind else {
            panic!()
        };
        assert_eq!(session.as_str(), "abcdef00-0000-0000-0000-000000000001");
    }
}
