//! The schema and its migrations.

use crate::storage::{Result, StorageError};

/// Schema version stored in `schema_version`; bump together with a new migration step.
const SCHEMA_VERSION: i64 = 3;

pub(super) fn migrate(client: &mut postgres::Client) -> Result<()> {
    let mut tx = client.transaction()?;
    // Two servers starting on one database migrate one after the other.
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext('onevs100 schema'))",
        &[],
    )?;
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS schema_version (
             id      SMALLINT PRIMARY KEY CHECK (id = 1),  -- one row
             version BIGINT NOT NULL
         )",
    )?;
    let version: i64 = tx
        .query_opt("SELECT version FROM schema_version WHERE id = 1", &[])?
        .map(|row| row.try_get(0))
        .transpose()?
        .unwrap_or(0);
    if version > SCHEMA_VERSION {
        return Err(StorageError::UnsupportedVersion {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    if version < 1 {
        tx.batch_execute(
            "CREATE TABLE players (
                 xuid       TEXT PRIMARY KEY,               -- 16 hex digits
                 gamertag   TEXT,
                 first_seen BIGINT NOT NULL,                -- unix seconds
                 last_seen  BIGINT NOT NULL,
                 avatar     BYTEA
             );
             CREATE TABLE user_blobs (
                 xuid    TEXT NOT NULL,
                 kind    TEXT NOT NULL,                     -- user_storage | user_stats
                 data    BYTEA NOT NULL,
                 updated BIGINT NOT NULL,
                 PRIMARY KEY (xuid, kind)
             );
             CREATE TABLE counters (
                 key   TEXT PRIMARY KEY,
                 value DOUBLE PRECISION NOT NULL
             );
             CREATE TABLE programme (
                 id                        SMALLINT PRIMARY KEY CHECK (id = 1), -- one row
                 zone                      TEXT NOT NULL,   -- IANA name
                 primetime_at              TEXT NOT NULL,   -- HH:MM in zone, comma-separated
                 primetime_minutes         BIGINT NOT NULL,
                 extended_play_minutes     BIGINT NOT NULL,
                 min_extended_play_minutes BIGINT NOT NULL
             );
             CREATE TABLE special_episodes (
                 date     TEXT PRIMARY KEY,                 -- YYYY-MM-DD in the programme's zone
                 title    TEXT NOT NULL,
                 category TEXT NOT NULL,
                 minutes  BIGINT NOT NULL,
                 mode     TEXT NOT NULL,                    -- pt | ep
                 artwork  TEXT
             );
             CREATE TABLE shows (
                 session_id TEXT PRIMARY KEY,               -- random UUID
                 episode_id TEXT NOT NULL,
                 start_unix BIGINT NOT NULL,
                 end_unix   BIGINT NOT NULL,
                 kind       TEXT NOT NULL,                  -- primetime | special | extended_play
                 title      TEXT,                           -- a special's
                 category   TEXT,                           -- a special's
                 mode       TEXT,                           -- a special's
                 artwork    TEXT,                           -- a special's
                 manual     BOOLEAN NOT NULL
             );
             CREATE INDEX shows_by_start ON shows (start_unix);
             -- The imported question bank; `position` keeps the bank order the title is served.
             CREATE TABLE question_sources (
                 id       TEXT PRIMARY KEY,
                 position BIGINT NOT NULL,
                 name     TEXT NOT NULL,
                 licence  TEXT NOT NULL,
                 url      TEXT NOT NULL
             );
             CREATE TABLE categories (
                 id       TEXT PRIMARY KEY,                 -- otdb:9
                 position BIGINT NOT NULL,
                 name     TEXT NOT NULL
             );
             CREATE TABLE questions (
                 id              BIGINT PRIMARY KEY,
                 position        BIGINT NOT NULL,
                 locale          TEXT NOT NULL,
                 difficulty      SMALLINT NOT NULL,         -- 1-5
                 text            TEXT NOT NULL,
                 choice1         TEXT NOT NULL,
                 choice2         TEXT NOT NULL,
                 choice3         TEXT NOT NULL,
                 correct         SMALLINT NOT NULL,         -- 1-3
                 source          TEXT NOT NULL,
                 origin_category TEXT,                      -- the source's own category
                 origin_id       TEXT                       -- the source's own id
             );
             CREATE TABLE question_categories (
                 question_id BIGINT NOT NULL,
                 category    TEXT NOT NULL,
                 position    BIGINT NOT NULL,
                 PRIMARY KEY (question_id, category)
             );",
        )?;
    }
    if version < 2 {
        tx.batch_execute(
            "CREATE TABLE error_reports (
                 id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                 received_unix_ms BIGINT NOT NULL,
                 console          TEXT NOT NULL,            -- IP address
                 key              TEXT NOT NULL,
                 document         BYTEA NOT NULL
             )",
        )?;
    }
    if version < 3 {
        // A special replaces one Primetime show of its day: the programme's first start, or
        // 20:00 (the settings' default) in a database whose programme is not seeded yet.
        tx.batch_execute(
            "ALTER TABLE special_episodes ADD COLUMN at TEXT;  -- HH:MM, the Primetime start replaced
             UPDATE special_episodes SET at = COALESCE(
                 (SELECT split_part(primetime_at, ',', 1) FROM programme WHERE id = 1), '20:00');
             ALTER TABLE special_episodes ALTER COLUMN at SET NOT NULL;
             ALTER TABLE special_episodes DROP CONSTRAINT special_episodes_pkey;
             ALTER TABLE special_episodes ADD PRIMARY KEY (date, at);",
        )?;
    }
    tx.execute(
        "INSERT INTO schema_version (id, version) VALUES (1, $1)
         ON CONFLICT (id) DO UPDATE SET version = excluded.version",
        &[&SCHEMA_VERSION],
    )?;
    tx.commit()?;
    Ok(())
}
