//! Persistent data behind a small storage trait: only what must outlive a restart.
//! Session state stays in memory; reloading it would resurrect stale rounds.
//! Synchronous; [`MemoryStorage`] backs tests, [`SqliteStorage`] is the default and
//! [`PostgresStorage`] serves a shared database.

mod dump;
mod memory;
mod postgres;
mod sqlite;

use std::{collections::BTreeMap, fmt, net::IpAddr};

use serde::{Deserialize, Serialize};

pub use dump::{
    Blob, DUMP_VERSION, Dump, PlayerDump, ProgrammeDump, ShowDump, ShowProblem, SpecialDump,
};
pub use memory::MemoryStorage;
pub use postgres::{PostgresStorage, PostgresUrl};
pub use sqlite::SqliteStorage;

use crate::{
    ids::Xuid,
    programme::{Programme, Show, SpecialEpisode, SpecialSlot, ValidationError},
    questions::{BankError, QuestionBank},
    time::{LocalDate, Utc},
};

pub type Result<T> = std::result::Result<T, StorageError>;

/// Why a storage call failed.
#[derive(Debug)]
pub enum StorageError {
    /// The SQLite call failed.
    Sqlite(rusqlite::Error),
    /// The PostgreSQL call failed.
    Postgres(::postgres::Error),
    /// Reading or writing a file failed.
    Io(std::io::Error),
    /// A stored document is a version this server does not read, and the version it does.
    UnsupportedVersion { found: i64, supported: i64 },
    /// A counter total that is not a finite number; the counter's key.
    NonFinite { key: String },
    /// A field of a stored document that this server cannot read; the field and its value.
    InvalidField { field: &'static str, value: String },
    /// A show this server cannot read; the show and what is wrong with it.
    InvalidShow {
        session_id: String,
        problem: ShowProblem,
    },
    /// A programme or special episode that does not pass its own checks.
    NotValid(ValidationError),
    /// A question bank that does not pass its own checks.
    InvalidBank { source: BankError },
    /// A blob that is not the base64 it is marked as.
    Base64 { error: base64::DecodeError },
    /// A dumped player with a gamertag, an avatar or one seen time, but not both seen times.
    IncompletePlayer { xuid: Xuid },
    /// A dumped special without a Primetime start, in a dump without a programme to take the
    /// first one from; the special's date.
    SpecialWithoutTime { date: LocalDate },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("storage: ")?;
        match self {
            Self::Sqlite(error) => write!(f, "{error}"),
            Self::Postgres(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::UnsupportedVersion { found, supported } => {
                write!(
                    f,
                    "unsupported version {found} (this server reads version {supported})"
                )
            }
            Self::NonFinite { key } => write!(f, "counter {key:?} is not finite"),
            Self::InvalidField { field, value } => write!(f, "invalid {field} {value:?}"),
            Self::InvalidShow {
                session_id,
                problem,
            } => write!(f, "show {session_id:?}: {}", problem.describe()),
            Self::NotValid(error) => error.fmt(f),
            Self::InvalidBank { source } => write!(f, "question bank: {source}"),
            Self::Base64 { error } => write!(f, "not base64: {error}"),
            Self::IncompletePlayer { xuid } => {
                write!(f, "player {xuid:016x} has a record without both seen times")
            }
            Self::SpecialWithoutTime { date } => write!(
                f,
                "the special on {date} has no Primetime start and there is no programme to take one from"
            ),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Postgres(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::NotValid(error) => Some(error),
            Self::InvalidBank { source } => Some(source),
            Self::Base64 { error } => Some(error),
            Self::UnsupportedVersion { .. }
            | Self::NonFinite { .. }
            | Self::InvalidField { .. }
            | Self::InvalidShow { .. }
            | Self::IncompletePlayer { .. }
            | Self::SpecialWithoutTime { .. } => None,
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<::postgres::Error> for StorageError {
    fn from(error: ::postgres::Error) -> Self {
        Self::Postgres(error)
    }
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Per-player title documents (raw XML, opaque).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlobKind {
    UserStorage,
    UserStats,
}

impl BlobKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::UserStorage => "user_storage",
            Self::UserStats => "user_stats",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "user_storage" => Some(Self::UserStorage),
            "user_stats" => Some(Self::UserStats),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerRecord {
    pub xuid: Xuid,
    /// Absent until the title reports a name.
    pub gamertag: Option<String>,
    pub first_seen: Utc,
    pub last_seen: Utc,
    /// `XamAvatarGetMetadataLocalUser` blob of the player's last session.
    pub avatar: Option<Vec<u8>>,
}

/// A player as the admin's list shows it: the sizes of what is stored, not the bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerSummary {
    pub xuid: Xuid,
    /// Absent until the title reports a name.
    pub gamertag: Option<String>,
    pub first_seen: Utc,
    pub last_seen: Utc,
    /// Absent until the title uploads an avatar.
    pub avatar_bytes: Option<usize>,
    pub user_storage_bytes: usize,
    pub user_stats_bytes: usize,
}

/// The number of a saved error report; later reports have larger numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ErrorReportId(pub i64);

/// A console's error report (`Log.UserError`) as it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorReport {
    pub received: Utc,
    pub console: IpAddr,
    pub key: String,
    /// The posted document, unparsed, so a report in an unknown format is kept as well.
    pub document: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedErrorReport {
    pub id: ErrorReportId,
    pub report: ErrorReport,
}

/// How an import treats data already present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    /// Upsert players and blobs, set counters; keep everything not in the dump.
    Merge,
    /// Delete all persistent data first, then load the dump.
    Replace,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct ImportSummary {
    pub players: usize,
    pub blobs: usize,
    pub counters: usize,
    /// Whether the dump carried a programme (which then replaced the stored one).
    pub programme: bool,
    pub specials: usize,
    pub shows: usize,
    pub questions: usize,
}

pub trait Storage: Send + Sync {
    /// Register or refresh a player seen joining (keeps `first_seen`, updates a known name and
    /// `last_seen`).
    fn touch_player(&self, xuid: Xuid, gamertag: Option<&str>, now: Utc) -> Result<()>;
    /// Remember a player's latest avatar metadata (creates the player when unknown).
    fn set_avatar(&self, xuid: Xuid, avatar: &[u8], now: Utc) -> Result<()>;
    fn players(&self) -> Result<Vec<PlayerRecord>>;
    /// Every player with the sizes of their avatar and documents, by xuid; reads no blob.
    fn player_summaries(&self) -> Result<Vec<PlayerSummary>>;
    /// Remove a player and their blobs. `true` when something was removed.
    fn delete_player(&self, xuid: Xuid) -> Result<bool>;

    fn blob(&self, kind: BlobKind, xuid: Xuid) -> Result<Option<Vec<u8>>>;
    /// The gamertag and `kind` document of every player who has one; reads no avatar.
    fn players_with_blob(&self, kind: BlobKind) -> Result<Vec<(String, Vec<u8>)>>;
    fn put_blob(&self, kind: BlobKind, xuid: Xuid, data: &[u8], now: Utc) -> Result<()>;

    /// Add to a global counter and return the new total.
    fn counter_add(&self, key: &str, delta: f64) -> Result<f64>;
    fn counter(&self, key: &str) -> Result<Option<f64>>;

    /// The saved programme; `None` until one is set (`server::run` seeds it from the settings).
    fn programme(&self) -> Result<Option<Programme>>;
    fn set_programme(&self, programme: &Programme) -> Result<()>;
    /// Every saved special episode, by date and time.
    fn specials(&self) -> Result<Vec<SpecialEpisode>>;
    /// Save a special episode, replacing one in the same slot.
    fn put_special(&self, special: &SpecialEpisode) -> Result<()>;
    /// `true` when there was one in `slot`.
    fn delete_special(&self, slot: SpecialSlot) -> Result<bool>;

    /// Every materialised show, by start.
    fn shows(&self) -> Result<Vec<Show>>;
    /// Save shows, replacing ones with the same session id.
    fn put_shows(&self, shows: &[Show]) -> Result<()>;
    /// Drop the shows that start at or after `start` (re-planning); how many there were.
    fn delete_shows_from(&self, start: Utc) -> Result<usize>;
    /// Drop the shows that ended before `end` (history); how many there were.
    fn delete_shows_ended_before(&self, end: Utc) -> Result<usize>;

    /// The imported question bank; empty until one is imported (the built-in one is served then).
    fn question_bank(&self) -> Result<QuestionBank>;
    /// Merge `bank` into the saved one (by id), or replace it.
    fn import_questions(&self, bank: &QuestionBank, mode: ImportMode) -> Result<()>;

    /// Keep a console's error report, and only the newest `keep` reports. A report never
    /// replaces another one of the same name.
    fn add_error_report(&self, report: &ErrorReport, keep: usize) -> Result<()>;
    /// The newest `limit` error reports, newest first.
    fn error_reports(&self, limit: usize) -> Result<Vec<SavedErrorReport>>;
    /// Remove every error report; how many there were.
    fn clear_error_reports(&self) -> Result<usize>;

    /// Every saved item except the error reports, which are diagnostics and not part of a dump.
    fn export(&self) -> Result<Dump>;
    /// Error reports are kept whatever the mode.
    fn import(&self, dump: &Dump, mode: ImportMode) -> Result<ImportSummary>;
}

fn parse_console(text: &str) -> Result<IpAddr> {
    text.parse().map_err(|_| StorageError::InvalidField {
        field: "console",
        value: text.into(),
    })
}

fn parse_xuid(text: &str) -> Result<Xuid> {
    u64::from_str_radix(text, 16)
        .map(Xuid)
        .map_err(|_| StorageError::InvalidField {
            field: "xuid",
            value: text.into(),
        })
}

/// Plain in-memory form every backend can load from and flush to; export/import go through it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub players: BTreeMap<Xuid, PlayerRecord>,
    pub blobs: BTreeMap<(BlobKind, Xuid), Vec<u8>>,
    pub counters: BTreeMap<String, f64>,
    pub programme: Option<Programme>,
    pub specials: BTreeMap<SpecialSlot, SpecialEpisode>,
    /// By session id.
    pub shows: BTreeMap<String, Show>,
    pub questions: QuestionBank,
}

impl Data {
    fn shows_by_start(&self) -> Vec<Show> {
        let mut shows: Vec<_> = self.shows.values().cloned().collect();
        shows.sort_by_key(|show| show.plan.start);
        shows
    }
}

impl Data {
    fn apply_touch(&mut self, xuid: Xuid, gamertag: Option<&str>, now: Utc) {
        // Seen times are kept to the second, as every backend stores them.
        let now = now.trunc_secs();
        let record = self.players.entry(xuid).or_insert_with(|| PlayerRecord {
            xuid,
            gamertag: None,
            first_seen: now,
            last_seen: now,
            avatar: None,
        });
        if let Some(gamertag) = gamertag {
            record.gamertag = Some(gamertag.into());
        }
        record.last_seen = now;
    }

    fn merge(&mut self, other: Data) -> ImportSummary {
        let summary = ImportSummary {
            players: other.players.len(),
            blobs: other.blobs.len(),
            counters: other.counters.len(),
            programme: other.programme.is_some(),
            specials: other.specials.len(),
            shows: other.shows.len(),
            questions: other.questions.questions().len(),
        };
        self.players.extend(other.players);
        self.blobs.extend(other.blobs);
        self.counters.extend(other.counters);
        if other.programme.is_some() {
            self.programme = other.programme;
        }
        self.specials.extend(other.specials);
        self.shows.extend(other.shows);
        self.questions.merge(other.questions);
        summary
    }
}

fn finite_total(key: &str, total: f64) -> Result<f64> {
    if total.is_finite() {
        Ok(total)
    } else {
        Err(StorageError::NonFinite { key: key.into() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ids::QuestionId,
        programme::{PlannedShow, PrimetimeSlots, ShowKind},
    };

    const A: Xuid = Xuid(0xE000_0690_52E8_AD19);
    const B: Xuid = Xuid(0xE000_07FF_484D_4B37);

    /// PostgreSQL among them when `GAMESERVER_TEST_DATABASE_URL` names a database.
    fn backends() -> Vec<(&'static str, Box<dyn Storage>)> {
        let mut backends: Vec<(&'static str, Box<dyn Storage>)> = vec![
            ("memory", Box::new(MemoryStorage::default())),
            ("sqlite", Box::new(SqliteStorage::open_in_memory().unwrap())),
        ];
        if let Some(postgres) = postgres::test_storage() {
            backends.push(("postgres", Box::new(postgres)));
        }
        backends
    }

    /// The error the call failed with.
    fn failed<T>(result: Result<T>) -> StorageError {
        match result {
            Ok(_) => panic!("the call should have failed"),
            Err(error) => error,
        }
    }

    #[test]
    fn player_summaries_are_the_players_with_their_sizes() {
        for (name, s) in backends() {
            s.touch_player(
                Xuid(0xE000_0690_52E8_AD19),
                Some("Full"),
                Utc::from_unix_secs(10),
            )
            .unwrap();
            s.set_avatar(
                Xuid(0xE000_0690_52E8_AD19),
                &[1; 300],
                Utc::from_unix_secs(11),
            )
            .unwrap();
            s.put_blob(
                BlobKind::UserStorage,
                Xuid(0xE000_0690_52E8_AD19),
                b"<doc/>",
                Utc::from_unix_secs(12),
            )
            .unwrap();
            s.put_blob(
                BlobKind::UserStats,
                Xuid(0xE000_0690_52E8_AD19),
                &[0; 40],
                Utc::from_unix_secs(13),
            )
            .unwrap();
            s.touch_player(Xuid(0x12), Some("Stats only"), Utc::from_unix_secs(20))
                .unwrap();
            s.put_blob(
                BlobKind::UserStats,
                Xuid(0x12),
                &[0; 7],
                Utc::from_unix_secs(21),
            )
            .unwrap();
            s.touch_player(
                Xuid(0x0A00_0000_0000_0001),
                Some("Nothing"),
                Utc::from_unix_secs(30),
            )
            .unwrap();
            // Every player, then each blob fetched.
            let expected: Vec<PlayerSummary> = s
                .players()
                .unwrap()
                .into_iter()
                .map(|player| {
                    let size = |kind| s.blob(kind, player.xuid).unwrap().map_or(0, |b| b.len());
                    PlayerSummary {
                        xuid: player.xuid,
                        avatar_bytes: player.avatar.as_ref().map(Vec::len),
                        user_storage_bytes: size(BlobKind::UserStorage),
                        user_stats_bytes: size(BlobKind::UserStats),
                        gamertag: player.gamertag,
                        first_seen: player.first_seen,
                        last_seen: player.last_seen,
                    }
                })
                .collect();
            let summaries = s.player_summaries().unwrap();
            assert_eq!(summaries, expected, "{name}");
            let xuids: Vec<Xuid> = summaries.iter().map(|p| p.xuid).collect();
            assert_eq!(
                xuids,
                [Xuid(0x12), Xuid(0x0A00_0000_0000_0001), A],
                "{name}: by xuid"
            );
            assert_eq!(
                (
                    summaries[2].avatar_bytes,
                    summaries[2].user_storage_bytes,
                    summaries[2].user_stats_bytes
                ),
                (Some(300), 6, 40),
                "{name}"
            );
        }
    }

    #[test]
    fn players_keep_first_seen_and_update_name_and_avatar() {
        for (name, s) in backends() {
            s.touch_player(A, Some("Player1"), Utc::from_unix_secs(100))
                .unwrap();
            s.touch_player(A, Some("Renamed"), Utc::from_unix_secs(200))
                .unwrap();
            s.touch_player(A, None, Utc::from_unix_secs(300)).unwrap(); // no name never erases the known one
            s.set_avatar(A, &[1, 2, 3], Utc::from_unix_secs(400))
                .unwrap();
            s.set_avatar(B, &[9], Utc::from_unix_secs(500)).unwrap(); // avatar for a player not seen joining yet
            let mut players = s.players().unwrap();
            players.sort_by_key(|p| p.xuid);
            assert_eq!(players.len(), 2, "{name}");
            let a = players.iter().find(|p| p.xuid == A).unwrap();
            assert_eq!(
                (
                    a.gamertag.as_deref(),
                    a.first_seen,
                    a.last_seen,
                    a.avatar.as_deref()
                ),
                (
                    Some("Renamed"),
                    Utc::from_unix_secs(100),
                    Utc::from_unix_secs(400),
                    Some(&[1u8, 2, 3][..])
                ),
                "{name}"
            );
            let b = players.iter().find(|p| p.xuid == B).unwrap();
            assert_eq!(
                (b.gamertag.as_deref(), b.first_seen),
                (None, Utc::from_unix_secs(500)),
                "{name}"
            );
        }
    }

    #[test]
    fn blobs_round_trip_per_player_and_kind_and_delete_with_the_player() {
        for (name, s) in backends() {
            assert_eq!(s.blob(BlobKind::UserStorage, A).unwrap(), None, "{name}");
            s.put_blob(BlobKind::UserStorage, A, b"<one/>", Utc::from_unix_secs(1))
                .unwrap();
            s.put_blob(BlobKind::UserStorage, A, b"<two/>", Utc::from_unix_secs(2))
                .unwrap();
            s.put_blob(BlobKind::UserStats, A, b"<stats/>", Utc::from_unix_secs(2))
                .unwrap();
            s.put_blob(BlobKind::UserStorage, B, b"<b/>", Utc::from_unix_secs(2))
                .unwrap();
            assert_eq!(
                s.blob(BlobKind::UserStorage, A).unwrap().unwrap(),
                b"<two/>",
                "{name}"
            );
            assert_eq!(
                s.blob(BlobKind::UserStats, A).unwrap().unwrap(),
                b"<stats/>",
                "{name}"
            );
            assert!(s.delete_player(A).unwrap(), "{name}");
            assert!(!s.delete_player(A).unwrap(), "{name}");
            assert_eq!(s.blob(BlobKind::UserStorage, A).unwrap(), None, "{name}");
            assert_eq!(
                s.blob(BlobKind::UserStorage, B).unwrap().unwrap(),
                b"<b/>",
                "{name}"
            );
        }
    }

    #[test]
    fn players_with_a_document_are_listed_with_it() {
        for (name, s) in backends() {
            s.touch_player(A, Some("Alpha"), Utc::from_unix_secs(1))
                .unwrap();
            s.touch_player(B, Some("Bravo"), Utc::from_unix_secs(1))
                .unwrap();
            s.put_blob(BlobKind::UserStorage, A, b"<a/>", Utc::from_unix_secs(1))
                .unwrap();
            s.put_blob(BlobKind::UserStats, B, b"<b/>", Utc::from_unix_secs(1))
                .unwrap();
            // A document without a player record has no gamertag to list.
            s.put_blob(
                BlobKind::UserStorage,
                Xuid(0xC),
                b"<c/>",
                Utc::from_unix_secs(1),
            )
            .unwrap();
            assert_eq!(
                s.players_with_blob(BlobKind::UserStorage).unwrap(),
                [("Alpha".to_owned(), b"<a/>".to_vec())],
                "{name}"
            );
        }
    }

    #[test]
    fn every_error_report_is_kept_newest_first_until_cleared() {
        for (name, s) in backends() {
            let report = |ms, console: &str, document: &[u8]| ErrorReport {
                received: Utc::from_unix_ms(ms),
                console: console.parse().unwrap(),
                key: "v3.Session.MGS.1vs100.Log.UserError.LobbyFPP".into(),
                document: document.to_vec(),
            };
            let reports = [
                report(1_500, "192.0.2.1", b"<a/>"),
                report(2_250, "2001:db8::1", b"<b/>"),
                report(2_250, "192.0.2.1", b"not a document"),
            ];
            for report in &reports {
                s.add_error_report(report, 10).unwrap();
            }
            let saved = s.error_reports(10).unwrap();
            let listed: Vec<&ErrorReport> = saved.iter().map(|saved| &saved.report).collect();
            assert_eq!(listed, reports.iter().rev().collect::<Vec<_>>(), "{name}");
            assert!(
                saved.windows(2).all(|pair| pair[0].id > pair[1].id),
                "{name}"
            );
            assert_eq!(s.error_reports(1).unwrap(), saved[..1], "{name}");
            s.import(
                &MemoryStorage::default().export().unwrap(),
                ImportMode::Replace,
            )
            .unwrap();
            assert_eq!(
                s.error_reports(10).unwrap(),
                saved,
                "{name}: kept by an import"
            );
            assert_eq!(s.clear_error_reports().unwrap(), 3, "{name}");
            assert_eq!(s.error_reports(10).unwrap(), [], "{name}");
            for report in &reports {
                s.add_error_report(report, 2).unwrap();
            }
            let kept = s.error_reports(10).unwrap();
            let listed: Vec<&ErrorReport> = kept.iter().map(|saved| &saved.report).collect();
            assert_eq!(listed, [&reports[2], &reports[1]], "{name}: the newest two");
            assert!(kept[1].id > saved[0].id, "{name}: an id is never reused");
        }
    }

    #[test]
    fn counters_accumulate_and_reject_non_finite_totals() {
        for (name, s) in backends() {
            assert_eq!(s.counter("k").unwrap(), None, "{name}");
            assert_eq!(s.counter_add("k", 1.0).unwrap(), 1.0, "{name}");
            assert_eq!(s.counter_add("k", 2.5).unwrap(), 3.5, "{name}");
            assert_eq!(s.counter("k").unwrap(), Some(3.5), "{name}");
            let error = failed(s.counter_add("k", f64::INFINITY));
            assert!(
                matches!(&error, StorageError::NonFinite { key } if key == "k"),
                "{name}: {error:?}"
            );
            assert_eq!(s.counter("k").unwrap(), Some(3.5), "{name}");
        }
    }

    fn populate(s: &dyn Storage) {
        s.touch_player(A, Some("Player1"), Utc::from_unix_secs(100))
            .unwrap();
        s.set_avatar(A, &[0xFF, 0xFE, 0x00, 0x80], Utc::from_unix_secs(110))
            .unwrap(); // not UTF-8 -> base64 in the dump
        s.put_blob(
            BlobKind::UserStorage,
            A,
            "<UserStorage name=\"ü\"/>".as_bytes(),
            Utc::from_unix_secs(120),
        )
        .unwrap();
        s.put_blob(BlobKind::UserStats, B, b"<s/>", Utc::from_unix_secs(130))
            .unwrap(); // blob without a player row
        s.counter_add("v3.Stats.Sum.TotalSeen.251317", 4.0).unwrap();
        s.set_programme(&Programme {
            primetime_at: PrimetimeSlots::parse("21:30").unwrap(),
            ..Programme::default()
        })
        .unwrap();
        s.put_special(&halloween()).unwrap();
        s.put_shows(&[show("a", "2026-10-31T23:00:00Z", 60, special_kind())])
            .unwrap();
    }

    fn special_kind() -> ShowKind {
        ShowKind::Special {
            title: "Halloween".into(),
            category: "otdb:11".into(),
            mode: crate::schedule::Mode::ExtendedPlay,
            artwork: Some(crate::programme::ThemeArtwork::new("Community").unwrap()),
        }
    }

    fn show(id: &str, start: &str, minutes: u64, kind: ShowKind) -> Show {
        let start = Utc::parse(start).unwrap();
        Show {
            session_id: format!("session-{id}"),
            episode_id: format!("episode-{id}"),
            plan: PlannedShow {
                start,
                end: start + std::time::Duration::from_secs(minutes * 60),
                kind,
            },
            manual: false,
        }
    }

    #[test]
    fn materialised_shows_are_saved_by_start_and_pruned() {
        for (name, s) in backends() {
            let a = show("a", "2026-10-23T12:00:00Z", 120, ShowKind::ExtendedPlay);
            let b = show("b", "2026-10-23T14:00:00Z", 60, ShowKind::Primetime);
            let c = Show {
                manual: true,
                ..show("c", "2026-10-23T15:00:00Z", 90, special_kind())
            };
            s.put_shows(&[c.clone(), a.clone(), b.clone()]).unwrap();
            assert_eq!(
                s.shows().unwrap(),
                vec![a.clone(), b.clone(), c.clone()],
                "{name}"
            );
            let moved = Show {
                plan: PlannedShow {
                    end: b.plan.end + std::time::Duration::from_secs(60),
                    ..b.plan.clone()
                },
                ..b.clone()
            };
            s.put_shows(std::slice::from_ref(&moved)).unwrap();
            assert_eq!(s.shows().unwrap()[1], moved, "{name}: same id, replaced");

            assert_eq!(s.delete_shows_from(b.plan.start).unwrap(), 2, "{name}");
            assert_eq!(s.shows().unwrap(), vec![a.clone()], "{name}");
            s.put_shows(std::slice::from_ref(&b)).unwrap();
            assert_eq!(
                s.delete_shows_ended_before(b.plan.start).unwrap(),
                0,
                "{name}"
            );
            assert_eq!(
                s.delete_shows_ended_before(b.plan.end).unwrap(),
                1,
                "{name}: a ended first"
            );
            assert_eq!(s.shows().unwrap(), vec![b], "{name}");
        }
    }

    fn halloween() -> SpecialEpisode {
        SpecialEpisode {
            date: LocalDate::parse("2026-10-31").unwrap(),
            at: crate::time::LocalTime::from_hm(20, 0).unwrap(),
            title: "Halloween".into(),
            category: "otdb:11".into(),
            length: crate::programme::minutes(60),
            mode: crate::schedule::Mode::Primetime,
            artwork: None,
        }
    }

    #[test]
    fn the_programme_and_specials_are_saved() {
        for (name, s) in backends() {
            assert_eq!(s.programme().unwrap(), None, "{name}: none until seeded");
            let programme = Programme::default();
            s.set_programme(&programme).unwrap();
            assert_eq!(s.programme().unwrap(), Some(programme.clone()), "{name}");
            let invalid = Programme {
                extended_play: crate::programme::minutes(5),
                ..programme.clone()
            };
            let error = failed(s.set_programme(&invalid));
            assert!(
                matches!(
                    &error,
                    StorageError::NotValid(ValidationError::Length {
                        setting: "Extended Play",
                        ..
                    })
                ),
                "{name}: {error:?}"
            );
            assert_eq!(s.programme().unwrap(), Some(programme), "{name}: kept");

            let christmas = SpecialEpisode {
                date: LocalDate::parse("2026-12-25").unwrap(),
                at: crate::time::LocalTime::from_hm(20, 0).unwrap(),
                title: "Christmas".into(),
                category: "otdb:9".into(),
                length: crate::programme::minutes(30),
                mode: crate::schedule::Mode::ExtendedPlay,
                artwork: Some(crate::programme::ThemeArtwork::new("Community").unwrap()),
            };
            s.put_special(&christmas).unwrap();
            s.put_special(&halloween()).unwrap();
            let renamed = SpecialEpisode {
                title: "Spooky".into(),
                ..halloween()
            };
            s.put_special(&renamed).unwrap();
            assert_eq!(
                s.specials().unwrap(),
                vec![renamed, christmas.clone()],
                "{name}: one per date, by date"
            );
            let error = failed(s.put_special(&SpecialEpisode {
                title: " ".into(),
                ..halloween()
            }));
            assert!(
                matches!(
                    &error,
                    StorageError::NotValid(ValidationError::MissingTitle)
                ),
                "{name}: {error:?}"
            );
            assert!(s.delete_special(halloween().slot()).unwrap(), "{name}");
            assert!(!s.delete_special(halloween().slot()).unwrap(), "{name}");
            assert_eq!(s.specials().unwrap(), vec![christmas], "{name}");
        }
    }

    #[test]
    fn dumps_without_a_programme_still_import_and_a_bad_one_is_refused() {
        let old: Dump =
            serde_json::from_str(r#"{"version":1,"exported_unix":0,"players":[],"counters":{}}"#)
                .unwrap();
        let bad: Dump = serde_json::from_str(
            r#"{"version":1,"exported_unix":0,"players":[],"counters":{},
                "programme":{"zone":"Mars/Olympus","primetime_at":"20:00",
                             "primetime_minutes":60,"extended_play_minutes":120,
                             "min_extended_play_minutes":30}}"#,
        )
        .unwrap();
        for (name, s) in backends() {
            s.set_programme(&Programme::default()).unwrap();
            let summary = s.import(&old, ImportMode::Merge).unwrap();
            assert!(!summary.programme, "{name}");
            assert_eq!(s.programme().unwrap(), Some(Programme::default()), "{name}");
            let error = failed(s.import(&bad, ImportMode::Replace));
            assert!(
                matches!(&error, StorageError::InvalidField { field, value }
                    if *field == "zone" && value == "Mars/Olympus"),
                "{name}: {error:?}"
            );
            assert_eq!(s.programme().unwrap(), Some(Programme::default()), "{name}");
        }
    }

    #[test]
    fn a_player_without_a_record_exports_an_absent_name_and_times() {
        let storage = MemoryStorage::default();
        storage
            .put_blob(BlobKind::UserStorage, B, b"<b/>", Utc::from_unix_secs(1))
            .unwrap();
        let dump = storage.export().unwrap();
        let player = &dump.players[0];
        assert_eq!(
            (
                player.gamertag.as_deref(),
                player.first_seen,
                player.last_seen
            ),
            (None, None, None)
        );
        let json = serde_json::to_string(&dump).unwrap();
        assert!(
            json.contains(r#""gamertag":null"#)
                && json.contains(r#""first_seen":null"#)
                && json.contains(r#""last_seen":null"#),
            "{json}"
        );
    }

    #[test]
    fn an_old_dump_reads_an_empty_name_and_zero_times_as_absent() {
        let old: Dump = serde_json::from_str(
            r#"{"version":1,"exported_unix":0,
                "players":[{"xuid":"e000069052e8ad19","gamertag":"",
                            "first_seen":0,"last_seen":0,
                            "user_stats":{"text":"<stats/>"}}],
                "counters":{}}"#,
        )
        .unwrap();
        for (name, storage) in backends() {
            storage.import(&old, ImportMode::Replace).unwrap();
            assert!(storage.players().unwrap().is_empty(), "{name}: no record");
            assert_eq!(
                storage.blob(BlobKind::UserStats, A).unwrap().unwrap(),
                b"<stats/>",
                "{name}: the blob stays"
            );
            let dump = storage.export().unwrap();
            let player = &dump.players[0];
            assert_eq!(
                (
                    player.gamertag.as_deref(),
                    player.first_seen,
                    player.last_seen
                ),
                (None, None, None),
                "{name}"
            );
        }
    }

    #[test]
    fn a_dump_from_before_d23_imports_with_one_episode_extended_play() {
        let old: Dump = serde_json::from_str(
            r#"{"version":1,"exported_unix":0,"players":[],"counters":{},
                "programme":{"zone":"America/New_York","primetime_at":"20:00",
                             "primetime_minutes":60,"extended_play_minutes":120,
                             "min_extended_play_minutes":30}}"#,
        )
        .unwrap();
        for (name, s) in backends() {
            s.import(&old, ImportMode::Replace).unwrap();
            let programme = s.programme().unwrap().unwrap();
            assert_eq!(
                programme.extended_play,
                crate::programme::EP_EPISODE,
                "{name}"
            );
            assert_eq!(
                programme.min_extended_play,
                crate::programme::minutes(30),
                "{name}"
            );
        }
    }

    #[test]
    fn a_version_1_database_gains_the_programme_tables_and_keeps_its_data() {
        let dir = std::env::temp_dir().join(format!("onevs100-storage-v1-{}", std::process::id()));
        let path = dir.join("onevs100.db");
        {
            let s = SqliteStorage::open(&path).unwrap();
            s.counter_add("kept", 1.0).unwrap();
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE programme; DROP TABLE special_episodes; DROP TABLE shows;
             DROP TABLE question_categories; DROP TABLE questions; DROP TABLE categories;
             DROP TABLE question_sources;
             DROP TABLE error_reports;
             PRAGMA user_version = 1;",
        )
        .unwrap();
        drop(conn);
        let s = SqliteStorage::open(&path).unwrap();
        assert_eq!(s.counter("kept").unwrap(), Some(1.0));
        assert_eq!(s.programme().unwrap(), None);
        assert!(s.question_bank().unwrap().is_empty());
        assert_eq!(s.error_reports(1).unwrap(), []);
        s.put_special(&halloween()).unwrap();
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn bank(questions: &str) -> QuestionBank {
        let file = format!(
            r#"{{"version":1,
                "sources":[{{"id":"otdb","name":"Open Trivia Database","licence":"CC BY-SA 4.0"}}],
                "categories":[{{"id":"otdb:11","name":"Film","source":"otdb"}},
                              {{"id":"otdb:9","name":"General Knowledge","source":"otdb"}}],
                "questions":[{questions}]}}"#
        );
        QuestionBank::from_file(serde_json::from_str(&file).unwrap()).unwrap()
    }

    /// With an origin (the source's category and id) unless `id` is a multiple of 3.
    fn bank_question(id: u32, text: &str, categories: &str) -> String {
        let origin = if id.is_multiple_of(3) {
            String::new()
        } else {
            format!(r#","origin":{{"category":"{}","id":"q{id}"}}"#, id + 20)
        };
        format!(
            r#"{{"id":{id},"difficulty":2,"text":"{text}","choices":["A","B","C"],"correct":3,
                "categories":[{categories}],"source":"otdb"{origin}}}"#
        )
    }

    #[test]
    fn the_question_bank_is_saved_merged_replaced_and_dumped() {
        let first = bank(
            &[
                bank_question(7, "Seven", r#""otdb:9","otdb:11""#),
                bank_question(3, "Three", r#""otdb:11""#),
            ]
            .join(","),
        );
        let update = bank(
            &[
                bank_question(3, "Three again", r#""otdb:9""#),
                bank_question(5, "Five", ""),
            ]
            .join(","),
        );
        let mut merged = first.clone();
        merged.merge(update.clone());
        for (name, s) in backends() {
            assert!(s.question_bank().unwrap().is_empty(), "{name}");
            s.import_questions(&first, ImportMode::Replace).unwrap();
            assert_eq!(s.question_bank().unwrap(), first, "{name}");
            s.import_questions(&update, ImportMode::Merge).unwrap();
            assert_eq!(s.question_bank().unwrap(), merged, "{name}");
            let ids: Vec<u32> = merged.questions().iter().map(|q| q.id.0).collect();
            assert_eq!(ids, [7, 3, 5], "{name}: bank order");

            let dump: Dump =
                serde_json::from_str(&serde_json::to_string(&s.export().unwrap()).unwrap())
                    .unwrap();
            for (target_name, target) in backends() {
                let summary = target.import(&dump, ImportMode::Replace).unwrap();
                assert_eq!(summary.questions, 3, "{name} -> {target_name}");
                assert_eq!(
                    target.question_bank().unwrap(),
                    merged,
                    "{name} -> {target_name}"
                );
            }

            s.import_questions(&update, ImportMode::Replace).unwrap();
            assert_eq!(s.question_bank().unwrap(), update, "{name}");
        }
    }

    #[test]
    fn a_version_4_database_keeps_its_question_bank() {
        let dir = std::env::temp_dir().join(format!("onevs100-storage-v4-{}", std::process::id()));
        let path = dir.join("onevs100.db");
        let saved = bank(&[bank_question(3, "Three", r#""otdb:9""#)].join(","));
        {
            let s = SqliteStorage::open(&path).unwrap();
            s.import_questions(&saved, ImportMode::Replace).unwrap();
        }
        // Back to schema 4: categories name their source, questions have no origin, specials no
        // mode.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "ALTER TABLE special_episodes DROP COLUMN mode;
             ALTER TABLE special_episodes DROP COLUMN artwork;
             ALTER TABLE shows DROP COLUMN mode;
             ALTER TABLE shows DROP COLUMN artwork;
             ALTER TABLE categories ADD COLUMN source TEXT NOT NULL DEFAULT 'otdb';
             ALTER TABLE questions DROP COLUMN origin_category;
             ALTER TABLE questions DROP COLUMN origin_id;
             DROP TABLE error_reports;
             PRAGMA user_version = 4;",
        )
        .unwrap();
        drop(conn);
        let s = SqliteStorage::open(&path).unwrap();
        assert_eq!(s.question_bank().unwrap(), saved);
        let more = bank(&[bank_question(7, "Seven", r#""otdb:11""#)].join(","));
        s.import_questions(&more, ImportMode::Merge).unwrap();
        assert_eq!(
            s.question_bank()
                .unwrap()
                .get(QuestionId(7))
                .unwrap()
                .origin
                .id
                .as_deref(),
            Some("q7")
        );
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_version_7_database_gives_its_specials_the_first_primetime_start() {
        let dir = std::env::temp_dir().join(format!("onevs100-storage-v7-{}", std::process::id()));
        let path = dir.join("onevs100.db");
        {
            let s = SqliteStorage::open(&path).unwrap();
            s.set_programme(&Programme {
                primetime_at: PrimetimeSlots::parse("19:00,22:00").unwrap(),
                ..Programme::default()
            })
            .unwrap();
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE special_episodes;
             CREATE TABLE special_episodes (
                 date     TEXT PRIMARY KEY,
                 title    TEXT NOT NULL,
                 category TEXT NOT NULL,
                 minutes  INTEGER NOT NULL,
                 mode     TEXT NOT NULL DEFAULT 'pt',
                 artwork  TEXT
             );
             INSERT INTO special_episodes (date, title, category, minutes)
                 VALUES ('2026-10-31', 'Halloween', 'otdb:11', 60);
             PRAGMA user_version = 7;",
        )
        .unwrap();
        drop(conn);
        let s = SqliteStorage::open(&path).unwrap();
        let seven = SpecialEpisode {
            at: crate::time::LocalTime::from_hm(19, 0).unwrap(),
            ..halloween()
        };
        assert_eq!(s.specials().unwrap(), vec![seven.clone()]);
        let ten = SpecialEpisode {
            at: crate::time::LocalTime::from_hm(22, 0).unwrap(),
            ..halloween()
        };
        s.put_special(&ten).unwrap();
        assert_eq!(
            s.specials().unwrap(),
            vec![seven, ten],
            "one special per Primetime start"
        );
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_version_5_database_keeps_its_specials_as_primetime() {
        let dir = std::env::temp_dir().join(format!("onevs100-storage-v5-{}", std::process::id()));
        let path = dir.join("onevs100.db");
        {
            let s = SqliteStorage::open(&path).unwrap();
            s.put_special(&halloween()).unwrap();
            s.put_shows(&[show("a", "2026-10-31T23:00:00Z", 60, special_kind())])
                .unwrap();
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "ALTER TABLE special_episodes DROP COLUMN mode;
             ALTER TABLE special_episodes DROP COLUMN artwork;
             ALTER TABLE shows DROP COLUMN mode;
             ALTER TABLE shows DROP COLUMN artwork;
             DROP TABLE error_reports;
             PRAGMA user_version = 5;",
        )
        .unwrap();
        drop(conn);
        let s = SqliteStorage::open(&path).unwrap();
        assert_eq!(s.specials().unwrap(), vec![halloween()]);
        let ShowKind::Special { mode, artwork, .. } = &s.shows().unwrap()[0].plan.kind else {
            panic!("a special");
        };
        assert_eq!((*mode, artwork), (crate::schedule::Mode::Primetime, &None));
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dump_round_trips_between_backends_and_through_json() {
        let source = SqliteStorage::open_in_memory().unwrap();
        populate(&source);
        let json = serde_json::to_string_pretty(&source.export().unwrap()).unwrap();
        assert!(
            json.contains("\"text\"") && json.contains("\"base64\""),
            "{json}"
        );

        let dump: Dump = serde_json::from_str(&json).unwrap();
        for (name, target) in backends() {
            target.import(&dump, ImportMode::Merge).unwrap();
            let again = target.export().unwrap();
            assert_eq!(again.players, dump.players, "{name}");
            assert_eq!(again.counters, dump.counters, "{name}");
            assert_eq!(again.programme, dump.programme, "{name}");
            assert_eq!(target.specials().unwrap(), vec![halloween()], "{name}");
            assert_eq!(again.shows, dump.shows, "{name}");
            assert_eq!(
                target.shows().unwrap()[0].plan.kind,
                special_kind(),
                "{name}"
            );
            assert_eq!(
                target.blob(BlobKind::UserStats, B).unwrap().unwrap(),
                b"<s/>",
                "{name}"
            );
        }
    }

    #[test]
    fn import_replace_wipes_and_merge_keeps_other_data() {
        let source = MemoryStorage::default();
        populate(&source);
        let dump = source.export().unwrap();
        for (name, target) in backends() {
            target
                .touch_player(Xuid(0x1234), Some("Other"), Utc::from_unix_secs(1))
                .unwrap();
            target.counter_add("other", 1.0).unwrap();
            let summary = target.import(&dump, ImportMode::Merge).unwrap();
            assert_eq!(
                summary,
                ImportSummary {
                    players: 1,
                    blobs: 2,
                    counters: 1,
                    programme: true,
                    specials: 1,
                    shows: 1,
                    questions: 0,
                },
                "{name}"
            );
            assert!(
                target
                    .players()
                    .unwrap()
                    .iter()
                    .any(|p| p.xuid == Xuid(0x1234)),
                "{name}"
            );
            assert_eq!(target.counter("other").unwrap(), Some(1.0), "{name}");

            target.import(&dump, ImportMode::Replace).unwrap();
            assert!(
                !target
                    .players()
                    .unwrap()
                    .iter()
                    .any(|p| p.xuid == Xuid(0x1234)),
                "{name}"
            );
            assert_eq!(target.counter("other").unwrap(), None, "{name}");
        }
    }

    #[test]
    fn invalid_dumps_are_rejected_without_changing_anything() {
        for (name, s) in backends() {
            populate(s.as_ref());
            let before = s.export().unwrap();
            let mut bad_version = before.clone();
            bad_version.version = 99;
            let error = failed(s.import(&bad_version, ImportMode::Replace));
            assert!(
                matches!(error, StorageError::UnsupportedVersion { found: 99, supported } if supported == i64::from(DUMP_VERSION)),
                "{name}: {error:?}"
            );
            let mut bad_xuid = before.clone();
            bad_xuid.players[0].xuid = "not-hex".into();
            let error = failed(s.import(&bad_xuid, ImportMode::Replace));
            assert!(
                matches!(&error, StorageError::InvalidField { field, value }
                    if *field == "xuid" && value == "not-hex"),
                "{name}: {error:?}"
            );
            let mut bad_b64 = before.clone();
            bad_b64.players[0].avatar = Some(Blob::Base64 {
                base64: "!!".into(),
            });
            let error = failed(s.import(&bad_b64, ImportMode::Replace));
            assert!(
                matches!(error, StorageError::Base64 { .. }),
                "{name}: {error:?}"
            );
            let after = s.export().unwrap();
            assert_eq!(
                (after.players, after.counters),
                (before.players, before.counters),
                "{name}"
            );
        }
    }

    #[test]
    fn a_bad_show_in_a_dump_names_the_show_and_what_is_wrong() {
        for (name, s) in backends() {
            populate(s.as_ref());
            let before = s.export().unwrap();
            let mut ends_early = before.clone();
            ends_early.shows[0].end = ends_early.shows[0].start.clone();
            let error = failed(s.import(&ends_early, ImportMode::Replace));
            assert!(
                matches!(&error, StorageError::InvalidShow { session_id, problem }
                    if session_id == "session-a" && *problem == ShowProblem::EndsBeforeStart),
                "{name}: {error:?}"
            );
            let mut no_ids = before.clone();
            no_ids.shows[0].session_id = String::new();
            let error = failed(s.import(&no_ids, ImportMode::Replace));
            assert!(
                matches!(&error, StorageError::InvalidShow { problem, .. }
                    if *problem == ShowProblem::NoIds),
                "{name}: {error:?}"
            );
            let mut bad_mode = before.clone();
            bad_mode.shows[0].mode = Some("trash".into());
            let error = failed(s.import(&bad_mode, ImportMode::Replace));
            assert!(
                matches!(&error, StorageError::InvalidField { field, value }
                    if *field == "mode" && value == "trash"),
                "{name}: {error:?}"
            );
            assert_eq!(
                s.export().unwrap().shows,
                before.shows,
                "{name}: nothing changed"
            );
        }
    }

    #[test]
    fn sqlite_file_survives_reopen_and_refuses_a_newer_schema() {
        let dir = std::env::temp_dir().join(format!("onevs100-storage-{}", std::process::id()));
        let path = dir.join("nested").join("onevs100.db");
        {
            let s = SqliteStorage::open(&path).unwrap();
            populate(&s);
        }
        let reopened = SqliteStorage::open(&path).unwrap();
        assert_eq!(
            reopened.counter("v3.Stats.Sum.TotalSeen.251317").unwrap(),
            Some(4.0)
        );
        assert!(reopened.blob(BlobKind::UserStorage, A).unwrap().is_some());
        drop(reopened);

        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        drop(conn);
        let error = failed(SqliteStorage::open(&path));
        assert!(
            matches!(error, StorageError::UnsupportedVersion { found: 99, .. }),
            "{error:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sqlite_backend_keeps_its_failures_apart() {
        let dir =
            std::env::temp_dir().join(format!("onevs100-storage-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"a file where the directory should be").unwrap();
        let error = failed(SqliteStorage::open(&blocked.join("onevs100.db")));
        assert!(matches!(error, StorageError::Io(_)), "{error:?}");

        let garbage = dir.join("garbage.db");
        std::fs::write(&garbage, b"not a database").unwrap();
        let error = failed(SqliteStorage::open(&garbage));
        assert!(matches!(error, StorageError::Sqlite(_)), "{error:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A bank row the server would not have written; the column holding it.
    fn saved_bank_with(tag: &str, edit: &str) -> StorageError {
        let dir =
            std::env::temp_dir().join(format!("onevs100-storage-{tag}-{}", std::process::id()));
        let path = dir.join("onevs100.db");
        {
            let s = SqliteStorage::open(&path).unwrap();
            let saved = bank(&[bank_question(3, "Three", r#""otdb:9""#)].join(","));
            s.import_questions(&saved, ImportMode::Replace).unwrap();
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(edit).unwrap();
        drop(conn);
        let error = failed(SqliteStorage::open(&path).unwrap().question_bank());
        let _ = std::fs::remove_dir_all(&dir);
        error
    }

    #[test]
    fn a_saved_question_bank_that_no_longer_passes_its_checks_says_so() {
        let error = saved_bank_with("bank", "UPDATE questions SET text = '';");
        assert!(
            matches!(&error, StorageError::InvalidBank { source }
                if *source == BankError::NoText { id: QuestionId(3) }),
            "{error:?}"
        );
    }

    #[test]
    fn a_saved_question_outside_the_five_difficulties_is_not_read() {
        let error = saved_bank_with("bank-difficulty", "UPDATE questions SET difficulty = 9;");
        assert!(
            matches!(
                error,
                StorageError::Sqlite(rusqlite::Error::IntegralValueOutOfRange(2, 9))
            ),
            "{error:?}"
        );
    }
}
