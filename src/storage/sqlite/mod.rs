//! The SQLite backend, the default.

mod programme;
mod questions;
mod schema;

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use super::{
    BlobKind, Data, Dump, ErrorReport, ErrorReportId, ImportMode, ImportSummary, PlayerRecord,
    PlayerSummary, Result, SavedErrorReport, Storage, StorageError, finite_total, parse_console,
    parse_xuid,
};
use crate::{
    ids::Xuid,
    programme::{Programme, Show, SpecialEpisode, SpecialSlot},
    questions::QuestionBank,
    time::Utc,
};
use programme::{
    load_programme, load_shows, load_specials, write_programme, write_show, write_special,
};
use questions::{load_questions, write_questions};
use schema::migrate;

pub struct SqliteStorage {
    conn: Mutex<rusqlite::Connection>,
}

impl SqliteStorage {
    /// Open (creating directory, file and schema as needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(rusqlite::Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(rusqlite::Connection::open_in_memory()?)
    }

    fn init(conn: rusqlite::Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL keeps readers (admin export) from blocking the SBG writers; ignored for :memory:.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;")?;
        migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, rusqlite::Connection> {
        crate::sync::lock(&self.conn)
    }
}

fn hex(xuid: Xuid) -> String {
    format!("{xuid:016x}")
}

/// The `gamertag` column is NOT NULL: an empty name is how it stores no name.
fn stored_gamertag(gamertag: Option<&str>) -> &str {
    gamertag.unwrap_or("")
}

/// The name a `gamertag` column holds; see [`stored_gamertag`].
fn read_gamertag(stored: String) -> Option<String> {
    (!stored.is_empty()).then_some(stored)
}

fn load_players(conn: &rusqlite::Connection) -> Result<BTreeMap<Xuid, PlayerRecord>> {
    let mut players = BTreeMap::new();
    let mut stmt =
        conn.prepare("SELECT xuid, gamertag, first_seen, last_seen, avatar FROM players")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<Vec<u8>>>(4)?,
        ))
    })?;
    for row in rows {
        let (xuid, gamertag, first_seen, last_seen, avatar) = row?;
        let xuid = parse_xuid(&xuid)?;
        players.insert(
            xuid,
            PlayerRecord {
                xuid,
                gamertag: read_gamertag(gamertag),
                first_seen: Utc::from_unix_secs(first_seen),
                last_seen: Utc::from_unix_secs(last_seen),
                avatar,
            },
        );
    }
    Ok(players)
}

fn load_all(conn: &rusqlite::Connection) -> Result<Data> {
    let mut data = Data {
        players: load_players(conn)?,
        ..Data::default()
    };
    let mut stmt = conn.prepare("SELECT xuid, kind, data FROM user_blobs")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    for row in rows {
        let (xuid, kind, blob) = row?;
        if let Some(kind) = BlobKind::parse(&kind) {
            data.blobs.insert((kind, parse_xuid(&xuid)?), blob);
        }
    }
    let mut stmt = conn.prepare("SELECT key, value FROM counters")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })?;
    for row in rows {
        let (key, value) = row?;
        data.counters.insert(key, value);
    }
    data.programme = load_programme(conn)?;
    for special in load_specials(conn)? {
        data.specials.insert(special.slot(), special);
    }
    for show in load_shows(conn)? {
        data.shows.insert(show.session_id.clone(), show);
    }
    data.questions = load_questions(conn)?;
    Ok(data)
}

fn write_all(tx: &rusqlite::Transaction<'_>, data: &Data) -> Result<()> {
    for record in data.players.values() {
        tx.execute(
            "INSERT INTO players (xuid, gamertag, first_seen, last_seen, avatar)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(xuid) DO UPDATE SET gamertag = excluded.gamertag,
                 first_seen = excluded.first_seen, last_seen = excluded.last_seen,
                 avatar = excluded.avatar",
            rusqlite::params![
                hex(record.xuid),
                stored_gamertag(record.gamertag.as_deref()),
                record.first_seen.unix_secs(),
                record.last_seen.unix_secs(),
                record.avatar
            ],
        )?;
    }
    for ((kind, xuid), blob) in &data.blobs {
        tx.execute(
            "INSERT INTO user_blobs (xuid, kind, data, updated) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(xuid, kind) DO UPDATE SET data = excluded.data, updated = excluded.updated",
            rusqlite::params![
                hex(*xuid),
                kind.as_str(),
                blob,
                crate::time::now().unix_secs()
            ],
        )?;
    }
    for (key, value) in &data.counters {
        tx.execute(
            "INSERT INTO counters (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
    }
    if let Some(programme) = &data.programme {
        write_programme(tx, programme)?;
    }
    for special in data.specials.values() {
        write_special(tx, special)?;
    }
    for show in data.shows.values() {
        write_show(tx, show)?;
    }
    Ok(())
}

impl Storage for SqliteStorage {
    fn touch_player(&self, xuid: Xuid, gamertag: Option<&str>, now: Utc) -> Result<()> {
        self.conn().execute(
            "INSERT INTO players (xuid, gamertag, first_seen, last_seen) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(xuid) DO UPDATE SET last_seen = ?3,
                 gamertag = CASE WHEN ?2 = '' THEN gamertag ELSE ?2 END",
            rusqlite::params![hex(xuid), stored_gamertag(gamertag), now.unix_secs()],
        )?;
        Ok(())
    }

    fn set_avatar(&self, xuid: Xuid, avatar: &[u8], now: Utc) -> Result<()> {
        self.conn().execute(
            "INSERT INTO players (xuid, gamertag, first_seen, last_seen, avatar)
             VALUES (?1, ?4, ?3, ?3, ?2)
             ON CONFLICT(xuid) DO UPDATE SET avatar = ?2, last_seen = ?3",
            rusqlite::params![hex(xuid), avatar, now.unix_secs(), stored_gamertag(None)],
        )?;
        Ok(())
    }

    fn players(&self) -> Result<Vec<PlayerRecord>> {
        Ok(load_players(&self.conn())?.into_values().collect())
    }

    fn player_summaries(&self) -> Result<Vec<PlayerSummary>> {
        let conn = self.conn();
        // One query: SQLite measures the blobs (`length`) without handing them over.
        let mut stmt = conn.prepare(
            "SELECT p.xuid, p.gamertag, p.first_seen, p.last_seen, length(p.avatar),
                    coalesce(length(s.data), 0), coalesce(length(t.data), 0)
             FROM players p
             LEFT JOIN user_blobs s ON s.xuid = p.xuid AND s.kind = ?1
             LEFT JOIN user_blobs t ON t.xuid = p.xuid AND t.kind = ?2",
        )?;
        let rows = stmt.query_map(
            [BlobKind::UserStorage.as_str(), BlobKind::UserStats.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )?;
        let mut summaries = Vec::new();
        for row in rows {
            let (xuid, gamertag, first_seen, last_seen, avatar, storage, stats) = row?;
            summaries.push(PlayerSummary {
                xuid: parse_xuid(&xuid)?,
                gamertag: read_gamertag(gamertag),
                first_seen: Utc::from_unix_secs(first_seen),
                last_seen: Utc::from_unix_secs(last_seen),
                avatar_bytes: avatar.map(|bytes| bytes as usize),
                user_storage_bytes: storage as usize,
                user_stats_bytes: stats as usize,
            });
        }
        summaries.sort_by_key(|summary| summary.xuid);
        Ok(summaries)
    }

    fn delete_player(&self, xuid: Xuid) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let blobs = tx.execute("DELETE FROM user_blobs WHERE xuid = ?1", [hex(xuid)])?;
        let players = tx.execute("DELETE FROM players WHERE xuid = ?1", [hex(xuid)])?;
        tx.commit()?;
        Ok(blobs + players > 0)
    }

    fn blob(&self, kind: BlobKind, xuid: Xuid) -> Result<Option<Vec<u8>>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached("SELECT data FROM user_blobs WHERE xuid = ?1 AND kind = ?2")?;
        let mut rows = stmt.query(rusqlite::params![hex(xuid), kind.as_str()])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }

    fn players_with_blob(&self, kind: BlobKind) -> Result<Vec<(String, Vec<u8>)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT p.gamertag, b.data FROM players p
             JOIN user_blobs b ON b.xuid = p.xuid AND b.kind = ?1",
        )?;
        let rows = stmt.query_map([kind.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let rows = rows.collect::<rusqlite::Result<Vec<(String, Vec<u8>)>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(gamertag, blob)| Some((read_gamertag(gamertag)?, blob)))
            .collect())
    }

    fn put_blob(&self, kind: BlobKind, xuid: Xuid, data: &[u8], now: Utc) -> Result<()> {
        self.conn().execute(
            "INSERT INTO user_blobs (xuid, kind, data, updated) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(xuid, kind) DO UPDATE SET data = ?3, updated = ?4",
            rusqlite::params![hex(xuid), kind.as_str(), data, now.unix_secs()],
        )?;
        Ok(())
    }

    fn counter_add(&self, key: &str, delta: f64) -> Result<f64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let current: Option<f64> = tx
            .query_row("SELECT value FROM counters WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        let total = finite_total(key, current.unwrap_or(0.0) + delta)?;
        tx.execute(
            "INSERT INTO counters (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            rusqlite::params![key, total],
        )?;
        tx.commit()?;
        Ok(total)
    }

    fn counter(&self, key: &str) -> Result<Option<f64>> {
        let conn = self.conn();
        match conn.query_row("SELECT value FROM counters WHERE key = ?1", [key], |row| {
            row.get(0)
        }) {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn programme(&self) -> Result<Option<Programme>> {
        load_programme(&self.conn())
    }

    fn set_programme(&self, programme: &Programme) -> Result<()> {
        programme.validate().map_err(StorageError::NotValid)?;
        write_programme(&self.conn(), programme)
    }

    fn specials(&self) -> Result<Vec<SpecialEpisode>> {
        load_specials(&self.conn())
    }

    fn put_special(&self, special: &SpecialEpisode) -> Result<()> {
        special.validate().map_err(StorageError::NotValid)?;
        write_special(&self.conn(), special)
    }

    fn delete_special(&self, slot: SpecialSlot) -> Result<bool> {
        let removed = self.conn().execute(
            "DELETE FROM special_episodes WHERE date = ?1 AND at = ?2",
            [slot.date.to_string(), slot.at.to_string()],
        )?;
        Ok(removed > 0)
    }

    fn shows(&self) -> Result<Vec<Show>> {
        load_shows(&self.conn())
    }

    fn put_shows(&self, shows: &[Show]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for show in shows {
            write_show(&tx, show)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn delete_shows_from(&self, start: Utc) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM shows WHERE start_unix >= ?1",
            [start.unix_secs()],
        )?)
    }

    fn delete_shows_ended_before(&self, end: Utc) -> Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM shows WHERE end_unix < ?1", [end.unix_secs()])?)
    }

    fn question_bank(&self) -> Result<QuestionBank> {
        load_questions(&self.conn())
    }

    fn import_questions(&self, bank: &QuestionBank, mode: ImportMode) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let bank = match mode {
            ImportMode::Replace => bank.clone(),
            ImportMode::Merge => {
                let mut saved = load_questions(&tx)?;
                saved.merge(bank.clone());
                saved
            }
        };
        write_questions(&tx, &bank)?;
        tx.commit()?;
        Ok(())
    }

    fn add_error_report(&self, report: &ErrorReport, keep: usize) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO error_reports (received_unix_ms, console, key, document)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                report.received.unix_ms(),
                report.console.to_string(),
                report.key,
                report.document
            ],
        )?;
        // Saturated: a larger count keeps every report.
        let keep = i64::try_from(keep).unwrap_or(i64::MAX);
        tx.execute(
            "DELETE FROM error_reports WHERE id NOT IN
                 (SELECT id FROM error_reports ORDER BY id DESC LIMIT ?1)",
            [keep],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn error_reports(&self, limit: usize) -> Result<Vec<SavedErrorReport>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, received_unix_ms, console, key, document FROM error_reports
             ORDER BY id DESC LIMIT ?1",
        )?;
        // Saturated: a larger limit lists every report.
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map([limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })?;
        let mut reports = Vec::new();
        for row in rows {
            let (id, received, console, key, document) = row?;
            reports.push(SavedErrorReport {
                id: ErrorReportId(id),
                report: ErrorReport {
                    received: Utc::from_unix_ms(received),
                    console: parse_console(&console)?,
                    key,
                    document,
                },
            });
        }
        Ok(reports)
    }

    fn clear_error_reports(&self) -> Result<usize> {
        Ok(self.conn().execute("DELETE FROM error_reports", [])?)
    }

    fn export(&self) -> Result<Dump> {
        Ok(load_all(&self.conn())?.to_dump(crate::time::now()))
    }

    fn import(&self, dump: &Dump, mode: ImportMode) -> Result<ImportSummary> {
        let incoming = Data::from_dump(dump)?;
        let summary = ImportSummary {
            players: incoming.players.len(),
            blobs: incoming.blobs.len(),
            counters: incoming.counters.len(),
            programme: incoming.programme.is_some(),
            specials: incoming.specials.len(),
            shows: incoming.shows.len(),
            questions: incoming.questions.questions().len(),
        };
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if mode == ImportMode::Replace {
            tx.execute_batch(
                "DELETE FROM user_blobs; DELETE FROM players; DELETE FROM counters;
                 DELETE FROM programme; DELETE FROM special_episodes; DELETE FROM shows;
                 DELETE FROM question_categories; DELETE FROM questions; DELETE FROM categories;
                 DELETE FROM question_sources;",
            )?;
        }
        write_all(&tx, &incoming)?;
        if !incoming.questions.is_empty() {
            let bank = match mode {
                ImportMode::Replace => incoming.questions.clone(),
                ImportMode::Merge => {
                    let mut saved = load_questions(&tx)?;
                    saved.merge(incoming.questions.clone());
                    saved
                }
            };
            write_questions(&tx, &bank)?;
        }
        tx.commit()?;
        Ok(summary)
    }
}
