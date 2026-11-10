//! The schema and its migrations.

use crate::storage::{Result, StorageError};

/// Schema version stored in `PRAGMA user_version`; bump together with a new migration step.
const SCHEMA_VERSION: i64 = 8;

pub(super) fn migrate(conn: &rusqlite::Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(StorageError::UnsupportedVersion {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    if version < 1 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE players (
                 xuid       TEXT PRIMARY KEY,          -- 16 hex digits
                 gamertag   TEXT NOT NULL DEFAULT '',
                 first_seen INTEGER NOT NULL,
                 last_seen  INTEGER NOT NULL,
                 avatar     BLOB
             );
             CREATE TABLE user_blobs (
                 xuid    TEXT NOT NULL,
                 kind    TEXT NOT NULL,                -- user_storage | user_stats
                 data    BLOB NOT NULL,
                 updated INTEGER NOT NULL,
                 PRIMARY KEY (xuid, kind)
             );
             CREATE TABLE counters (
                 key   TEXT PRIMARY KEY,
                 value REAL NOT NULL
             );
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
    }
    if version < 2 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE programme (
                 id                        INTEGER PRIMARY KEY CHECK (id = 1), -- one row
                 zone                      TEXT NOT NULL,    -- IANA name
                 primetime_at              TEXT NOT NULL,    -- HH:MM in zone, comma-separated
                 primetime_minutes         INTEGER NOT NULL,
                 extended_play_minutes     INTEGER NOT NULL,
                 min_extended_play_minutes INTEGER NOT NULL
             );
             CREATE TABLE special_episodes (
                 date     TEXT PRIMARY KEY,                -- YYYY-MM-DD in the programme's zone
                 title    TEXT NOT NULL,
                 category TEXT NOT NULL,
                 minutes  INTEGER NOT NULL
             );
             PRAGMA user_version = 2;
             COMMIT;",
        )?;
    }
    if version < 3 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE shows (
                 session_id TEXT PRIMARY KEY,                -- random UUID
                 episode_id TEXT NOT NULL,
                 start_unix INTEGER NOT NULL,
                 end_unix   INTEGER NOT NULL,
                 kind       TEXT NOT NULL,                   -- primetime | special | extended_play
                 title      TEXT,                            -- a special's
                 category   TEXT,                            -- a special's
                 manual     INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX shows_by_start ON shows (start_unix);
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
    }
    if version < 4 {
        conn.execute_batch(
            "BEGIN;
             -- The imported question bank; `position` keeps the bank order the title is served.
             CREATE TABLE question_sources (
                 id       TEXT PRIMARY KEY,
                 position INTEGER NOT NULL,
                 name     TEXT NOT NULL,
                 licence  TEXT NOT NULL,
                 url      TEXT NOT NULL
             );
             CREATE TABLE categories (
                 id       TEXT PRIMARY KEY,                 -- otdb:9
                 position INTEGER NOT NULL,
                 name     TEXT NOT NULL,
                 source   TEXT NOT NULL
             );
             CREATE TABLE questions (
                 id         INTEGER PRIMARY KEY,
                 position   INTEGER NOT NULL,
                 locale     TEXT NOT NULL,
                 difficulty INTEGER NOT NULL,             -- 1-5
                 text       TEXT NOT NULL,
                 choice1    TEXT NOT NULL,
                 choice2    TEXT NOT NULL,
                 choice3    TEXT NOT NULL,
                 correct    INTEGER NOT NULL,             -- 1-3
                 source     TEXT NOT NULL
             );
             CREATE TABLE question_categories (
                 question_id INTEGER NOT NULL,
                 category    TEXT NOT NULL,
                 position    INTEGER NOT NULL,
                 PRIMARY KEY (question_id, category)
             );
             PRAGMA user_version = 4;
             COMMIT;",
        )?;
    }
    if version < 5 {
        // Categories are ours, not a source's: their source column goes, and a question keeps
        // where it was in its source.
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE categories DROP COLUMN source;
             ALTER TABLE questions ADD COLUMN origin_category TEXT;  -- the source's own category
             ALTER TABLE questions ADD COLUMN origin_id TEXT;        -- the source's own id
             PRAGMA user_version = 5;
             COMMIT;",
        )?;
    }
    if version < 6 {
        // A special's mode and theme artwork; a special stored without a mode is Primetime.
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE special_episodes ADD COLUMN mode TEXT NOT NULL DEFAULT 'pt'; -- pt | ep
             ALTER TABLE special_episodes ADD COLUMN artwork TEXT;
             ALTER TABLE shows ADD COLUMN mode TEXT;                                -- a special's
             ALTER TABLE shows ADD COLUMN artwork TEXT;                             -- a special's
             PRAGMA user_version = 6;
             COMMIT;",
        )?;
    }
    if version < 7 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE error_reports (
                 id               INTEGER PRIMARY KEY AUTOINCREMENT,
                 received_unix_ms INTEGER NOT NULL,
                 console          TEXT NOT NULL,          -- IP address
                 key              TEXT NOT NULL,
                 document         BLOB NOT NULL
             );
             PRAGMA user_version = 7;
             COMMIT;",
        )?;
    }
    if version < 8 {
        // A special replaces one Primetime show of its day: the programme's first start, or
        // 20:00 (the settings' default) in a database whose programme is not seeded yet.
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE special_episodes_by_start (
                 date     TEXT NOT NULL,                  -- YYYY-MM-DD in the programme's zone
                 at       TEXT NOT NULL,                  -- HH:MM, the Primetime start replaced
                 title    TEXT NOT NULL,
                 category TEXT NOT NULL,
                 minutes  INTEGER NOT NULL,
                 mode     TEXT NOT NULL DEFAULT 'pt',     -- pt | ep
                 artwork  TEXT,
                 PRIMARY KEY (date, at)
             );
             INSERT INTO special_episodes_by_start
                 SELECT date,
                        COALESCE((SELECT substr(primetime_at, 1, 5) FROM programme WHERE id = 1),
                                 '20:00'),
                        title, category, minutes, mode, artwork
                 FROM special_episodes;
             DROP TABLE special_episodes;
             ALTER TABLE special_episodes_by_start RENAME TO special_episodes;
             PRAGMA user_version = 8;
             COMMIT;",
        )?;
    }
    Ok(())
}
