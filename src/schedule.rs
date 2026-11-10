//! Show schedule: which show is on, and its ids.
//! Materialises the next 48 hours from the programme; without one serves a single show under constant ids.
//! Shows are back to back: the title moves to the next session only within 60 s of the current end.

use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime},
};

use sha1::{Digest, Sha1};

use crate::{
    programme::{
        self, PlannedShow, Programme, Show, ShowKind, SpecialEpisode, SpecialSlot, ValidationError,
    },
    sbg::key::{ParsedKey, ShowIdKind},
    storage::{MemoryStorage, Storage, StorageError},
    time::Utc,
};

/// The session id every internal key is normalised to (see [`canonicalize_key`]).
pub const CANONICAL_SESSION: &str = "00000000-0000-0000-0000-000000000001";
/// The episode id used in the `Setup` keys (`AvailableLanguages`, question bank locale lists).
pub const CANONICAL_EPISODE: &str = "00000000-0000-0000-0000-000000000000";

/// Game mode of a show, sent as MergedLaunchData `GameType` (`program.lua:133`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The One vs the Mob, elections (GameType 1).
    #[default]
    Primetime,
    /// Everyone plays as Mob, no host or elections (GameType 2).
    ExtendedPlay,
}

impl Mode {
    /// `pt` or `ep` (admin API).
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pt" => Some(Self::Primetime),
            "ep" => Some(Self::ExtendedPlay),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primetime => "pt",
            Self::ExtendedPlay => "ep",
        }
    }

    pub fn game_type(self) -> u8 {
        match self {
            Self::Primetime => 1,
            Self::ExtendedPlay => 2,
        }
    }
}

/// Season (episode group) id: one per calendar month of the show's start (UTC).
pub fn season_id(start: Utc) -> String {
    let (year, month) = start.year_month();
    derived_uuid(&format!("revival-season:{year:04}-{month:02}"))
}

fn derived_uuid(seed: &str) -> String {
    let digest = Sha1::digest(seed.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    format_uuid(b)
}

/// A random (version 4) UUID: a show's session and episode ids.
pub fn random_uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).expect("the operating system provides random bytes");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format_uuid(b)
}

fn format_uuid(b: [u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// Schedule settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleConfig {
    /// How long before a show's end rounds stop (Config `PT_MINIMUM_TIME_TILL_EVENT_END`).
    pub pt_min_end: Duration,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            pt_min_end: DEFAULT_PT_MIN_END,
        }
    }
}

/// With a programme, 5 keeps a round running through the 45:35 sponsor ad of a 50-minute show
/// (see `xsor`).
const DEFAULT_PT_MIN_END: Duration = Duration::from_secs(5 * 60);

/// Without a programme: how far ahead of now the show starts, and how long it is.
const UNSCHEDULED_LEAD: Duration = Duration::from_secs(2 * 60);
const UNSCHEDULED_LENGTH: Duration = Duration::from_secs(2 * 60 * 60);

/// Materialised shows that ended longer ago than this are dropped.
const HISTORY: Duration = Duration::from_secs(7 * 86_400);

/// Session id of the unscheduled following show (v2.0.1149.0 lists it as "Next").
pub const UNSCHEDULED_NEXT_SESSION: &str = "00000000-0000-0000-0000-000000000003";

/// Why a schedule change failed.
#[derive(Debug)]
pub enum ScheduleError {
    /// The programme, special episode or show does not pass its checks; nothing was written.
    Invalid { source: ValidationError },
    /// Storage failed.
    Storage { source: StorageError },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { source } => source.fmt(f),
            Self::Storage { source } => source.fmt(f),
        }
    }
}

impl std::error::Error for ScheduleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid { source } => Some(source),
            Self::Storage { source } => Some(source),
        }
    }
}

impl From<ValidationError> for ScheduleError {
    fn from(source: ValidationError) -> Self {
        Self::Invalid { source }
    }
}

impl From<StorageError> for ScheduleError {
    fn from(source: StorageError) -> Self {
        Self::Storage { source }
    }
}

/// The window of the unscheduled show `offset` shows ahead: from a lead after now, back to back.
fn unscheduled_window(now: SystemTime, offset: u64) -> (SystemTime, SystemTime) {
    (
        now + UNSCHEDULED_LEAD + UNSCHEDULED_LENGTH * offset as u32,
        now + UNSCHEDULED_LENGTH + UNSCHEDULED_LENGTH * offset as u32,
    )
}

/// Programme, specials, and materialised shows in `storage`; last session consoles used.
pub struct Schedule {
    pt_min_end: Duration,
    storage: Arc<dyn Storage>,
    /// `None`: no programme, the unscheduled show is served.
    programme: Option<Programme>,
    specials: Vec<SpecialEpisode>,
    /// By start, back to back.
    shows: Vec<Show>,
    /// The scheduled session consoles last talked about (see [`Schedule::activate_session`]).
    active_session: Option<String>,
}

impl Default for Schedule {
    /// No programme, nothing saved.
    fn default() -> Self {
        Self {
            pt_min_end: DEFAULT_PT_MIN_END,
            storage: Arc::new(MemoryStorage::default()),
            programme: None,
            specials: Vec::new(),
            shows: Vec::new(),
            active_session: None,
        }
    }
}

impl Schedule {
    /// The schedule saved in `storage`: its programme (if any), specials and shows.
    pub fn load(config: ScheduleConfig, storage: Arc<dyn Storage>) -> Result<Self, StorageError> {
        Ok(Self {
            pt_min_end: config.pt_min_end,
            programme: storage.programme()?,
            specials: storage.specials()?,
            shows: storage.shows()?,
            storage,
            active_session: None,
        })
    }

    /// Read the programme, specials and shows from storage again (after a data import replaced
    /// them there). Which session consoles last talked about is kept.
    pub fn reload(&mut self) -> Result<(), StorageError> {
        self.programme = self.storage.programme()?;
        self.specials = self.storage.specials()?;
        self.shows = self.storage.shows()?;
        Ok(())
    }

    pub fn is_enabled(&self) -> bool {
        self.programme.is_some()
    }

    pub fn pt_min_end(&self) -> Duration {
        self.pt_min_end
    }

    pub fn programme(&self) -> Option<&Programme> {
        self.programme.as_ref()
    }

    pub fn specials(&self) -> &[SpecialEpisode] {
        &self.specials
    }

    /// The materialised shows, the horizon topped up first.
    pub fn shows(&mut self, now: Utc) -> &[Show] {
        self.refresh(now);
        &self.shows
    }

    /// Drop old shows and materialise the horizon (`programme::extend`), saving what is new.
    fn refresh(&mut self, now: Utc) {
        let Some(programme) = self.programme.clone() else {
            return;
        };
        let cutoff = now - HISTORY;
        if self
            .shows
            .first()
            .is_some_and(|show| show.plan.end < cutoff)
        {
            self.shows.retain(|show| show.plan.end >= cutoff);
            if let Err(error) = self.storage.delete_shows_ended_before(cutoff) {
                tracing::warn!(%error, "old shows not removed from storage");
            }
        }
        let mut ids = random_uuid;
        let new = programme::extend(&programme, &self.specials, &self.shows, now, &mut ids);
        if new.is_empty() {
            return;
        }
        if let Err(error) = self.storage.put_shows(&new) {
            tracing::error!(%error, "planned shows not saved: their ids change after a restart");
        }
        for show in &new {
            tracing::info!(
                session = %show.session_id,
                start = %show.plan.start.to_grid(),
                minutes = show.plan.minutes(),
                kind = show.plan.kind.as_str(),
                "show planned"
            );
        }
        self.shows.extend(new);
    }

    /// Keep the shows consoles may know (`programme::frozen`), plan the rest again.
    fn replan(&mut self, now: Utc) {
        self.refresh(now);
        let keep = programme::frozen(&self.shows, now);
        if let Some(first_dropped) = self.shows.get(keep) {
            if let Err(error) = self.storage.delete_shows_from(first_dropped.plan.start) {
                tracing::warn!(%error, "re-planned shows not removed from storage");
            }
            self.shows.truncate(keep);
        }
        self.refresh(now);
    }

    /// Cut saved Extended Play shows to one episode and plan again; `true` when changed.
    pub fn fit_extended_play_to_one_episode(&mut self, now: Utc) -> Result<bool, ScheduleError> {
        let Some(saved) = self.programme.clone() else {
            return Ok(false);
        };
        let (programme, changed) = saved.with_one_episode_extended_play();
        if !changed {
            return Ok(false);
        }
        tracing::warn!(
            minutes = programme::whole_minutes(programme.extended_play),
            "saved programme had Extended Play shows longer than one episode: cut to one episode"
        );
        self.set_programme(programme, now)?;
        Ok(true)
    }

    /// Save a new programme and plan the shows after the ones consoles may know.
    pub fn set_programme(&mut self, programme: Programme, now: Utc) -> Result<(), ScheduleError> {
        programme.validate_new()?;
        self.storage.set_programme(&programme)?;
        self.programme = Some(programme);
        self.replan(now);
        Ok(())
    }

    /// Save a special episode (replacing one in its slot) and plan again.
    pub fn put_special(&mut self, special: SpecialEpisode, now: Utc) -> Result<(), ScheduleError> {
        special.validate()?;
        if let Some(programme) = &self.programme {
            programme.fits(&special)?;
        }
        self.storage.put_special(&special)?;
        self.specials.retain(|saved| saved.slot() != special.slot());
        self.specials.push(special);
        self.specials.sort_by_key(SpecialEpisode::slot);
        self.replan(now);
        Ok(())
    }

    /// Remove the special episode in `slot` and plan again; `false` when there was none.
    pub fn delete_special(&mut self, slot: SpecialSlot, now: Utc) -> Result<bool, ScheduleError> {
        let removed = self.storage.delete_special(slot)?;
        if removed {
            self.specials.retain(|special| special.slot() != slot);
            self.replan(now);
        }
        Ok(removed)
    }

    /// LAN-test override: end the running show now and run a `mode` show for `length`.
    pub fn start_show_now(
        &mut self,
        now: Utc,
        mode: Mode,
        length: Duration,
    ) -> Result<Show, ScheduleError> {
        if self.programme.is_none() {
            return Err(ValidationError::NoProgramme.into());
        }
        if !(Duration::from_secs(60)..=programme::MAX_SHOW).contains(&length) {
            return Err(ValidationError::ShowLength { length }.into());
        }
        if mode == Mode::ExtendedPlay && length > programme::EP_EPISODE {
            return Err(ValidationError::ExtendedPlayLength { length }.into());
        }
        self.refresh(now);
        self.storage.delete_shows_from(now)?;
        self.shows.retain(|show| show.plan.start < now);
        let mut changed = Vec::new();
        if let Some(running) = self.shows.last_mut().filter(|show| show.plan.end > now) {
            running.plan.end = now;
            changed.push(running.clone());
        }
        let show = Show {
            session_id: random_uuid(),
            episode_id: random_uuid(),
            plan: PlannedShow {
                start: now,
                end: now + length,
                kind: match mode {
                    Mode::Primetime => ShowKind::Primetime,
                    Mode::ExtendedPlay => ShowKind::ExtendedPlay,
                },
            },
            manual: true,
        };
        changed.push(show.clone());
        self.storage.put_shows(&changed)?;
        self.shows.push(show.clone());
        self.refresh(now);
        Ok(show)
    }

    /// `offset`-th show from the running one (0); unscheduled when no programme.
    pub fn showing(&mut self, now: SystemTime, offset: usize) -> Showing {
        if self.programme.is_some() {
            let at = Utc::from(now);
            self.refresh(at);
            if let Some(show) = self
                .shows
                .iter()
                .filter(|show| show.plan.end > at)
                .nth(offset)
            {
                return Showing::Scheduled(show.clone());
            }
            tracing::warn!(
                offset,
                "no planned show that far ahead: serving the unscheduled one"
            );
        }
        let (start, end) = unscheduled_window(now, offset as u64);
        Showing::Unscheduled {
            session_id: CANONICAL_SESSION,
            start,
            end,
        }
    }

    /// `offset`-th show and the one after; distinct ids since the title folds equal ones.
    pub fn showing_pair(&mut self, now: SystemTime, offset: usize) -> [Showing; 2] {
        let current = self.showing(now, offset);
        let mut next = self.showing(now, offset + 1);
        if let (Showing::Unscheduled { .. }, Showing::Unscheduled { session_id, .. }) =
            (&current, &mut next)
        {
            *session_id = UNSCHEDULED_NEXT_SESSION;
        }
        [current, next]
    }

    /// Show for a session `id`: that show, else the running one.
    pub fn showing_for_session(&mut self, now: SystemTime, id: Option<&str>) -> Showing {
        if self.programme.is_some() {
            self.refresh(Utc::from(now));
            if let Some(show) = id.and_then(|id| self.show_by_session_id(id)) {
                return Showing::Scheduled(show.clone());
            }
        } else if id == Some(UNSCHEDULED_NEXT_SESSION) {
            let (start, end) = unscheduled_window(now, 1);
            return Showing::Unscheduled {
                session_id: UNSCHEDULED_NEXT_SESSION,
                start,
                end,
            };
        }
        self.showing(now, 0)
    }

    /// Session of the running show, else the next one.
    pub fn current_session_id(&mut self, now: SystemTime) -> String {
        self.showing(now, 0).session_id()
    }

    /// Question category of a special show; `None` otherwise.
    pub fn category_of(&mut self, now: Utc, asked: &ShowRef) -> Option<String> {
        self.programme.as_ref()?;
        self.refresh(now);
        let show = self.shows.iter().find(|show| match asked {
            ShowRef::Session(id) => show.session_id.eq_ignore_ascii_case(id),
            ShowRef::Episode(id) => show.episode_id.eq_ignore_ascii_case(id),
        })?;
        match &show.plan.kind {
            ShowKind::Special { category, .. } => Some(category.clone()),
            ShowKind::Primetime | ShowKind::ExtendedPlay => None,
        }
    }

    fn show_by_session_id(&self, id: &str) -> Option<&Show> {
        self.shows
            .iter()
            .find(|show| show.session_id.eq_ignore_ascii_case(id))
    }

    /// The scheduled session whose state the store holds, if a console started one.
    pub fn active_session(&self) -> Option<&str> {
        self.active_session.as_deref()
    }

    /// Forget which scheduled session is active (`sbg::reset_volatile`).
    pub fn reset_active_session(&mut self) {
        self.active_session = None;
    }

    /// Mark session `id` active; `true` when the show changed and its state must drop.
    pub fn activate_session(&mut self, now: SystemTime, id: &str) -> bool {
        let Some(show) = self.show_by_session_id(id) else {
            return false;
        };
        if show.plan.start > Utc::from(now) {
            return false;
        }
        let id = id.to_ascii_lowercase();
        let changed = self
            .active_session
            .as_ref()
            .is_some_and(|current| *current != id);
        self.active_session = Some(id);
        changed
    }
}

/// One show as the schedule serves it: the answer to "when is the show", scheduled or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Showing {
    /// A materialised show of the programme.
    Scheduled(Show),
    /// Without a programme: a show starting a lead from now.
    Unscheduled {
        session_id: &'static str,
        start: SystemTime,
        end: SystemTime,
    },
}

impl Showing {
    pub fn start(&self) -> SystemTime {
        match self {
            Self::Scheduled(show) => show.plan.start.into(),
            Self::Unscheduled { start, .. } => *start,
        }
    }

    pub fn end(&self) -> SystemTime {
        match self {
            Self::Scheduled(show) => show.plan.end.into(),
            Self::Unscheduled { end, .. } => *end,
        }
    }

    pub fn session_id(&self) -> String {
        match self {
            Self::Scheduled(show) => show.session_id.clone(),
            Self::Unscheduled { session_id, .. } => (*session_id).to_owned(),
        }
    }

    pub fn mode(&self) -> Mode {
        match self {
            Self::Scheduled(show) => show.plan.mode(),
            Self::Unscheduled { .. } => Mode::default(),
        }
    }

    /// Seconds from the show's start to `now`, negative before it.
    pub fn show_seconds(&self, now: SystemTime) -> i64 {
        Utc::from(now).unix_secs() - Utc::from(self.start()).unix_secs()
    }

    /// Log the handed-out show; warn past the Extended Play join limit.
    pub fn log_handed_out(&self, what: &str, now: SystemTime, peer: Option<std::net::IpAddr>) {
        let show_seconds = self.show_seconds(now);
        let (session, mode) = (self.session_id(), self.mode().as_str());
        if self.mode() == Mode::ExtendedPlay && show_seconds > EP_JOIN_LIMIT_SECS {
            tracing::warn!(what, %session, mode, show_seconds, limit = EP_JOIN_LIMIT_SECS, ?peer, "show handed out past the Extended Play join limit: a 1149 console joining now asserts");
        } else {
            tracing::debug!(what, %session, mode, show_seconds, ?peer, "show handed out");
        }
    }
}

/// Latest show time a v2.0.1149.0 console can join an Extended Play show; later asserts.
pub const EP_JOIN_LIMIT_SECS: i64 = 1_919;

fn looks_like_uuid(segment: &str) -> bool {
    let b = segment.as_bytes();
    b.len() == 36
        && [8, 13, 18, 23].iter().all(|&i| b[i] == b'-')
        && b.iter()
            .enumerate()
            .all(|(i, c)| [8, 13, 18, 23].contains(&i) || c.is_ascii_hexdigit())
}

/// The show a key names, by the id it carried (lowercased).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShowRef {
    Session(String),
    Episode(String),
}

impl ShowRef {
    pub fn id(&self) -> &str {
        match self {
            Self::Session(id) | Self::Episode(id) => id,
        }
    }
}

/// Rewrite a key's session/episode id to the canonical constants; returns the id found.
pub fn canonicalize_key(key: ParsedKey) -> (ParsedKey, Option<ShowRef>) {
    let Some(show_id) = key.show_id() else {
        return (key, None);
    };
    let canonical = match show_id.kind {
        ShowIdKind::Session => CANONICAL_SESSION,
        ShowIdKind::Episode => CANONICAL_EPISODE,
    };
    if !looks_like_uuid(show_id.id) || show_id.id.eq_ignore_ascii_case(canonical) {
        return (key, None);
    }
    let original = show_id.id.to_ascii_lowercase();
    let found = match show_id.kind {
        ShowIdKind::Session => ShowRef::Session(original),
        ShowIdKind::Episode => ShowRef::Episode(original),
    };
    (key.with_segment(show_id.index, canonical), Some(found))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::programme::PrimetimeSlots;

    /// v2.0.1149.0 `STATE_ExtendedPlay.PredictQuestions` (`stateextendedplay.lua:156`) with the
    /// values of `eptimings.lua`: the question it walks to for a join at `join_time`, and the
    /// stat break index it reaches (a 5th does not exist and asserts).
    fn predict_questions(join_time: f64) -> (u32, usize) {
        const INTRO: f64 = 0.0 + 30.0 + 10.0 + 29.0;
        const QUESTION: f64 = 4.0 + 4.0 + 4.0 + 4.0 + 2.5 + 6.8 + 4.0 + 6.0 + 2.0;
        const JIP_INTRO: f64 = 10.0 + 6.0;
        const BREAKS: [f64; 4] = [70.0, 115.0, 70.0, 30.0];
        let has_break =
            |question: u32| (question.is_multiple_of(9) && question != 36) || question == 37;
        let (mut question, mut index, mut start) = (0, 1, INTRO);
        while start < join_time + JIP_INTRO {
            question += 1;
            start += QUESTION;
            if has_break(question) {
                let Some(length) = BREAKS.get(index - 1) else {
                    return (question, index);
                };
                start += length;
                index += 1;
            }
        }
        (question, index)
    }

    #[test]
    fn the_extended_play_join_limit_is_where_the_1149_walk_runs_out_of_stat_breaks() {
        // The loading screen predicts at show time + 30 (join) + 30 (intro ad), `ads.lua:134`.
        let at = |show_time: i64| predict_questions(show_time as f64 + 60.0);
        assert!(at(EP_JOIN_LIMIT_SECS).1 <= 5 && at(EP_JOIN_LIMIT_SECS).0 < 45);
        assert_eq!(
            at(EP_JOIN_LIMIT_SECS + 1),
            (45, 5),
            "the 5th stat break asserts"
        );
        // Joins up to the last question are predicted; later ones are "past the end" (clean).
        assert_eq!(predict_questions(1650.0).0, 36);
        assert_eq!(predict_questions(1651.0).0, 37);
    }

    #[test]
    fn show_seconds_count_from_the_start_handed_out() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let showing = Showing::Unscheduled {
            session_id: "s",
            start,
            end: start + Duration::from_secs(3600),
        };
        assert_eq!(
            showing.show_seconds(start + Duration::from_secs(1919)),
            1919
        );
        assert_eq!(showing.show_seconds(start - Duration::from_secs(30)), -30);
    }

    #[test]
    fn random_uuids_are_version_4_and_differ() {
        let (a, b) = (random_uuid(), random_uuid());
        assert_ne!(a, b);
        for id in [a, b] {
            assert!(looks_like_uuid(&id), "{id}");
            assert_eq!(&id[14..15], "4", "{id}");
            assert!("89ab".contains(&id[19..20]), "{id}");
        }
    }
    #[test]
    fn modes_parse_and_map_to_the_titles_game_type() {
        assert_eq!(Mode::parse("pt"), Some(Mode::Primetime));
        assert_eq!(Mode::parse("ep"), Some(Mode::ExtendedPlay));
        assert_eq!(Mode::parse("EP"), None);
        // program.lua:133: GameType 1 = Game.Primetime, 2 = Game.ExtendedPlay.
        assert_eq!(Mode::Primetime.game_type(), 1);
        assert_eq!(Mode::ExtendedPlay.game_type(), 2);
        assert_eq!(Mode::default(), Mode::Primetime);
    }

    fn utc(text: &str) -> Utc {
        Utc::parse(text).unwrap()
    }

    /// A schedule with the default programme, saved in memory.
    fn programmed() -> (Schedule, Arc<MemoryStorage>) {
        let storage = Arc::new(MemoryStorage::default());
        storage.set_programme(&Programme::default()).unwrap();
        let schedule = Schedule::load(ScheduleConfig::default(), storage.clone()).unwrap();
        (schedule, storage)
    }

    #[test]
    fn a_saved_programme_with_long_extended_play_is_cut_to_one_episode() {
        let (mut schedule, storage) = programmed();
        let at = utc("2026-10-23T12:40:00Z");
        let listed: Vec<_> = schedule.shows(at)[..2].to_vec();
        // The storage refuses a programme whose Extended Play shows run past one episode, so set
        // it directly.
        schedule.programme = Some(Programme {
            extended_play: programme::minutes(120),
            ..Programme::default()
        });
        assert!(schedule.fit_extended_play_to_one_episode(at).unwrap());
        let saved = storage.programme().unwrap().unwrap();
        assert_eq!(saved.extended_play, programme::EP_EPISODE);
        let shows = schedule.shows(at);
        assert_eq!(
            shows[..2],
            listed[..],
            "the listed shows stay as consoles know them"
        );
        assert!(shows[2..].iter().all(|show| show.plan.minutes()
            <= u64::from(programme::whole_minutes(programme::EP_EPISODE))
            || show.plan.mode() == Mode::Primetime));
        assert!(!schedule.fit_extended_play_to_one_episode(at).unwrap());
    }

    #[test]
    fn season_ids_are_one_per_month() {
        let october = season_id(utc("2026-10-01T00:00:00Z"));
        assert_eq!(october, season_id(utc("2026-10-31T23:00:00Z")));
        assert_ne!(october, season_id(utc("2026-11-01T00:00:00Z")));
        assert!(looks_like_uuid(&october));
    }

    #[test]
    fn without_a_programme_the_shows_are_the_canonical_and_the_next_session() {
        let mut schedule = Schedule::default();
        let now = SystemTime::from(utc("2026-10-23T12:00:00Z"));
        let unscheduled = |session_id, offset| {
            let (start, end) = unscheduled_window(now, offset);
            Showing::Unscheduled {
                session_id,
                start,
                end,
            }
        };
        let current = unscheduled(CANONICAL_SESSION, 0);
        let next = unscheduled(UNSCHEDULED_NEXT_SESSION, 1);
        assert_eq!(schedule.showing(now, 0), current);
        // Alone, a later show keeps the canonical id; in a pair it is the "Next" session.
        assert_eq!(schedule.showing(now, 1), unscheduled(CANONICAL_SESSION, 1));
        assert_eq!(
            schedule.showing_pair(now, 0),
            [current.clone(), next.clone()]
        );
        assert_eq!(schedule.showing_for_session(now, None), current);
        assert_eq!(
            schedule.showing_for_session(now, Some(UNSCHEDULED_NEXT_SESSION)),
            next
        );
        assert_eq!(schedule.current_session_id(now), CANONICAL_SESSION);
        assert!(!schedule.is_enabled());
        assert!(!schedule.activate_session(now, CANONICAL_SESSION));
        assert!(
            schedule
                .start_show_now(
                    utc("2026-10-23T12:00:00Z"),
                    Mode::Primetime,
                    programme::minutes(30),
                )
                .is_err()
        );
    }

    #[test]
    fn with_a_programme_the_shows_are_materialised_saved_and_served() {
        let (mut schedule, storage) = programmed();
        let at = utc("2026-10-23T12:30:00Z");
        let now = SystemTime::from(at);
        let [current, next] = schedule.showing_pair(now, 0);
        let (Showing::Scheduled(current), Showing::Scheduled(next)) = (current, next) else {
            panic!("scheduled shows");
        };
        assert!(current.plan.start <= at && at < current.plan.end);
        assert_eq!(current.plan.end, next.plan.start);
        assert!(looks_like_uuid(&current.session_id));
        assert_ne!(current.session_id, next.session_id);
        assert_eq!(schedule.current_session_id(now), current.session_id);
        assert_eq!(
            schedule.showing_for_session(now, Some(&next.session_id.to_uppercase())),
            Showing::Scheduled(next.clone())
        );
        assert_eq!(
            schedule.showing_for_session(now, Some("unknown")),
            Showing::Scheduled(current.clone()),
            "an unknown id means the show running now"
        );

        // Saved: a restart serves the same shows with the same ids.
        assert_eq!(storage.shows().unwrap(), schedule.shows(at).to_vec());
        let mut restarted = Schedule::load(ScheduleConfig::default(), storage.clone()).unwrap();
        assert_eq!(
            restarted.current_session_id(now),
            current.session_id,
            "same id after a restart"
        );
    }

    #[test]
    fn a_new_show_switches_once_it_has_started() {
        let (mut schedule, _) = programmed();
        let at = utc("2026-10-23T12:30:00Z");
        let [current, next] = schedule.showing_pair(SystemTime::from(at), 0);
        let (current, next) = (current.session_id(), next.session_id());
        let now = SystemTime::from(at);
        assert!(
            !schedule.activate_session(now, &current),
            "the first show seen"
        );
        assert!(!schedule.activate_session(now, &next), "not started yet");
        assert!(!schedule.activate_session(now, "unknown"));
        let Showing::Scheduled(next_show) = schedule.showing(now, 1) else {
            panic!("scheduled");
        };
        let later = SystemTime::from(next_show.plan.start);
        assert!(schedule.activate_session(later, &next), "a new show");
        assert!(!schedule.activate_session(later, &next), "the same show");
    }

    #[test]
    fn changing_the_programme_keeps_the_listed_shows() {
        let (mut schedule, storage) = programmed();
        let at = utc("2026-10-23T12:30:00Z");
        let before = schedule.shows(at).to_vec();
        let later_primetime = Programme {
            primetime_at: PrimetimeSlots::parse("21:00").unwrap(),
            ..Programme::default()
        };
        schedule.set_programme(later_primetime.clone(), at).unwrap();
        let after = schedule.shows(at).to_vec();
        assert_eq!(after[..2], before[..2], "running and next stay");
        assert_ne!(after[2], before[2], "the rest is planned again");
        assert_eq!(storage.shows().unwrap(), after);
        assert_eq!(storage.programme().unwrap(), Some(later_primetime));
        let primetime_starts: Vec<_> = after
            .iter()
            .filter(|show| show.plan.kind == ShowKind::Primetime)
            .map(|show| Programme::default().zone.local_text(show.plan.start))
            .collect();
        assert!(
            primetime_starts
                .iter()
                .all(|start| start.ends_with("21:00")),
            "{primetime_starts:?}"
        );

        let invalid = Programme {
            primetime: programme::minutes(1),
            ..Programme::default()
        };
        assert!(matches!(
            schedule.set_programme(invalid, at),
            Err(ScheduleError::Invalid {
                source: ValidationError::Length {
                    setting: "Primetime",
                    length
                }
            }) if length == programme::minutes(1)
        ));
    }

    #[test]
    fn a_special_tomorrow_is_planned_in_and_out() {
        let (mut schedule, storage) = programmed();
        let at = utc("2026-10-23T12:30:00Z");
        let tomorrow = Programme::default().zone.date_of(at).next_day().unwrap();
        let special = SpecialEpisode {
            date: tomorrow,
            at: crate::time::LocalTime::from_hm(20, 0).unwrap(),
            title: "Quiz Night".into(),
            category: "otdb:9".into(),
            length: programme::minutes(60),
            mode: crate::schedule::Mode::Primetime,
            artwork: None,
        };
        schedule.put_special(special.clone(), at).unwrap();
        let has_special = |shows: &[Show]| {
            shows
                .iter()
                .any(|show| show.plan.category() == Some("otdb:9"))
        };
        assert!(has_special(schedule.shows(at)));
        assert_eq!(schedule.specials(), std::slice::from_ref(&special));
        assert!(has_special(&storage.shows().unwrap()));
        assert!(schedule.delete_special(special.slot(), at).unwrap());
        assert!(!schedule.delete_special(special.slot(), at).unwrap());
        assert!(!has_special(schedule.shows(at)));
    }

    #[test]
    fn a_show_started_by_hand_ends_the_running_one_and_the_programme_follows() {
        let (mut schedule, storage) = programmed();
        let at = utc("2026-10-23T12:40:00Z");
        let running = schedule.shows(at)[0].clone();
        let manual = schedule
            .start_show_now(at, Mode::Primetime, programme::minutes(20))
            .unwrap();
        assert!(manual.manual);
        let shows = schedule.shows(at).to_vec();
        assert_eq!(shows[0].session_id, running.session_id);
        assert_eq!(shows[0].plan.end, at, "the running show ends now");
        assert_eq!(shows[1], manual);
        assert_eq!(shows[2].plan.start, at + Duration::from_secs(20 * 60));
        for pair in shows.windows(2) {
            assert_eq!(pair[0].plan.end, pair[1].plan.start);
        }
        assert_eq!(storage.shows().unwrap(), shows);
        assert_eq!(
            schedule.current_session_id(SystemTime::from(at)),
            manual.session_id
        );
        assert!(
            schedule
                .start_show_now(at, Mode::ExtendedPlay, Duration::ZERO)
                .is_err()
        );
        assert!(
            schedule
                .start_show_now(
                    at,
                    Mode::ExtendedPlay,
                    programme::EP_EPISODE + Duration::from_secs(60)
                )
                .is_err(),
            "an Extended Play show is one episode"
        );
    }

    #[test]
    fn shows_older_than_a_week_are_dropped() {
        let (mut schedule, storage) = programmed();
        let at = utc("2026-10-01T12:30:00Z");
        let first = schedule.shows(at)[0].session_id.clone();
        let much_later = at + Duration::from_secs(10 * 86_400);
        let shows = schedule.shows(much_later).to_vec();
        assert!(shows.iter().all(|show| show.session_id != first));
        assert!(shows[0].plan.end >= much_later - HISTORY);
        assert_eq!(storage.shows().unwrap(), shows);
    }

    fn canon(key: String) -> (String, Option<ShowRef>) {
        let (out, found) = canonicalize_key(ParsedKey::parse(&key));
        (out.raw, found)
    }

    #[test]
    fn keys_are_normalised_to_the_canonical_session() {
        let id = "0a1b2c3d-1111-5222-8333-444455556666";
        let (key, found) = canon(format!("v3.Session.MGS.1vs100.Store.{id}.GameState"));
        assert_eq!(
            key,
            format!("v3.Session.MGS.1vs100.Store.{CANONICAL_SESSION}.GameState")
        );
        assert_eq!(found, Some(ShowRef::Session(id.into())));
        let (key, _) = canon(format!(
            "v3.Season.MGS.1vs100.TopN.Prizing.Session.{id}.Round.0.RoundEndTopScores"
        ));
        assert_eq!(
            key,
            format!(
                "v3.Season.MGS.1vs100.TopN.Prizing.Session.{CANONICAL_SESSION}.Round.0.RoundEndTopScores"
            )
        );
        let (key, found) = canon(format!(
            "v3.Setup.MGS.1vs100.Store.Sessions.{id}.MergedLaunchData"
        ));
        assert_eq!(
            key,
            format!("v3.Setup.MGS.1vs100.Store.Sessions.{CANONICAL_SESSION}.MergedLaunchData")
        );
        assert_eq!(found, Some(ShowRef::Session(id.into())));
        let (key, found) = canon(format!("v3.Setup.MGS.1vs100.Store.{id}.AvailableLanguages"));
        assert_eq!(
            key,
            format!("v3.Setup.MGS.1vs100.Store.{CANONICAL_EPISODE}.AvailableLanguages")
        );
        assert_eq!(found, Some(ShowRef::Episode(id.into())));
        let upper = id.to_ascii_uppercase();
        let (key, found) = canon(format!(
            "v3.Setup.MGS.1vs100.Store.QuestionBank.{upper}.Locale.en-US.Difficulty.3.ResetID"
        ));
        assert_eq!(
            key,
            format!(
                "v3.Setup.MGS.1vs100.Store.QuestionBank.{CANONICAL_EPISODE}.Locale.en-US.Difficulty.3.ResetID"
            )
        );
        assert_eq!(found, Some(ShowRef::Episode(id.into())));
        // untouched: canonical ids, other keys, non-uuid segments
        for untouched in [
            format!("v3.Session.MGS.1vs100.Store.{CANONICAL_SESSION}.GameState"),
            "v3.Setup.MGS.1vs100.Store.Config".to_owned(),
            "v3.Session.MGS.1vs100.Sum.AnswerCount".to_owned(),
            "v3.Session.MGS.1vs100.Store.P.GameState".to_owned(),
            "v3.Setup.MGS.1vs100.Store.QuestionBank.Difficulty.1.ResetID".to_owned(),
        ] {
            let (key, found) = canon(untouched.clone());
            assert_eq!(key, untouched);
            assert!(found.is_none());
        }
    }
}
