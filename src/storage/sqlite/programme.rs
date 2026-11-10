//! The programme, its specials and the materialised shows.

use crate::{
    programme::{Programme, Show, SpecialEpisode},
    storage::{ProgrammeDump, Result, ShowDump, SpecialDump},
    time::Utc,
};

pub(super) fn load_shows(conn: &rusqlite::Connection) -> Result<Vec<Show>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, episode_id, start_unix, end_unix, kind, title, category, mode,
                artwork, manual
         FROM shows ORDER BY start_unix",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ShowDump {
            session_id: row.get(0)?,
            episode_id: row.get(1)?,
            start: Utc::from_unix_secs(row.get(2)?).to_grid(),
            end: Utc::from_unix_secs(row.get(3)?).to_grid(),
            kind: row.get(4)?,
            title: row.get(5)?,
            category: row.get(6)?,
            mode: row.get(7)?,
            artwork: row.get(8)?,
            manual: row.get(9)?,
        })
    })?;
    rows.map(|row| row?.to_show()).collect()
}

pub(super) fn write_show(conn: &rusqlite::Connection, show: &Show) -> Result<()> {
    let row = ShowDump::from_show(show);
    conn.execute(
        "INSERT INTO shows (session_id, episode_id, start_unix, end_unix, kind, title, category,
                            mode, artwork, manual)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(session_id) DO UPDATE SET episode_id = ?2, start_unix = ?3, end_unix = ?4,
             kind = ?5, title = ?6, category = ?7, mode = ?8, artwork = ?9, manual = ?10",
        rusqlite::params![
            row.session_id,
            row.episode_id,
            show.plan.start.unix_secs(),
            show.plan.end.unix_secs(),
            row.kind,
            row.title,
            row.category,
            row.mode,
            row.artwork,
            row.manual
        ],
    )?;
    Ok(())
}

pub(super) fn load_programme(conn: &rusqlite::Connection) -> Result<Option<Programme>> {
    let row = conn.query_row(
        "SELECT zone, primetime_at, primetime_minutes, extended_play_minutes,
                min_extended_play_minutes FROM programme WHERE id = 1",
        [],
        |row| {
            Ok(ProgrammeDump {
                zone: row.get(0)?,
                primetime_at: row.get(1)?,
                primetime_minutes: row.get(2)?,
                extended_play_minutes: row.get(3)?,
                min_extended_play_minutes: row.get(4)?,
            })
        },
    );
    match row {
        Ok(row) => row.to_programme().map(Some),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn load_specials(conn: &rusqlite::Connection) -> Result<Vec<SpecialEpisode>> {
    let mut stmt = conn.prepare(
        "SELECT date, at, title, category, minutes, mode, artwork FROM special_episodes
             ORDER BY date, at",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SpecialDump {
            date: row.get(0)?,
            at: Some(row.get(1)?),
            title: row.get(2)?,
            category: row.get(3)?,
            minutes: row.get(4)?,
            mode: row.get(5)?,
            artwork: row.get(6)?,
        })
    })?;
    rows.map(|row| row?.to_special(None)).collect()
}

pub(super) fn write_programme(conn: &rusqlite::Connection, programme: &Programme) -> Result<()> {
    let row = ProgrammeDump::from_programme(programme);
    conn.execute(
        "INSERT INTO programme (id, zone, primetime_at, primetime_minutes, extended_play_minutes,
                                min_extended_play_minutes)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET zone = ?1, primetime_at = ?2, primetime_minutes = ?3,
             extended_play_minutes = ?4, min_extended_play_minutes = ?5",
        rusqlite::params![
            row.zone,
            row.primetime_at,
            row.primetime_minutes,
            row.extended_play_minutes,
            row.min_extended_play_minutes
        ],
    )?;
    Ok(())
}

pub(super) fn write_special(conn: &rusqlite::Connection, special: &SpecialEpisode) -> Result<()> {
    let row = SpecialDump::from_special(special);
    conn.execute(
        "INSERT INTO special_episodes (date, at, title, category, minutes, mode, artwork)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(date, at) DO UPDATE SET title = ?3, category = ?4, minutes = ?5, mode = ?6,
             artwork = ?7",
        rusqlite::params![
            row.date,
            row.at,
            row.title,
            row.category,
            row.minutes,
            row.mode,
            row.artwork
        ],
    )?;
    Ok(())
}
