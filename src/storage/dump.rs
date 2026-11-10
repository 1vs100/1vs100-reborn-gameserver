//! The JSON dump: every backend's data in one backend-neutral document, for backups and for
//! moving data between backends.

use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use super::{BlobKind, Data, PlayerRecord, Result, StorageError, parse_xuid};
use crate::{
    ids::Xuid,
    programme::{
        self, PlannedShow, PrimetimeSlots, Programme, Show, ShowKind, SpecialEpisode, ThemeArtwork,
    },
    questions::{BankFile, QuestionBank},
    schedule::Mode,
    time::{LocalDate, LocalTime, Utc, Zone},
};

pub const DUMP_VERSION: u32 = 1;

/// Dump byte string: text when UTF-8, base64 otherwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Blob {
    Text { text: String },
    Base64 { base64: String },
}

impl Blob {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self::Text { text: text.into() },
            Err(_) => Self::Base64 {
                base64: STANDARD.encode(bytes),
            },
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        match self {
            Self::Text { text } => Ok(text.clone().into_bytes()),
            Self::Base64 { base64 } => STANDARD
                .decode(base64)
                .map_err(|error| StorageError::Base64 { error }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerDump {
    /// 16 hex digits, e.g. `e000069052e8ad19`.
    pub xuid: String,
    /// Absent for a player known only by their stored blobs.
    #[serde(default)]
    pub gamertag: Option<String>,
    #[serde(default)]
    pub first_seen: Option<i64>,
    #[serde(default)]
    pub last_seen: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Blob>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_storage: Option<Blob>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_stats: Option<Blob>,
}

/// The programme in a dump, as text (`America/New_York`, `20:00`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProgrammeDump {
    pub zone: String,
    pub primetime_at: String,
    pub primetime_minutes: u32,
    pub extended_play_minutes: u32,
    pub min_extended_play_minutes: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpecialDump {
    /// `YYYY-MM-DD`, in the programme's zone.
    pub date: String,
    /// `HH:MM`, the Primetime start it replaces; absent means the programme's first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    pub title: String,
    pub category: String,
    pub minutes: u32,
    /// `pt` or `ep`; absent means Primetime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork: Option<String>,
}

/// What is wrong with a show read from storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowProblem {
    /// No session or episode id.
    NoIds,
    /// A start or an end that is not a time.
    BadTime,
    /// An end that is not after the start.
    EndsBeforeStart,
    /// A special with no title or no category.
    SpecialWithoutTitle,
    /// A kind that is not `primetime`, `special` or `extended_play`.
    UnknownKind,
}

impl ShowProblem {
    pub fn describe(self) -> &'static str {
        match self {
            Self::NoIds => "needs a session and an episode id",
            Self::BadTime => "start or end is not a time",
            Self::EndsBeforeStart => "ends before it starts",
            Self::SpecialWithoutTitle => "a special needs a title and a category",
            Self::UnknownKind => "unknown kind",
        }
    }
}

/// A special's mode as stored: absent is Primetime.
fn special_mode(stored: Option<&str>) -> Result<Mode> {
    match stored {
        None => Ok(Mode::Primetime),
        Some(text) => Mode::parse(text).ok_or_else(|| StorageError::InvalidField {
            field: "mode",
            value: text.into(),
        }),
    }
}

fn artwork(stored: Option<&str>) -> Result<Option<ThemeArtwork>> {
    stored
        .map(|name| {
            ThemeArtwork::new(name).map_err(|_| StorageError::InvalidField {
                field: "artwork",
                value: name.into(),
            })
        })
        .transpose()
}

/// A materialised show in a dump; times as `2026-10-23T20:00:00Z`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShowDump {
    pub session_id: String,
    pub episode_id: String,
    pub start: String,
    pub end: String,
    /// `primetime`, `special` or `extended_play`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// A special's `pt` or `ep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// A special's theme artwork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork: Option<String>,
    #[serde(default)]
    pub manual: bool,
}

impl ShowDump {
    pub(super) fn from_show(show: &Show) -> Self {
        let (title, category, mode, artwork) = match &show.plan.kind {
            ShowKind::Special {
                title,
                category,
                mode,
                artwork,
            } => (
                Some(title.clone()),
                Some(category.clone()),
                Some(mode.as_str().to_owned()),
                artwork.as_ref().map(|art| art.as_str().to_owned()),
            ),
            _ => (None, None, None, None),
        };
        Self {
            session_id: show.session_id.clone(),
            episode_id: show.episode_id.clone(),
            start: show.plan.start.to_grid(),
            end: show.plan.end.to_grid(),
            kind: show.plan.kind.as_str().to_owned(),
            title,
            category,
            mode,
            artwork,
            manual: show.manual,
        }
    }

    /// The show, if every field is valid.
    pub fn to_show(&self) -> Result<Show> {
        let problem = |problem| StorageError::InvalidShow {
            session_id: self.session_id.clone(),
            problem,
        };
        if self.session_id.trim().is_empty() || self.episode_id.trim().is_empty() {
            return Err(problem(ShowProblem::NoIds));
        }
        let time = |text: &str| Utc::parse(text).ok_or(ShowProblem::BadTime);
        let (start, end) = (
            time(&self.start).map_err(problem)?,
            time(&self.end).map_err(problem)?,
        );
        if start >= end {
            return Err(problem(ShowProblem::EndsBeforeStart));
        }
        let kind = match (self.kind.as_str(), &self.title, &self.category) {
            ("primetime", ..) => ShowKind::Primetime,
            ("extended_play", ..) => ShowKind::ExtendedPlay,
            ("special", Some(title), Some(category)) => ShowKind::Special {
                title: title.clone(),
                category: category.clone(),
                mode: special_mode(self.mode.as_deref())?,
                artwork: artwork(self.artwork.as_deref())?,
            },
            ("special", ..) => return Err(problem(ShowProblem::SpecialWithoutTitle)),
            _ => return Err(problem(ShowProblem::UnknownKind)),
        };
        Ok(Show {
            session_id: self.session_id.clone(),
            episode_id: self.episode_id.clone(),
            plan: PlannedShow { start, end, kind },
            manual: self.manual,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dump {
    pub version: u32,
    pub exported_unix: i64,
    pub players: Vec<PlayerDump>,
    pub counters: BTreeMap<String, f64>,
    /// Absent when the dump holds no programme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub programme: Option<ProgrammeDump>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub specials: Vec<SpecialDump>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shows: Vec<ShowDump>,
    /// The imported question bank, as a bank file; absent while the built-in one is served.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub questions: Option<BankFile>,
}

impl ProgrammeDump {
    pub(super) fn from_programme(programme: &Programme) -> Self {
        Self {
            zone: programme.zone.name().to_owned(),
            primetime_at: programme.primetime_at.to_string(),
            primetime_minutes: programme::whole_minutes(programme.primetime),
            extended_play_minutes: programme::whole_minutes(programme.extended_play),
            min_extended_play_minutes: programme::whole_minutes(programme.min_extended_play),
        }
    }

    /// The programme, if every field is valid.
    pub fn to_programme(&self) -> Result<Programme> {
        let programme = Programme {
            zone: Zone::get(&self.zone).ok_or_else(|| StorageError::InvalidField {
                field: "zone",
                value: self.zone.clone(),
            })?,
            primetime_at: PrimetimeSlots::parse(&self.primetime_at).map_err(|_| {
                StorageError::InvalidField {
                    field: "primetime_at",
                    value: self.primetime_at.clone(),
                }
            })?,
            primetime: programme::minutes(self.primetime_minutes),
            extended_play: programme::minutes(self.extended_play_minutes),
            min_extended_play: programme::minutes(self.min_extended_play_minutes),
        };
        // A dump may hold 120-minute Extended Play shows: cut them to one episode.
        let (programme, _) = programme.with_one_episode_extended_play();
        programme.validate().map_err(StorageError::NotValid)?;
        Ok(programme)
    }
}

impl SpecialDump {
    pub(super) fn from_special(special: &SpecialEpisode) -> Self {
        Self {
            date: special.date.to_string(),
            at: Some(special.at.to_string()),
            title: special.title.clone(),
            category: special.category.clone(),
            minutes: programme::whole_minutes(special.length),
            mode: Some(special.mode.as_str().to_owned()),
            artwork: special.artwork.as_ref().map(|art| art.as_str().to_owned()),
        }
    }

    /// The special episode, if every field is valid. `first_at` stands in for an absent start.
    pub fn to_special(&self, first_at: Option<LocalTime>) -> Result<SpecialEpisode> {
        let date = LocalDate::parse(&self.date).ok_or_else(|| StorageError::InvalidField {
            field: "date",
            value: self.date.clone(),
        })?;
        let at = match &self.at {
            Some(at) => LocalTime::parse(at).ok_or_else(|| StorageError::InvalidField {
                field: "at",
                value: at.clone(),
            })?,
            None => first_at.ok_or(StorageError::SpecialWithoutTime { date })?,
        };
        let special = SpecialEpisode {
            date,
            at,
            title: self.title.clone(),
            category: self.category.clone(),
            length: programme::minutes(self.minutes),
            mode: special_mode(self.mode.as_deref())?,
            artwork: artwork(self.artwork.as_deref())?,
        };
        special.validate().map_err(StorageError::NotValid)?;
        Ok(special)
    }
}

/// A dumped seen time in unix seconds, refused when it is past what a time holds.
fn seen_time(field: &'static str, secs: i64) -> Result<Utc> {
    secs.checked_mul(1_000)
        .map(Utc::from_unix_ms)
        .ok_or_else(|| StorageError::InvalidField {
            field,
            value: secs.to_string(),
        })
}

impl Data {
    pub fn to_dump(&self, now: Utc) -> Dump {
        let mut xuids: Vec<Xuid> = self.players.keys().copied().collect();
        for (_, xuid) in self.blobs.keys() {
            if !xuids.contains(xuid) {
                xuids.push(*xuid);
            }
        }
        xuids.sort_unstable();
        let players = xuids
            .into_iter()
            .map(|xuid| {
                let record = self.players.get(&xuid);
                let blob = |kind| self.blobs.get(&(kind, xuid)).map(|b| Blob::from_bytes(b));
                PlayerDump {
                    xuid: format!("{xuid:016x}"),
                    gamertag: record.and_then(|r| r.gamertag.clone()),
                    first_seen: record.map(|r| r.first_seen.unix_secs()),
                    last_seen: record.map(|r| r.last_seen.unix_secs()),
                    avatar: record
                        .and_then(|r| r.avatar.as_deref())
                        .map(Blob::from_bytes),
                    user_storage: blob(BlobKind::UserStorage),
                    user_stats: blob(BlobKind::UserStats),
                }
            })
            .collect();
        Dump {
            version: DUMP_VERSION,
            exported_unix: now.unix_secs(),
            players,
            counters: self.counters.clone(),
            programme: self.programme.as_ref().map(ProgrammeDump::from_programme),
            specials: self
                .specials
                .values()
                .map(SpecialDump::from_special)
                .collect(),
            shows: self
                .shows_by_start()
                .iter()
                .map(ShowDump::from_show)
                .collect(),
            questions: (!self.questions.is_empty()).then(|| self.questions.to_file()),
        }
    }

    pub fn from_dump(dump: &Dump) -> Result<Self> {
        if dump.version != DUMP_VERSION {
            return Err(StorageError::UnsupportedVersion {
                found: dump.version.into(),
                supported: DUMP_VERSION.into(),
            });
        }
        let mut data = Self::default();
        for player in &dump.players {
            let xuid = parse_xuid(&player.xuid)?;
            // An older export wrote "" and 0 for a player known only by their blobs: read them as
            // absent.
            let gamertag = player
                .gamertag
                .clone()
                .filter(|gamertag| !gamertag.is_empty());
            let first_seen = player.first_seen.filter(|seen| *seen != 0);
            let last_seen = player.last_seen.filter(|seen| *seen != 0);
            match (first_seen, last_seen) {
                (Some(first_seen), Some(last_seen)) => {
                    data.players.insert(
                        xuid,
                        PlayerRecord {
                            xuid,
                            gamertag,
                            first_seen: seen_time("first_seen", first_seen)?,
                            last_seen: seen_time("last_seen", last_seen)?,
                            avatar: player.avatar.as_ref().map(Blob::to_bytes).transpose()?,
                        },
                    );
                }
                (None, None) if gamertag.is_none() && player.avatar.is_none() => {}
                _ => return Err(StorageError::IncompletePlayer { xuid }),
            }
            for (kind, blob) in [
                (BlobKind::UserStorage, &player.user_storage),
                (BlobKind::UserStats, &player.user_stats),
            ] {
                if let Some(blob) = blob {
                    data.blobs.insert((kind, xuid), blob.to_bytes()?);
                }
            }
        }
        for (key, value) in &dump.counters {
            if !value.is_finite() {
                return Err(StorageError::NonFinite { key: key.clone() });
            }
        }
        data.counters = dump.counters.clone();
        data.programme = dump
            .programme
            .as_ref()
            .map(ProgrammeDump::to_programme)
            .transpose()?;
        let first_at = data
            .programme
            .as_ref()
            .map(|programme| programme.primetime_at.first());
        for special in &dump.specials {
            let special = special.to_special(first_at)?;
            data.specials.insert(special.slot(), special);
        }
        for show in &dump.shows {
            let show = show.to_show()?;
            data.shows.insert(show.session_id.clone(), show);
        }
        if let Some(file) = &dump.questions {
            data.questions = QuestionBank::from_file(file.clone())
                .map_err(|source| StorageError::InvalidBank { source })?;
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::questions::{BANK_FILE_VERSION, Category, Choice, Difficulty, Question, Source};
    use std::collections::BTreeMap;

    fn questions() -> BankFile {
        BankFile {
            version: BANK_FILE_VERSION,
            sources: vec![Source {
                id: "otdb".into(),
                name: "Open Trivia Database".into(),
                licence: "CC BY-SA 4.0".into(),
                url: "https://opentdb.com".into(),
            }],
            categories: vec![Category {
                id: "science-nature".into(),
                name: "Science & Nature".into(),
            }],
            questions: vec![Question {
                id: crate::ids::QuestionId(251_317),
                locale: crate::questions::LOCALE.into(),
                difficulty: Difficulty::Medium,
                text: "Which planet is red?".into(),
                choices: ["Earth".into(), "Mars".into(), "Venus".into()],
                correct: Choice::B,
                categories: vec!["science-nature".into()],
                source: "otdb".into(),
                origin: Default::default(),
            }],
        }
    }

    fn dump() -> Dump {
        Dump {
            version: DUMP_VERSION,
            exported_unix: 1_792_715_141,
            players: vec![
                PlayerDump {
                    xuid: "e000069052e8ad19".into(),
                    gamertag: Some("Player1".into()),
                    first_seen: Some(100),
                    last_seen: Some(200),
                    avatar: Some(Blob::from_bytes(b"\x89PNG")),
                    user_storage: Some(Blob::from_bytes(b"<XsorObject />")),
                    user_stats: None,
                },
                // A player known only by their blobs, as an older export wrote them.
                PlayerDump {
                    xuid: "e0000766484d4b37".into(),
                    gamertag: Some(String::new()),
                    first_seen: Some(0),
                    last_seen: Some(0),
                    avatar: None,
                    user_storage: Some(Blob::from_bytes(b"<XsorObject />")),
                    user_stats: None,
                },
            ],
            counters: BTreeMap::from([(
                "v3.Stats.MGS.1vs100.Sum.StatsUpdate.TotalSeen.251317".into(),
                3.0,
            )]),
            programme: Some(ProgrammeDump {
                zone: "America/New_York".into(),
                primetime_at: "20:00".into(),
                primetime_minutes: 60,
                extended_play_minutes: 120,
                min_extended_play_minutes: 45,
            }),
            specials: vec![SpecialDump {
                date: "2026-10-24".into(),
                // No start: the programme's first.
                at: None,
                title: "Movie Night".into(),
                category: "otdb:11".into(),
                minutes: 30,
                mode: Some("ep".into()),
                artwork: Some("Community".into()),
            }],
            shows: vec![ShowDump {
                session_id: "session".into(),
                episode_id: "episode".into(),
                start: "2026-10-23T20:00:00Z".into(),
                end: "2026-10-23T21:00:00Z".into(),
                kind: "special".into(),
                title: Some("Movie Night".into()),
                category: Some("otdb:11".into()),
                mode: Some("pt".into()),
                artwork: None,
                manual: false,
            }],
            questions: Some(questions()),
        }
    }

    #[test]
    fn a_dump_imports_to_the_data_it_holds() {
        insta::assert_debug_snapshot!(Data::from_dump(&dump()).unwrap());
    }

    /// The errors of a backup of another version, a bad counter, an unreadable show, a bank of
    /// another version, a player record without both seen times and a seen time past what a
    /// time holds.
    #[test]
    fn a_dump_this_server_cannot_import_names_what_is_wrong() {
        let other_version = Dump {
            version: DUMP_VERSION + 1,
            ..dump()
        };
        let mut not_finite = dump();
        not_finite.counters.insert("bad".into(), f64::NAN);
        let mut unknown_show = dump();
        unknown_show.shows[0].kind = "challenge".into();
        let mut bad_bank = dump();
        bad_bank.questions = Some(BankFile {
            version: BANK_FILE_VERSION + 1,
            ..questions()
        });
        let mut half_seen = dump();
        half_seen.players[0].last_seen = None;
        let mut too_late = dump();
        too_late.players[0].first_seen = Some(i64::MAX);
        let shown = [
            Data::from_dump(&other_version).unwrap_err(),
            Data::from_dump(&not_finite).unwrap_err(),
            Data::from_dump(&unknown_show).unwrap_err(),
            Data::from_dump(&bad_bank).unwrap_err(),
            Data::from_dump(&half_seen).unwrap_err(),
            Data::from_dump(&too_late).unwrap_err(),
        ];
        insta::assert_debug_snapshot!(shown);
    }
}
