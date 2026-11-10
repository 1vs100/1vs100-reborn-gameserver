//! The PostgreSQL backend, selected by `GAMESERVER_DATABASE_URL`.

mod programme;
mod questions;
mod schema;

use std::{
    collections::BTreeMap,
    fmt,
    str::FromStr,
    sync::{Mutex, MutexGuard},
};

use postgres::{GenericClient, NoTls};

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

/// A connection string libpq accepts, as `postgres://user:password@host/database` or
/// `host=... user=...`.
#[derive(Clone, PartialEq, Eq)]
pub struct PostgresUrl(String);

impl PostgresUrl {
    /// `None` for a string that is not a connection string or names no host to connect to.
    pub fn parse(text: &str) -> Option<Self> {
        postgres::Config::from_str(text)
            .ok()
            .filter(|config| !config.get_hosts().is_empty() || !config.get_hostaddrs().is_empty())
            .map(|_| Self(text.into()))
    }

    fn config(&self) -> postgres::Config {
        postgres::Config::from_str(&self.0).expect("parsed when the URL was made")
    }
}

/// Without the string: it can hold a password.
impl fmt::Debug for PostgresUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PostgresUrl(..)")
    }
}

/// How long a connection attempt may take unless the URL says otherwise.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub struct PostgresStorage {
    config: postgres::Config,
    client: Mutex<postgres::Client>,
}

impl PostgresStorage {
    /// Connect to the database at `url` and bring its schema up to date.
    pub fn connect(url: &PostgresUrl) -> Result<Self> {
        Self::with_config(url.config())
    }

    fn with_config(mut config: postgres::Config) -> Result<Self> {
        // An unreachable database fails startup instead of stalling it.
        if config.get_connect_timeout().is_none() {
            config.connect_timeout(CONNECT_TIMEOUT);
        }
        let mut client = config.connect(NoTls)?;
        migrate(&mut client)?;
        Ok(Self {
            config,
            client: Mutex::new(client),
        })
    }

    /// The connection, opened again when the server closed it. The call that found it closed has
    /// already failed; only later calls see the new connection.
    fn client(&self) -> Result<MutexGuard<'_, postgres::Client>> {
        let mut client = crate::sync::lock(&self.client);
        if client.is_closed() {
            tracing::warn!("database connection closed; reconnecting");
            *client = self.config.connect(NoTls)?;
        }
        Ok(client)
    }
}

/// A stored count or minute total; a negative or too large one is not data this server wrote.
fn unsigned(field: &'static str, value: i64) -> Result<u32> {
    u32::try_from(value).map_err(|_| StorageError::InvalidField {
        field,
        value: value.to_string(),
    })
}

/// A stored blob length; `length()` of a `BYTEA` is never negative.
fn length(value: i32) -> usize {
    usize::try_from(value).expect("a length is not negative")
}

fn hex(xuid: Xuid) -> String {
    format!("{xuid:016x}")
}

fn load_players(client: &mut impl GenericClient) -> Result<BTreeMap<Xuid, PlayerRecord>> {
    let mut players = BTreeMap::new();
    for row in client.query(
        "SELECT xuid, gamertag, first_seen, last_seen, avatar FROM players",
        &[],
    )? {
        let xuid = parse_xuid(row.try_get(0)?)?;
        players.insert(
            xuid,
            PlayerRecord {
                xuid,
                gamertag: row.try_get(1)?,
                first_seen: Utc::from_unix_secs(row.try_get(2)?),
                last_seen: Utc::from_unix_secs(row.try_get(3)?),
                avatar: row.try_get(4)?,
            },
        );
    }
    Ok(players)
}

fn load_all(client: &mut impl GenericClient) -> Result<Data> {
    let mut data = Data {
        players: load_players(client)?,
        ..Data::default()
    };
    for row in client.query("SELECT xuid, kind, data FROM user_blobs", &[])? {
        if let Some(kind) = BlobKind::parse(row.try_get(1)?) {
            data.blobs
                .insert((kind, parse_xuid(row.try_get(0)?)?), row.try_get(2)?);
        }
    }
    for row in client.query("SELECT key, value FROM counters", &[])? {
        data.counters.insert(row.try_get(0)?, row.try_get(1)?);
    }
    data.programme = load_programme(client)?;
    for special in load_specials(client)? {
        data.specials.insert(special.slot(), special);
    }
    for show in load_shows(client)? {
        data.shows.insert(show.session_id.clone(), show);
    }
    data.questions = load_questions(client)?;
    Ok(data)
}

fn write_all(client: &mut impl GenericClient, data: &Data) -> Result<()> {
    for record in data.players.values() {
        client.execute(
            "INSERT INTO players (xuid, gamertag, first_seen, last_seen, avatar)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (xuid) DO UPDATE SET gamertag = excluded.gamertag,
                 first_seen = excluded.first_seen, last_seen = excluded.last_seen,
                 avatar = excluded.avatar",
            &[
                &hex(record.xuid),
                &record.gamertag,
                &record.first_seen.unix_secs(),
                &record.last_seen.unix_secs(),
                &record.avatar,
            ],
        )?;
    }
    let now = crate::time::now().unix_secs();
    for ((kind, xuid), blob) in &data.blobs {
        client.execute(
            "INSERT INTO user_blobs (xuid, kind, data, updated) VALUES ($1, $2, $3, $4)
             ON CONFLICT (xuid, kind) DO UPDATE SET data = excluded.data,
                 updated = excluded.updated",
            &[&hex(*xuid), &kind.as_str(), blob, &now],
        )?;
    }
    for (key, value) in &data.counters {
        client.execute(
            "INSERT INTO counters (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            &[key, value],
        )?;
    }
    if let Some(programme) = &data.programme {
        write_programme(client, programme)?;
    }
    for special in data.specials.values() {
        write_special(client, special)?;
    }
    for show in data.shows.values() {
        write_show(client, show)?;
    }
    Ok(())
}

impl Storage for PostgresStorage {
    fn touch_player(&self, xuid: Xuid, gamertag: Option<&str>, now: Utc) -> Result<()> {
        self.client()?.execute(
            "INSERT INTO players (xuid, gamertag, first_seen, last_seen) VALUES ($1, $2, $3, $3)
             ON CONFLICT (xuid) DO UPDATE SET last_seen = $3,
                 gamertag = coalesce($2, players.gamertag)",
            &[&hex(xuid), &gamertag, &now.unix_secs()],
        )?;
        Ok(())
    }

    fn set_avatar(&self, xuid: Xuid, avatar: &[u8], now: Utc) -> Result<()> {
        self.client()?.execute(
            "INSERT INTO players (xuid, first_seen, last_seen, avatar) VALUES ($1, $3, $3, $2)
             ON CONFLICT (xuid) DO UPDATE SET avatar = $2, last_seen = $3",
            &[&hex(xuid), &avatar, &now.unix_secs()],
        )?;
        Ok(())
    }

    fn players(&self) -> Result<Vec<PlayerRecord>> {
        Ok(load_players(&mut *self.client()?)?.into_values().collect())
    }

    fn player_summaries(&self) -> Result<Vec<PlayerSummary>> {
        // The server measures the blobs (`length`) without sending them.
        let rows = self.client()?.query(
            "SELECT p.xuid, p.gamertag, p.first_seen, p.last_seen, length(p.avatar),
                    coalesce(length(s.data), 0), coalesce(length(t.data), 0)
             FROM players p
             LEFT JOIN user_blobs s ON s.xuid = p.xuid AND s.kind = $1
             LEFT JOIN user_blobs t ON t.xuid = p.xuid AND t.kind = $2",
            &[
                &BlobKind::UserStorage.as_str(),
                &BlobKind::UserStats.as_str(),
            ],
        )?;
        let mut summaries = rows
            .iter()
            .map(|row| {
                Ok(PlayerSummary {
                    xuid: parse_xuid(row.try_get(0)?)?,
                    gamertag: row.try_get(1)?,
                    first_seen: Utc::from_unix_secs(row.try_get(2)?),
                    last_seen: Utc::from_unix_secs(row.try_get(3)?),
                    avatar_bytes: row.try_get::<_, Option<i32>>(4)?.map(length),
                    user_storage_bytes: length(row.try_get(5)?),
                    user_stats_bytes: length(row.try_get(6)?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        // By number, as the other backends list them.
        summaries.sort_by_key(|summary| summary.xuid);
        Ok(summaries)
    }

    fn delete_player(&self, xuid: Xuid) -> Result<bool> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let blobs = tx.execute("DELETE FROM user_blobs WHERE xuid = $1", &[&hex(xuid)])?;
        let players = tx.execute("DELETE FROM players WHERE xuid = $1", &[&hex(xuid)])?;
        tx.commit()?;
        Ok(blobs + players > 0)
    }

    fn blob(&self, kind: BlobKind, xuid: Xuid) -> Result<Option<Vec<u8>>> {
        let row = self.client()?.query_opt(
            "SELECT data FROM user_blobs WHERE xuid = $1 AND kind = $2",
            &[&hex(xuid), &kind.as_str()],
        )?;
        Ok(row.map(|row| row.try_get(0)).transpose()?)
    }

    fn players_with_blob(&self, kind: BlobKind) -> Result<Vec<(String, Vec<u8>)>> {
        self.client()?
            .query(
                "SELECT p.gamertag, b.data FROM players p
                 JOIN user_blobs b ON b.xuid = p.xuid AND b.kind = $1
                 WHERE p.gamertag IS NOT NULL",
                &[&kind.as_str()],
            )?
            .iter()
            .map(|row| Ok((row.try_get(0)?, row.try_get(1)?)))
            .collect()
    }

    fn put_blob(&self, kind: BlobKind, xuid: Xuid, data: &[u8], now: Utc) -> Result<()> {
        self.client()?.execute(
            "INSERT INTO user_blobs (xuid, kind, data, updated) VALUES ($1, $2, $3, $4)
             ON CONFLICT (xuid, kind) DO UPDATE SET data = $3, updated = $4",
            &[&hex(xuid), &kind.as_str(), &data, &now.unix_secs()],
        )?;
        Ok(())
    }

    fn counter_add(&self, key: &str, delta: f64) -> Result<f64> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        // The row exists and is locked before it is read, so concurrent adds queue up. The sum is
        // taken here, not in SQL, where an overflow is an error instead of a non-finite total.
        tx.execute(
            "INSERT INTO counters (key, value) VALUES ($1, 0) ON CONFLICT (key) DO NOTHING",
            &[&key],
        )?;
        let current: f64 = tx
            .query_one(
                "SELECT value FROM counters WHERE key = $1 FOR UPDATE",
                &[&key],
            )?
            .try_get(0)?;
        let total = finite_total(key, current + delta)?;
        tx.execute(
            "UPDATE counters SET value = $2 WHERE key = $1",
            &[&key, &total],
        )?;
        tx.commit()?;
        Ok(total)
    }

    fn counter(&self, key: &str) -> Result<Option<f64>> {
        let row = self
            .client()?
            .query_opt("SELECT value FROM counters WHERE key = $1", &[&key])?;
        Ok(row.map(|row| row.try_get(0)).transpose()?)
    }

    fn programme(&self) -> Result<Option<Programme>> {
        load_programme(&mut *self.client()?)
    }

    fn set_programme(&self, programme: &Programme) -> Result<()> {
        programme.validate().map_err(StorageError::NotValid)?;
        write_programme(&mut *self.client()?, programme)
    }

    fn specials(&self) -> Result<Vec<SpecialEpisode>> {
        load_specials(&mut *self.client()?)
    }

    fn put_special(&self, special: &SpecialEpisode) -> Result<()> {
        special.validate().map_err(StorageError::NotValid)?;
        write_special(&mut *self.client()?, special)
    }

    fn delete_special(&self, slot: SpecialSlot) -> Result<bool> {
        let removed = self.client()?.execute(
            "DELETE FROM special_episodes WHERE date = $1 AND at = $2",
            &[&slot.date.to_string(), &slot.at.to_string()],
        )?;
        Ok(removed > 0)
    }

    fn shows(&self) -> Result<Vec<Show>> {
        load_shows(&mut *self.client()?)
    }

    fn put_shows(&self, shows: &[Show]) -> Result<()> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        for show in shows {
            write_show(&mut tx, show)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn delete_shows_from(&self, start: Utc) -> Result<usize> {
        let removed = self.client()?.execute(
            "DELETE FROM shows WHERE start_unix >= $1",
            &[&start.unix_secs()],
        )?;
        Ok(usize::try_from(removed).expect("a row count fits in usize"))
    }

    fn delete_shows_ended_before(&self, end: Utc) -> Result<usize> {
        let removed = self
            .client()?
            .execute("DELETE FROM shows WHERE end_unix < $1", &[&end.unix_secs()])?;
        Ok(usize::try_from(removed).expect("a row count fits in usize"))
    }

    fn question_bank(&self) -> Result<QuestionBank> {
        load_questions(&mut *self.client()?)
    }

    fn import_questions(&self, bank: &QuestionBank, mode: ImportMode) -> Result<()> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let bank = match mode {
            ImportMode::Replace => bank.clone(),
            ImportMode::Merge => {
                let mut saved = load_questions(&mut tx)?;
                saved.merge(bank.clone());
                saved
            }
        };
        write_questions(&mut tx, &bank)?;
        tx.commit()?;
        Ok(())
    }

    fn add_error_report(&self, report: &ErrorReport, keep: usize) -> Result<()> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "INSERT INTO error_reports (received_unix_ms, console, key, document)
             VALUES ($1, $2, $3, $4)",
            &[
                &report.received.unix_ms(),
                &report.console.to_string(),
                &report.key,
                &report.document,
            ],
        )?;
        // Saturated: a larger count keeps every report.
        let keep = i64::try_from(keep).unwrap_or(i64::MAX);
        tx.execute(
            "DELETE FROM error_reports WHERE id NOT IN
                 (SELECT id FROM error_reports ORDER BY id DESC LIMIT $1)",
            &[&keep],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn error_reports(&self, limit: usize) -> Result<Vec<SavedErrorReport>> {
        // Saturated: a larger limit lists every report.
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.client()?
            .query(
                "SELECT id, received_unix_ms, console, key, document FROM error_reports
                 ORDER BY id DESC LIMIT $1",
                &[&limit],
            )?
            .iter()
            .map(|row| {
                Ok(SavedErrorReport {
                    id: ErrorReportId(row.try_get(0)?),
                    report: ErrorReport {
                        received: Utc::from_unix_ms(row.try_get(1)?),
                        console: parse_console(row.try_get(2)?)?,
                        key: row.try_get(3)?,
                        document: row.try_get(4)?,
                    },
                })
            })
            .collect()
    }

    fn clear_error_reports(&self) -> Result<usize> {
        let removed = self.client()?.execute("DELETE FROM error_reports", &[])?;
        Ok(usize::try_from(removed).expect("a row count fits in usize"))
    }

    fn export(&self) -> Result<Dump> {
        let mut client = self.client()?;
        // One snapshot: the server writes between the tables' reads otherwise.
        let mut tx = client
            .build_transaction()
            .isolation_level(postgres::IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()?;
        let data = load_all(&mut tx)?;
        tx.commit()?;
        Ok(data.to_dump(crate::time::now()))
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
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        if mode == ImportMode::Replace {
            tx.batch_execute(
                "DELETE FROM user_blobs; DELETE FROM players; DELETE FROM counters;
                 DELETE FROM programme; DELETE FROM special_episodes; DELETE FROM shows;
                 DELETE FROM question_categories; DELETE FROM questions; DELETE FROM categories;
                 DELETE FROM question_sources;",
            )?;
        }
        write_all(&mut tx, &incoming)?;
        if !incoming.questions.is_empty() {
            let bank = match mode {
                ImportMode::Replace => incoming.questions.clone(),
                ImportMode::Merge => {
                    let mut saved = load_questions(&mut tx)?;
                    saved.merge(incoming.questions.clone());
                    saved
                }
            };
            write_questions(&mut tx, &bank)?;
        }
        tx.commit()?;
        Ok(summary)
    }
}

/// A database of its own for one test: a fresh schema in the database at
/// `GAMESERVER_TEST_DATABASE_URL`. `None` when that is unset. The schemas are left behind, so
/// point it at a database kept for tests.
#[cfg(test)]
pub(crate) fn test_storage() -> Option<PostgresStorage> {
    #[allow(clippy::disallowed_methods)] // a test database, not a server setting
    let url = std::env::var("GAMESERVER_TEST_DATABASE_URL").ok()?;
    let mut config =
        postgres::Config::from_str(&url).expect("GAMESERVER_TEST_DATABASE_URL is a database URL");
    let mut id = [0u8; 8];
    getrandom::getrandom(&mut id).expect("random bytes");
    let schema = format!("test_{:016x}", u64::from_le_bytes(id));
    config
        .connect(NoTls)
        .and_then(|mut client| client.batch_execute(&format!("CREATE SCHEMA {schema}")))
        .expect("a test schema");
    config.options(&format!("-c search_path={schema}"));
    Some(PostgresStorage::with_config(config).expect("a test database"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        programme::{PrimetimeSlots, minutes},
        time::{LocalDate, LocalTime},
    };

    #[test]
    fn a_version_2_database_gives_its_specials_the_first_primetime_start() {
        let Some(s) = test_storage() else {
            return;
        };
        s.set_programme(&Programme {
            primetime_at: PrimetimeSlots::parse("19:00,22:00").unwrap(),
            ..Programme::default()
        })
        .unwrap();
        {
            let mut client = s.client().unwrap();
            client
                .batch_execute(
                    "ALTER TABLE special_episodes DROP CONSTRAINT special_episodes_pkey;
                     ALTER TABLE special_episodes DROP COLUMN at;
                     ALTER TABLE special_episodes ADD PRIMARY KEY (date);
                     INSERT INTO special_episodes (date, title, category, minutes, mode)
                         VALUES ('2026-10-31', 'Halloween', 'otdb:11', 60, 'pt');
                     UPDATE schema_version SET version = 2;",
                )
                .unwrap();
            migrate(&mut client).unwrap();
        }
        let special = |at| SpecialEpisode {
            date: LocalDate::parse("2026-10-31").unwrap(),
            at,
            title: "Halloween".into(),
            category: "otdb:11".into(),
            length: minutes(60),
            mode: crate::schedule::Mode::Primetime,
            artwork: None,
        };
        let seven = special(LocalTime::from_hm(19, 0).unwrap());
        assert_eq!(s.specials().unwrap(), vec![seven.clone()]);
        let ten = special(LocalTime::from_hm(22, 0).unwrap());
        s.put_special(&ten).unwrap();
        assert_eq!(
            s.specials().unwrap(),
            vec![seven, ten],
            "one special per Primetime start"
        );
    }
}
