//! The in-memory backend: tests and `GAMESERVER_DATA_DIR=memory`.

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
};

use super::{
    BlobKind, Data, Dump, ErrorReport, ErrorReportId, ImportMode, ImportSummary, PlayerRecord,
    PlayerSummary, Result, SavedErrorReport, Storage, StorageError, finite_total,
};
use crate::{
    ids::Xuid,
    programme::{Programme, Show, SpecialEpisode, SpecialSlot},
    questions::QuestionBank,
    time::Utc,
};

#[derive(Default)]
pub struct MemoryStorage {
    data: Mutex<Data>,
    /// Apart from `data`, which an import replaces.
    error_reports: Mutex<ErrorReports>,
}

#[derive(Default)]
struct ErrorReports {
    /// Oldest first.
    saved: VecDeque<SavedErrorReport>,
    /// Ids are not reused after a clear, as in the databases.
    last_id: i64,
}

impl MemoryStorage {
    fn data(&self) -> MutexGuard<'_, Data> {
        crate::sync::lock(&self.data)
    }
}

impl Storage for MemoryStorage {
    fn touch_player(&self, xuid: Xuid, gamertag: Option<&str>, now: Utc) -> Result<()> {
        self.data().apply_touch(xuid, gamertag, now);
        Ok(())
    }

    fn set_avatar(&self, xuid: Xuid, avatar: &[u8], now: Utc) -> Result<()> {
        let mut data = self.data();
        data.apply_touch(xuid, None, now);
        if let Some(record) = data.players.get_mut(&xuid) {
            record.avatar = Some(avatar.to_vec());
        }
        Ok(())
    }

    fn players(&self) -> Result<Vec<PlayerRecord>> {
        Ok(self.data().players.values().cloned().collect())
    }

    fn player_summaries(&self) -> Result<Vec<PlayerSummary>> {
        let data = self.data();
        let size = |kind, xuid| data.blobs.get(&(kind, xuid)).map_or(0, Vec::len);
        Ok(data
            .players
            .values()
            .map(|player| PlayerSummary {
                xuid: player.xuid,
                gamertag: player.gamertag.clone(),
                first_seen: player.first_seen,
                last_seen: player.last_seen,
                avatar_bytes: player.avatar.as_ref().map(Vec::len),
                user_storage_bytes: size(BlobKind::UserStorage, player.xuid),
                user_stats_bytes: size(BlobKind::UserStats, player.xuid),
            })
            .collect())
    }

    fn delete_player(&self, xuid: Xuid) -> Result<bool> {
        let mut data = self.data();
        // Remove every blob (no short-circuit), then the player row.
        let removed_storage = data.blobs.remove(&(BlobKind::UserStorage, xuid)).is_some();
        let removed_stats = data.blobs.remove(&(BlobKind::UserStats, xuid)).is_some();
        let had_blobs = removed_storage || removed_stats;
        Ok(data.players.remove(&xuid).is_some() || had_blobs)
    }

    fn blob(&self, kind: BlobKind, xuid: Xuid) -> Result<Option<Vec<u8>>> {
        Ok(self.data().blobs.get(&(kind, xuid)).cloned())
    }

    fn players_with_blob(&self, kind: BlobKind) -> Result<Vec<(String, Vec<u8>)>> {
        let data = self.data();
        Ok(data
            .players
            .values()
            .filter_map(|player| {
                let gamertag = player.gamertag.clone()?;
                let blob = data.blobs.get(&(kind, player.xuid))?;
                Some((gamertag, blob.clone()))
            })
            .collect())
    }

    fn put_blob(&self, kind: BlobKind, xuid: Xuid, data: &[u8], _now: Utc) -> Result<()> {
        self.data().blobs.insert((kind, xuid), data.to_vec());
        Ok(())
    }

    fn counter_add(&self, key: &str, delta: f64) -> Result<f64> {
        let mut data = self.data();
        let total = finite_total(key, data.counters.get(key).copied().unwrap_or(0.0) + delta)?;
        data.counters.insert(key.into(), total);
        Ok(total)
    }

    fn counter(&self, key: &str) -> Result<Option<f64>> {
        Ok(self.data().counters.get(key).copied())
    }

    fn programme(&self) -> Result<Option<Programme>> {
        Ok(self.data().programme.clone())
    }

    fn set_programme(&self, programme: &Programme) -> Result<()> {
        programme.validate().map_err(StorageError::NotValid)?;
        self.data().programme = Some(programme.clone());
        Ok(())
    }

    fn specials(&self) -> Result<Vec<SpecialEpisode>> {
        Ok(self.data().specials.values().cloned().collect())
    }

    fn put_special(&self, special: &SpecialEpisode) -> Result<()> {
        special.validate().map_err(StorageError::NotValid)?;
        self.data().specials.insert(special.slot(), special.clone());
        Ok(())
    }

    fn delete_special(&self, slot: SpecialSlot) -> Result<bool> {
        Ok(self.data().specials.remove(&slot).is_some())
    }

    fn shows(&self) -> Result<Vec<Show>> {
        Ok(self.data().shows_by_start())
    }

    fn put_shows(&self, shows: &[Show]) -> Result<()> {
        let mut data = self.data();
        for show in shows {
            data.shows.insert(show.session_id.clone(), show.clone());
        }
        Ok(())
    }

    fn delete_shows_from(&self, start: Utc) -> Result<usize> {
        let mut data = self.data();
        let before = data.shows.len();
        data.shows.retain(|_, show| show.plan.start < start);
        Ok(before - data.shows.len())
    }

    fn delete_shows_ended_before(&self, end: Utc) -> Result<usize> {
        let mut data = self.data();
        let before = data.shows.len();
        data.shows.retain(|_, show| show.plan.end >= end);
        Ok(before - data.shows.len())
    }

    fn question_bank(&self) -> Result<QuestionBank> {
        Ok(self.data().questions.clone())
    }

    fn import_questions(&self, bank: &QuestionBank, mode: ImportMode) -> Result<()> {
        let mut data = self.data();
        match mode {
            ImportMode::Replace => data.questions = bank.clone(),
            ImportMode::Merge => data.questions.merge(bank.clone()),
        }
        Ok(())
    }

    fn add_error_report(&self, report: &ErrorReport, keep: usize) -> Result<()> {
        let mut reports = crate::sync::lock(&self.error_reports);
        reports.last_id += 1;
        let id = ErrorReportId(reports.last_id);
        reports.saved.push_back(SavedErrorReport {
            id,
            report: report.clone(),
        });
        while reports.saved.len() > keep {
            reports.saved.pop_front();
        }
        Ok(())
    }

    fn error_reports(&self, limit: usize) -> Result<Vec<SavedErrorReport>> {
        let reports = crate::sync::lock(&self.error_reports);
        Ok(reports.saved.iter().rev().take(limit).cloned().collect())
    }

    fn clear_error_reports(&self) -> Result<usize> {
        Ok(std::mem::take(&mut crate::sync::lock(&self.error_reports).saved).len())
    }

    fn export(&self) -> Result<Dump> {
        Ok(self.data().to_dump(crate::time::now()))
    }

    fn import(&self, dump: &Dump, mode: ImportMode) -> Result<ImportSummary> {
        let incoming = Data::from_dump(dump)?;
        let mut data = self.data();
        if mode == ImportMode::Replace {
            *data = Data::default();
        }
        Ok(data.merge(incoming))
    }
}
