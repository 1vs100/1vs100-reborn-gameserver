//! The programme, its specials and the materialised shows.

use postgres::GenericClient;

use super::unsigned;
use crate::{
    programme::{Programme, Show, SpecialEpisode},
    storage::{ProgrammeDump, Result, ShowDump, SpecialDump},
    time::Utc,
};

pub(super) fn load_shows(client: &mut impl GenericClient) -> Result<Vec<Show>> {
    client
        .query(
            "SELECT session_id, episode_id, start_unix, end_unix, kind, title, category, mode,
                    artwork, manual
             FROM shows ORDER BY start_unix",
            &[],
        )?
        .iter()
        .map(|row| {
            ShowDump {
                session_id: row.try_get(0)?,
                episode_id: row.try_get(1)?,
                start: Utc::from_unix_secs(row.try_get(2)?).to_grid(),
                end: Utc::from_unix_secs(row.try_get(3)?).to_grid(),
                kind: row.try_get(4)?,
                title: row.try_get(5)?,
                category: row.try_get(6)?,
                mode: row.try_get(7)?,
                artwork: row.try_get(8)?,
                manual: row.try_get(9)?,
            }
            .to_show()
        })
        .collect()
}

pub(super) fn write_show(client: &mut impl GenericClient, show: &Show) -> Result<()> {
    let row = ShowDump::from_show(show);
    client.execute(
        "INSERT INTO shows (session_id, episode_id, start_unix, end_unix, kind, title, category,
                            mode, artwork, manual)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (session_id) DO UPDATE SET episode_id = $2, start_unix = $3, end_unix = $4,
             kind = $5, title = $6, category = $7, mode = $8, artwork = $9, manual = $10",
        &[
            &row.session_id,
            &row.episode_id,
            &show.plan.start.unix_secs(),
            &show.plan.end.unix_secs(),
            &row.kind,
            &row.title,
            &row.category,
            &row.mode,
            &row.artwork,
            &row.manual,
        ],
    )?;
    Ok(())
}

pub(super) fn load_programme(client: &mut impl GenericClient) -> Result<Option<Programme>> {
    let Some(row) = client.query_opt(
        "SELECT zone, primetime_at, primetime_minutes, extended_play_minutes,
                min_extended_play_minutes FROM programme WHERE id = 1",
        &[],
    )?
    else {
        return Ok(None);
    };
    ProgrammeDump {
        zone: row.try_get(0)?,
        primetime_at: row.try_get(1)?,
        primetime_minutes: unsigned("primetime_minutes", row.try_get(2)?)?,
        extended_play_minutes: unsigned("extended_play_minutes", row.try_get(3)?)?,
        min_extended_play_minutes: unsigned("min_extended_play_minutes", row.try_get(4)?)?,
    }
    .to_programme()
    .map(Some)
}

pub(super) fn load_specials(client: &mut impl GenericClient) -> Result<Vec<SpecialEpisode>> {
    client
        .query(
            "SELECT date, at, title, category, minutes, mode, artwork FROM special_episodes
             ORDER BY date, at",
            &[],
        )?
        .iter()
        .map(|row| {
            SpecialDump {
                date: row.try_get(0)?,
                at: Some(row.try_get(1)?),
                title: row.try_get(2)?,
                category: row.try_get(3)?,
                minutes: unsigned("minutes", row.try_get(4)?)?,
                mode: Some(row.try_get(5)?),
                artwork: row.try_get(6)?,
            }
            .to_special(None)
        })
        .collect()
}

pub(super) fn write_programme(
    client: &mut impl GenericClient,
    programme: &Programme,
) -> Result<()> {
    let row = ProgrammeDump::from_programme(programme);
    client.execute(
        "INSERT INTO programme (id, zone, primetime_at, primetime_minutes, extended_play_minutes,
                                min_extended_play_minutes)
         VALUES (1, $1, $2, $3, $4, $5)
         ON CONFLICT (id) DO UPDATE SET zone = $1, primetime_at = $2, primetime_minutes = $3,
             extended_play_minutes = $4, min_extended_play_minutes = $5",
        &[
            &row.zone,
            &row.primetime_at,
            &i64::from(row.primetime_minutes),
            &i64::from(row.extended_play_minutes),
            &i64::from(row.min_extended_play_minutes),
        ],
    )?;
    Ok(())
}

pub(super) fn write_special(
    client: &mut impl GenericClient,
    special: &SpecialEpisode,
) -> Result<()> {
    let row = SpecialDump::from_special(special);
    client.execute(
        "INSERT INTO special_episodes (date, at, title, category, minutes, mode, artwork)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (date, at) DO UPDATE SET title = $3, category = $4, minutes = $5, mode = $6,
             artwork = $7",
        &[
            &row.date,
            &row.at,
            &row.title,
            &row.category,
            &i64::from(row.minutes),
            &special.mode.as_str(),
            &row.artwork,
        ],
    )?;
    Ok(())
}
