//! Daily programme: Primetime shows at fixed times each day, Extended Play back to back between
//! them, optional specials.
//! [`plan`] is pure; the schedule saves the programme and serves the shows.

use std::{fmt, time::Duration};

use crate::{
    schedule::Mode,
    time::{LocalDate, LocalTime, Utc, Zone},
};

/// Owner settings: when Primetime is, and how long shows are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Programme {
    pub zone: Zone,
    /// Daily Primetime starts in `zone`.
    pub primetime_at: PrimetimeSlots,
    pub primetime: Duration,
    pub extended_play: Duration,
    /// Shortest Extended Play show worth starting.
    pub min_extended_play: Duration,
}

impl Default for Programme {
    /// 8 PM Eastern, Primetime 60 minutes, Extended Play one episode.
    fn default() -> Self {
        Self {
            zone: Zone::get("America/New_York").expect("the compiled-in database knows New York"),
            primetime_at: PrimetimeSlots::one(LocalTime::from_hm(20, 0).expect("20:00 is a time")),
            primetime: minutes(60),
            extended_play: EP_EPISODE,
            min_extended_play: minutes(30),
        }
    }
}

/// The daily Primetime starts: at least one, each once, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrimetimeSlots(Vec<LocalTime>);

impl PrimetimeSlots {
    pub fn one(at: LocalTime) -> Self {
        Self(vec![at])
    }

    pub fn new(mut times: Vec<LocalTime>) -> Result<Self, SlotsError> {
        times.sort_unstable();
        if times.is_empty() {
            return Err(SlotsError::Empty);
        }
        if let Some(pair) = times.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(SlotsError::Repeated { at: pair[0] });
        }
        Ok(Self(times))
    }

    /// `HH:MM` times separated by commas: `19:00,22:00`.
    pub fn parse(text: &str) -> Result<Self, SlotsError> {
        text.split(',')
            .map(|at| {
                LocalTime::parse(at.trim()).ok_or_else(|| SlotsError::NotATime {
                    text: at.trim().to_owned(),
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .and_then(Self::new)
    }

    pub fn times(&self) -> &[LocalTime] {
        &self.0
    }

    pub fn first(&self) -> LocalTime {
        self.0[0]
    }

    /// Each start with the next one, the last with the first of the next day.
    fn with_next(&self) -> impl Iterator<Item = SlotPair> + '_ {
        let next = self.0.iter().skip(1).chain(std::iter::once(&self.0[0]));
        self.0
            .iter()
            .zip(next)
            .map(|(&at, &next)| SlotPair { at, next })
    }

    /// The start after `at`, the next day's first after the last.
    fn next_after(&self, at: LocalTime) -> Option<LocalTime> {
        self.with_next()
            .find(|pair| pair.at == at)
            .map(|pair| pair.next)
    }
}

struct SlotPair {
    at: LocalTime,
    next: LocalTime,
}

/// Why a list of Primetime starts is refused.
#[derive(Debug, PartialEq, Eq)]
pub enum SlotsError {
    Empty,
    NotATime { text: String },
    Repeated { at: LocalTime },
}

impl fmt::Display for SlotsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("no Primetime start given"),
            Self::NotATime { text } => write!(f, "{text:?} is not a time HH:MM"),
            Self::Repeated { at } => write!(f, "Primetime start {at} given twice"),
        }
    }
}

impl std::error::Error for SlotsError {}

impl fmt::Display for PrimetimeSlots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, at) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            at.fmt(f)?;
        }
        Ok(())
    }
}

/// Time from `from` to the next `to` on the wall clock of a day without a clock change.
fn wall_clock_gap(from: LocalTime, to: LocalTime) -> Duration {
    const DAY: Duration = Duration::from_secs(86_400);
    let (from, to) = (from.since_midnight(), to.since_midnight());
    if to > from {
        to - from
    } else {
        to + DAY - from
    }
}

/// Shortest show allowed.
pub const MIN_SHOW: Duration = minutes(15);

/// Longest show allowed.
pub const MAX_SHOW: Duration = minutes(240);

/// Longest Extended Play show: one episode; longer asserts on join.
pub const EP_EPISODE: Duration = minutes(30);

/// A length of `minutes` whole minutes.
pub const fn minutes(minutes: u32) -> Duration {
    Duration::from_secs(minutes as u64 * 60)
}

/// The whole minutes a length spans.
pub fn whole_minutes(length: Duration) -> u32 {
    // A length is set with `minutes(u32)`, so the cap only keeps a longer one from wrapping.
    u32::try_from(length.as_secs() / 60).unwrap_or(u32::MAX)
}

/// Why a programme, special episode, theme or show does not pass its checks.
#[derive(Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// A programme length outside [`MIN_SHOW`]..=[`MAX_SHOW`]; the setting and the length given.
    Length {
        setting: &'static str,
        length: Duration,
    },
    /// An Extended Play show of more than one episode; the length given.
    ExtendedPlayLength { length: Duration },
    /// The shortest Extended Play show longer than a whole one; both lengths given.
    ShortestLength { shortest: Duration, whole: Duration },
    /// A special episode without a title.
    MissingTitle,
    /// A special episode without a question category.
    MissingCategory,
    /// A special episode's length outside [`MIN_SHOW`]..=its mode's longest; the mode and length
    /// given.
    SpecialLength { mode: Mode, length: Duration },
    /// Theme artwork that is not letters, digits and underscores; the name given.
    Artwork { name: String },
    /// A show started by hand, and no programme to schedule it.
    NoProgramme,
    /// A show started by hand of zero length or more than [`MAX_SHOW`]; the length given.
    ShowLength { length: Duration },
    /// A Primetime show that runs into the next one; both starts given.
    PrimetimeOverlap { at: LocalTime, next: LocalTime },
    /// Time before a Primetime start that whole Extended Play shows leave over and that is
    /// shorter than the shortest one; the start and the time left over given.
    UnfilledGap { before: LocalTime, left: Duration },
    /// A special episode for a time the programme has no Primetime start at; the time given.
    NoPrimetimeAt { at: LocalTime },
    /// A special episode that runs into the next Primetime start; its length and that start.
    SpecialRunsOver { length: Duration, next: LocalTime },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { setting, length } => write!(
                f,
                "{setting} length must be {}..={} minutes, got {}",
                whole_minutes(MIN_SHOW),
                whole_minutes(MAX_SHOW),
                whole_minutes(*length)
            ),
            Self::ExtendedPlayLength { length } => write!(
                f,
                "an Extended Play show is one episode, at most {} minutes, got {}",
                whole_minutes(EP_EPISODE),
                whole_minutes(*length)
            ),
            Self::ShortestLength { shortest, whole } => write!(
                f,
                "the shortest Extended Play show is {} minutes, longer than the whole one's {}",
                whole_minutes(*shortest),
                whole_minutes(*whole)
            ),
            Self::MissingTitle => f.write_str("a special episode needs a title"),
            Self::MissingCategory => f.write_str("a special episode needs a question category"),
            Self::SpecialLength { mode, length } => {
                let longest = match mode {
                    Mode::Primetime => MAX_SHOW,
                    Mode::ExtendedPlay => EP_EPISODE,
                };
                write!(
                    f,
                    "a {} special episode is {}..={} minutes long, got {}",
                    mode.as_str(),
                    whole_minutes(MIN_SHOW),
                    whole_minutes(longest),
                    whole_minutes(*length)
                )
            }
            Self::Artwork { name } => {
                write!(
                    f,
                    "theme artwork {name:?} must be letters, digits and underscores"
                )
            }
            Self::NoProgramme => f.write_str("no programme: shows are not scheduled"),
            Self::ShowLength { length } => write!(
                f,
                "a show is 1..={} minutes long, got {}",
                whole_minutes(MAX_SHOW),
                whole_minutes(*length)
            ),
            Self::PrimetimeOverlap { at, next } => {
                write!(f, "the Primetime show at {at} runs into the one at {next}")
            }
            Self::NoPrimetimeAt { at } => write!(f, "the programme has no Primetime show at {at}"),
            Self::SpecialRunsOver { length, next } => write!(
                f,
                "a {} minute special episode runs into the Primetime show at {next}",
                whole_minutes(*length)
            ),
            Self::UnfilledGap { before, left } => write!(
                f,
                "{} minutes before the Primetime show at {before} are shorter than the shortest \
                 Extended Play show and would have no show",
                whole_minutes(*left)
            ),
        }
    }
}

impl std::error::Error for ValidationError {}

impl Programme {
    /// Lengths the planner accepts.
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (setting, length) in [
            ("Primetime", self.primetime),
            ("Extended Play", self.extended_play),
            ("shortest Extended Play", self.min_extended_play),
        ] {
            if !(MIN_SHOW..=MAX_SHOW).contains(&length) {
                return Err(ValidationError::Length { setting, length });
            }
        }
        if self.extended_play > EP_EPISODE {
            return Err(ValidationError::ExtendedPlayLength {
                length: self.extended_play,
            });
        }
        if self.min_extended_play > self.extended_play {
            return Err(ValidationError::ShortestLength {
                shortest: self.min_extended_play,
                whole: self.extended_play,
            });
        }
        for SlotPair { at, next } in self.primetime_at.with_next() {
            if wall_clock_gap(at, next) < self.primetime {
                return Err(ValidationError::PrimetimeOverlap { at, next });
            }
        }
        Ok(())
    }

    /// [`Programme::validate`], and every gap between Primetime shows filled by Extended Play
    /// shows. Only a change is held to the gap check: saved programmes and the settings load
    /// without it.
    pub fn validate_new(&self) -> Result<(), ValidationError> {
        self.validate()?;
        for SlotPair { at, next } in self.primetime_at.with_next() {
            let gap = wall_clock_gap(at, next).saturating_sub(self.primetime);
            let left = Duration::from_secs(gap.as_secs() % self.extended_play.as_secs());
            if !left.is_zero() && left < self.min_extended_play {
                return Err(ValidationError::UnfilledGap { before: next, left });
            }
        }
        Ok(())
    }

    /// A special replaces one of the programme's Primetime shows and ends before the next.
    pub fn fits(&self, special: &SpecialEpisode) -> Result<(), ValidationError> {
        let at = special.at;
        match self.primetime_at.next_after(at) {
            None => Err(ValidationError::NoPrimetimeAt { at }),
            Some(next) if wall_clock_gap(at, next) < special.length => {
                Err(ValidationError::SpecialRunsOver {
                    length: special.length,
                    next,
                })
            }
            Some(_) => Ok(()),
        }
    }

    /// Cap Extended Play shows at one episode; `true` when changed.
    pub fn with_one_episode_extended_play(mut self) -> (Self, bool) {
        let changed = self.extended_play > EP_EPISODE;
        if changed {
            self.extended_play = EP_EPISODE;
            self.min_extended_play = self.min_extended_play.min(EP_EPISODE);
        }
        (self, changed)
    }
}

/// Theme name that picks an episode package's theme art (`media/themes/MainTheme_<name>.png`,
/// `media/set/EPTheme_<name>.mb.bin`), e.g. `Community`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeArtwork(String);

impl ThemeArtwork {
    pub fn new(name: impl Into<String>) -> Result<Self, ValidationError> {
        let name = name.into();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ValidationError::Artwork { name });
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The Primetime show a special episode replaces: the one at `at` on `date`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpecialSlot {
    pub date: LocalDate,
    pub at: LocalTime,
}

/// Themed episode replacing the Primetime show at `at` on `date`. Planned only while the
/// programme has a Primetime start at `at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecialEpisode {
    pub date: LocalDate,
    pub at: LocalTime,
    /// Shown as the show's theme: the lobby marquee and the Extended Play screens.
    pub title: String,
    /// A question bank category (`otdb:9`).
    pub category: String,
    pub length: Duration,
    /// The title shows the theme in Extended Play only; Primetime keeps its own marquee text.
    pub mode: Mode,
    pub artwork: Option<ThemeArtwork>,
}

impl SpecialEpisode {
    pub fn slot(&self) -> SpecialSlot {
        SpecialSlot {
            date: self.date,
            at: self.at,
        }
    }

    /// A title and a category, and a length within [`MIN_SHOW`]..=[`MAX_SHOW`], or one episode for
    /// Extended Play.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.title.trim().is_empty() {
            return Err(ValidationError::MissingTitle);
        }
        if self.category.trim().is_empty() {
            return Err(ValidationError::MissingCategory);
        }
        let longest = match self.mode {
            Mode::Primetime => MAX_SHOW,
            Mode::ExtendedPlay => EP_EPISODE,
        };
        if !(MIN_SHOW..=longest).contains(&self.length) {
            return Err(ValidationError::SpecialLength {
                mode: self.mode,
                length: self.length,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShowKind {
    Primetime,
    /// A special episode in a Primetime show's place.
    Special {
        title: String,
        category: String,
        mode: Mode,
        artwork: Option<ThemeArtwork>,
    },
    ExtendedPlay,
}

impl ShowKind {
    /// `primetime`, `special` or `extended_play` (storage, admin API).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Primetime => "primetime",
            Self::Special { .. } => "special",
            Self::ExtendedPlay => "extended_play",
        }
    }
}

/// One show of the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedShow {
    pub start: Utc,
    pub end: Utc,
    pub kind: ShowKind,
}

impl PlannedShow {
    /// Title `GameType`.
    pub fn mode(&self) -> Mode {
        match self.kind {
            ShowKind::Primetime => Mode::Primetime,
            ShowKind::Special { mode, .. } => mode,
            ShowKind::ExtendedPlay => Mode::ExtendedPlay,
        }
    }

    pub fn minutes(&self) -> u64 {
        self.end.saturating_duration_since(self.start).as_secs() / 60
    }

    /// Question category, if special.
    pub fn category(&self) -> Option<&str> {
        match &self.kind {
            ShowKind::Special { category, .. } => Some(category),
            _ => None,
        }
    }
}

/// Materialisation horizon: Grid lists current plus next.
pub const HORIZON: Duration = Duration::from_secs(48 * 3600);

/// Committed show: has ids; consoles may know it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Show {
    /// Scopes the show's SBG keys and matchmaking roster.
    pub session_id: String,
    pub episode_id: String,
    pub plan: PlannedShow,
    /// Started by hand, not planned.
    pub manual: bool,
}

/// Top up `shows` to `now + HORIZON` from the last show's end, or from `now`.
pub fn extend(
    programme: &Programme,
    specials: &[SpecialEpisode],
    shows: &[Show],
    now: Utc,
    new_id: &mut dyn FnMut() -> String,
) -> Vec<Show> {
    // Planning goes on from now; ended shows are history.
    let floor = shows.last().map(|last| last.plan.end);
    let from = floor.map_or(now, |end| end.max(now));
    let until = now + HORIZON;
    if from >= until {
        return Vec::new();
    }
    plan(programme, specials, from, until)
        .into_iter()
        .filter(|planned| planned.end > from)
        .map(|mut planned| {
            if let Some(floor) = floor {
                planned.start = planned.start.max(floor);
            }
            Show {
                session_id: new_id(),
                episode_id: new_id(),
                plan: planned,
                manual: false,
            }
        })
        .collect()
}

/// Shows a programme change keeps: past ones plus the two the Grid lists.
pub fn frozen(shows: &[Show], now: Utc) -> usize {
    shows
        .iter()
        .position(|show| show.plan.end > now)
        .map_or(shows.len(), |listed| (listed + 2).min(shows.len()))
}

/// Every show overlapping `from..until`, in order; Extended Play fills Primetime gaps.
pub fn plan(
    programme: &Programme,
    specials: &[SpecialEpisode],
    from: Utc,
    until: Utc,
) -> Vec<PlannedShow> {
    // Bound the range with one Primetime show each side to learn both end gaps.
    let zone = &programme.zone;
    let mut day = zone.date_of(from).previous_day();
    let last = zone.date_of(until).next_day().and_then(LocalDate::next_day);
    let mut anchors: Vec<PlannedShow> = Vec::new();
    while let Some(date) = day {
        if last.is_some_and(|last| date > last) {
            break;
        }
        for &at in programme.primetime_at.times() {
            let Some(start) = zone.at(date, at) else {
                continue;
            };
            let special = specials
                .iter()
                .find(|special| special.slot() == SpecialSlot { date, at });
            let (kind, length) = match special {
                Some(special) => (
                    ShowKind::Special {
                        title: special.title.clone(),
                        category: special.category.clone(),
                        mode: special.mode,
                        artwork: special.artwork.clone(),
                    },
                    special.length,
                ),
                None => (ShowKind::Primetime, programme.primetime),
            };
            // A clock change, or a programme change after a special was saved, can bring the next
            // start inside a show.
            if let Some(previous) = anchors.last_mut() {
                previous.end = previous.end.min(start);
            }
            anchors.push(PlannedShow {
                start,
                end: start + length,
                kind,
            });
        }
        day = date.next_day();
    }

    let mut shows = Vec::new();
    for pair in anchors.windows(2) {
        shows.push(pair[0].clone());
        fill_extended_play(programme, pair[0].end, pair[1].start, &mut shows);
    }
    shows.extend(anchors.last().cloned());
    shows.retain(|show| show.end > from && show.start < until);
    shows
}

/// Extended Play shows from `start` to `end`: whole ones, then the rest as one shorter.
fn fill_extended_play(programme: &Programme, start: Utc, end: Utc, shows: &mut Vec<PlannedShow>) {
    let whole = programme.extended_play;
    let shortest = programme.min_extended_play;
    let mut at = start;
    while end.saturating_duration_since(at) >= whole {
        shows.push(PlannedShow {
            start: at,
            end: at + whole,
            kind: ShowKind::ExtendedPlay,
        });
        at = at + whole;
    }
    let rest = end.saturating_duration_since(at);
    if rest.is_zero() {
        return;
    }
    if rest >= shortest {
        shows.push(PlannedShow {
            start: at,
            end,
            kind: ShowKind::ExtendedPlay,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> Utc {
        Utc::parse(text).unwrap()
    }

    fn date(text: &str) -> LocalDate {
        LocalDate::parse(text).unwrap()
    }

    /// The plan of one New York day, from its midnight to the next.
    fn day(programme: &Programme, specials: &[SpecialEpisode], on: &str) -> Vec<PlannedShow> {
        let zone = &programme.zone;
        let from = zone.at(date(on), LocalTime::MIDNIGHT).unwrap();
        let until = zone
            .at(date(on).next_day().unwrap(), LocalTime::MIDNIGHT)
            .unwrap();
        plan(programme, specials, from, until)
    }

    fn lengths(shows: &[PlannedShow]) -> Vec<(u64, Mode)> {
        shows.iter().map(|s| (s.minutes(), s.mode())).collect()
    }

    /// Shows follow each other without a gap or an overlap.
    fn assert_back_to_back(shows: &[PlannedShow]) {
        for pair in shows.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "{pair:?}");
        }
    }

    #[test]
    fn a_normal_day_is_primetime_at_eight_and_extended_play_around_it() {
        let programme = Programme::default();
        let shows = plan(
            &programme,
            &[],
            utc("2026-10-23T01:00:00Z"), // 21:00 EDT on the 22nd, when Primetime ends
            utc("2026-10-24T01:00:00Z"),
        );
        assert_back_to_back(&shows);
        // 23 hours of one-episode Extended Play shows, then Primetime.
        let mut expected = vec![(u64::from(whole_minutes(EP_EPISODE)), Mode::ExtendedPlay); 46];
        expected.push((60, Mode::Primetime));
        assert_eq!(lengths(&shows), expected);
        assert_eq!(shows[0].start, utc("2026-10-23T01:00:00Z"));
        assert_eq!(shows[46].start, utc("2026-10-24T00:00:00Z"), "20:00 EDT");
    }

    #[test]
    fn each_primetime_slot_is_a_show_and_extended_play_fills_between_them() {
        let programme = Programme {
            primetime_at: PrimetimeSlots::parse("22:00,19:00").unwrap(),
            ..Programme::default()
        };
        let shows = day(&programme, &[], "2026-10-23");
        assert_back_to_back(&shows);
        let primetime: Vec<_> = shows
            .iter()
            .filter(|s| s.mode() == Mode::Primetime)
            .map(|s| programme.zone.local_text(s.start))
            .collect();
        assert_eq!(primetime, ["2026-10-23 19:00", "2026-10-23 22:00"]);
        let between = shows
            .iter()
            .filter(|s| {
                s.start >= utc("2026-10-23T23:00:00Z") && s.end <= utc("2026-10-24T03:00:00Z")
            })
            .map(|s| (s.minutes(), s.mode()))
            .collect::<Vec<_>>();
        assert_eq!(
            between,
            [
                (60, Mode::Primetime),
                (30, Mode::ExtendedPlay),
                (30, Mode::ExtendedPlay),
                (30, Mode::ExtendedPlay),
                (30, Mode::ExtendedPlay),
                (60, Mode::Primetime),
            ]
        );
    }

    #[test]
    fn back_to_back_primetime_slots_have_no_extended_play_between_them() {
        let programme = Programme {
            primetime_at: PrimetimeSlots::parse("20:00,21:00").unwrap(),
            ..Programme::default()
        };
        let shows = day(&programme, &[], "2026-10-23");
        let at_nine = shows
            .iter()
            .position(|s| programme.zone.local_text(s.start) == "2026-10-23 21:00")
            .unwrap();
        assert_eq!(shows[at_nine - 1].mode(), Mode::Primetime);
        assert_eq!(shows[at_nine].mode(), Mode::Primetime);
    }

    #[test]
    fn slots_are_sorted_and_each_given_once() {
        let slots = PrimetimeSlots::parse("22:00, 19:00").unwrap();
        assert_eq!(slots.to_string(), "19:00,22:00");
        assert_eq!(
            PrimetimeSlots::parse("19:00,19:00"),
            Err(SlotsError::Repeated {
                at: LocalTime::from_hm(19, 0).unwrap()
            })
        );
        for bad in ["", "19:00,7pm", "19:00,"] {
            assert!(
                matches!(PrimetimeSlots::parse(bad), Err(SlotsError::NotATime { .. })),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn slots_that_overlap_or_leave_a_gap_without_a_show_are_refused() {
        let with = |slots: &str| Programme {
            primetime_at: PrimetimeSlots::parse(slots).unwrap(),
            ..Programme::default()
        };
        assert_eq!(
            with("19:00,19:30").validate(),
            Err(ValidationError::PrimetimeOverlap {
                at: LocalTime::from_hm(19, 0).unwrap(),
                next: LocalTime::from_hm(19, 30).unwrap(),
            })
        );
        // The last show of the day runs past midnight into the first.
        assert!(matches!(
            with("00:30,23:45").validate(),
            Err(ValidationError::PrimetimeOverlap { .. })
        ));
        assert_eq!(
            with("19:00,20:15").validate_new(),
            Err(ValidationError::UnfilledGap {
                before: LocalTime::from_hm(20, 15).unwrap(),
                left: minutes(15),
            })
        );
        // Loading does not check gaps.
        assert_eq!(with("19:00,20:15").validate(), Ok(()));
        assert_eq!(with("19:00,20:00,21:30").validate_new(), Ok(()));
    }

    #[test]
    fn a_special_must_end_before_the_next_primetime_start() {
        let programme = Programme {
            primetime_at: PrimetimeSlots::parse("19:00,21:00").unwrap(),
            ..Programme::default()
        };
        let special = |length| SpecialEpisode {
            date: date("2026-12-24"),
            at: LocalTime::from_hm(19, 0).unwrap(),
            title: "Holiday".into(),
            category: "otdb:9".into(),
            length,
            mode: Mode::Primetime,
            artwork: None,
        };
        assert_eq!(programme.fits(&special(minutes(120))), Ok(()));
        assert_eq!(
            programme.fits(&special(minutes(121))),
            Err(ValidationError::SpecialRunsOver {
                length: minutes(121),
                next: LocalTime::from_hm(21, 0).unwrap(),
            })
        );
        let eight = LocalTime::from_hm(20, 0).unwrap();
        assert_eq!(
            programme.fits(&SpecialEpisode {
                at: eight,
                ..special(minutes(60))
            }),
            Err(ValidationError::NoPrimetimeAt { at: eight })
        );
    }

    #[test]
    fn a_special_replaces_only_the_primetime_show_at_its_start() {
        let programme = Programme {
            primetime_at: PrimetimeSlots::parse("19:00,22:00").unwrap(),
            ..Programme::default()
        };
        let special = |at| SpecialEpisode {
            date: date("2026-10-23"),
            at,
            title: "Movie Night".into(),
            category: "otdb:11".into(),
            length: minutes(60),
            mode: Mode::Primetime,
            artwork: None,
        };
        let primetime = |specials: &[SpecialEpisode]| {
            day(&programme, specials, "2026-10-23")
                .into_iter()
                .filter(|s| s.kind != ShowKind::ExtendedPlay)
                .map(|s| (programme.zone.local_text(s.start), s.kind.as_str()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            primetime(&[special(LocalTime::from_hm(22, 0).unwrap())]),
            [
                ("2026-10-23 19:00".to_owned(), "primetime"),
                ("2026-10-23 22:00".to_owned(), "special"),
            ]
        );
        // A start the programme does not have: the special is kept but not planned.
        assert_eq!(
            primetime(&[special(LocalTime::from_hm(20, 0).unwrap())]),
            [
                ("2026-10-23 19:00".to_owned(), "primetime"),
                ("2026-10-23 22:00".to_owned(), "primetime"),
            ]
        );
    }

    #[test]
    fn daylight_saving_days_need_no_short_show() {
        let programme = Programme::default();
        // From Primetime's end on the day before a change to the next Primetime: 22 hours
        // around spring forward, 24 around fall back; both whole Extended Play shows.
        for (from, until, count) in [
            // 7 March is still EST: Primetime 01:00-02:00 UTC.
            ("2026-03-08T02:00:00Z", "2026-03-09T00:00:00Z", 44),
            ("2026-11-01T01:00:00Z", "2026-11-02T01:00:00Z", 48),
        ] {
            let shows = plan(&programme, &[], utc(from), utc(until));
            assert_back_to_back(&shows);
            assert_eq!(
                lengths(&shows),
                vec![(u64::from(whole_minutes(EP_EPISODE)), Mode::ExtendedPlay); count],
                "{from}"
            );
        }
    }

    #[test]
    fn every_day_of_a_year_has_one_primetime_show_at_eight_new_york_time() {
        let programme = Programme::default();
        let shows = plan(
            &programme,
            &[],
            utc("2026-01-01T05:00:00Z"),
            utc("2027-01-01T05:00:00Z"),
        );
        assert_back_to_back(&shows);
        let primetime: Vec<_> = shows
            .iter()
            .filter(|s| s.mode() == Mode::Primetime)
            .collect();
        assert_eq!(primetime.len(), 365);
        for show in primetime {
            let local_day = programme.zone.date_of(show.start);
            assert_eq!(
                programme.zone.at(local_day, programme.primetime_at.first()),
                Some(show.start)
            );
        }
        for show in &shows {
            assert!(
                show.minutes() == u64::from(whole_minutes(EP_EPISODE))
                    || show.mode() == Mode::Primetime
            );
        }
    }

    #[test]
    fn an_extended_play_special_is_one_episode_in_primetimes_place() {
        let programme = Programme::default();
        let special = SpecialEpisode {
            date: date("2026-10-31"),
            at: LocalTime::from_hm(20, 0).unwrap(),
            title: "Movie Night".into(),
            category: "otdb:11".into(),
            length: EP_EPISODE,
            mode: Mode::ExtendedPlay,
            artwork: Some(ThemeArtwork::new("Community").unwrap()),
        };
        special.validate().unwrap();
        let shows = day(&programme, std::slice::from_ref(&special), "2026-10-31");
        assert_back_to_back(&shows);
        let specials: Vec<_> = shows.iter().filter(|s| s.category().is_some()).collect();
        assert_eq!(specials.len(), 1);
        assert_eq!(specials[0].start, utc("2026-11-01T00:00:00Z"), "20:00 EDT");
        assert_eq!(specials[0].mode(), Mode::ExtendedPlay);
        assert!(shows.iter().all(|s| s.mode() == Mode::ExtendedPlay));

        let too_long = SpecialEpisode {
            length: EP_EPISODE + Duration::from_secs(60),
            ..special
        };
        assert_eq!(
            too_long.validate(),
            Err(ValidationError::SpecialLength {
                mode: Mode::ExtendedPlay,
                length: EP_EPISODE + Duration::from_secs(60)
            })
        );
        assert_eq!(
            ThemeArtwork::new("In The News"),
            Err(ValidationError::Artwork {
                name: "In The News".into()
            })
        );
    }

    #[test]
    fn a_special_takes_its_days_primetime_place() {
        let programme = Programme::default();
        let special = SpecialEpisode {
            date: date("2026-10-31"),
            at: LocalTime::from_hm(20, 0).unwrap(),
            title: "Halloween".into(),
            category: "otdb:11".into(),
            length: minutes(90),
            mode: crate::schedule::Mode::Primetime,
            artwork: None,
        };
        let shows = day(&programme, std::slice::from_ref(&special), "2026-10-31");
        let specials: Vec<_> = shows.iter().filter(|s| s.category().is_some()).collect();
        assert_eq!(specials.len(), 1);
        assert_eq!(specials[0].start, utc("2026-11-01T00:00:00Z"), "20:00 EDT");
        assert_eq!(specials[0].minutes(), 90);
        assert_eq!(specials[0].mode(), Mode::Primetime);
        assert_eq!(specials[0].category(), Some("otdb:11"));
        assert!(!shows.iter().any(|s| s.kind == ShowKind::Primetime));
        assert_back_to_back(&shows);
        // The days around it keep their Primetime show.
        let next = day(&programme, std::slice::from_ref(&special), "2026-11-02");
        assert!(next.iter().any(|s| s.kind == ShowKind::Primetime));
    }

    #[test]
    fn a_rest_under_the_minimum_stays_a_lobby_before_the_next_show() {
        let programme = Programme::default();
        // A 100-minute special leaves 22 h 20 min until the next Primetime show: 44 whole
        // Extended Play shows and 20 minutes, under the 30-minute minimum. Added to the last show
        // they would outlast its episode, so they stay a lobby before Primetime.
        let special = SpecialEpisode {
            date: date("2026-10-23"),
            at: LocalTime::from_hm(20, 0).unwrap(),
            title: "Long".into(),
            category: "otdb:9".into(),
            length: minutes(100),
            mode: crate::schedule::Mode::Primetime,
            artwork: None,
        };
        let shows = plan(
            &programme,
            &[special],
            utc("2026-10-24T00:00:00Z"),
            utc("2026-10-25T00:30:00Z"),
        );
        let ep: Vec<_> = shows
            .iter()
            .filter(|s| s.mode() == Mode::ExtendedPlay)
            .collect();
        assert_eq!(ep.len(), 44);
        assert!(
            ep.iter()
                .all(|s| s.minutes() == u64::from(whole_minutes(EP_EPISODE)))
        );
        assert_back_to_back(&shows[..shows.len() - 1]);
        let primetime = shows.last().unwrap();
        assert_eq!(primetime.mode(), Mode::Primetime);
        assert_eq!(
            primetime
                .start
                .saturating_duration_since(ep.last().unwrap().end),
            Duration::from_secs(20 * 60),
            "the lobby before Primetime"
        );
    }

    #[test]
    fn the_plan_covers_the_range_and_nothing_else() {
        let programme = Programme::default();
        let from = utc("2026-10-23T12:30:00Z");
        let until = utc("2026-10-23T13:30:00Z");
        let shows = plan(&programme, &[], from, until);
        assert!(!shows.is_empty());
        assert!(shows.first().unwrap().start <= from);
        assert!(shows.last().unwrap().end >= until);
        assert!(shows.iter().all(|s| s.end > from && s.start < until));
    }

    /// Ids `id-1`, `id-2`, ... in order.
    fn counter() -> impl FnMut() -> String {
        let mut next = 0;
        move || {
            next += 1;
            format!("id-{next}")
        }
    }

    fn assert_shows_back_to_back(shows: &[Show]) {
        for pair in shows.windows(2) {
            assert_eq!(pair[0].plan.end, pair[1].plan.start, "{pair:?}");
        }
    }

    #[test]
    fn the_horizon_is_filled_from_now_and_then_topped_up() {
        let programme = Programme::default();
        let now = utc("2026-10-23T12:40:00Z");
        let mut ids = counter();
        let mut shows = extend(&programme, &[], &[], now, &mut ids);
        assert_shows_back_to_back(&shows);
        assert!(
            shows[0].plan.start <= now && now < shows[0].plan.end,
            "the running show"
        );
        assert_eq!(
            shows[0].plan.start,
            utc("2026-10-23T12:30:00Z"),
            "with its planned start, not cut to now"
        );
        assert!(shows.last().unwrap().plan.end >= now + HORIZON);
        assert_eq!(shows[0].session_id, "id-1");
        assert_eq!(shows[0].episode_id, "id-2");
        assert_eq!(
            shows
                .iter()
                .filter(|s| s.plan.mode() == Mode::Primetime)
                .count(),
            2,
            "two days, two Primetime shows"
        );

        // Nothing to add while the horizon is covered; an hour later, one more show at most.
        assert!(extend(&programme, &[], &shows, now, &mut ids).is_empty());
        let later = now + Duration::from_secs(3 * 3600);
        let more = extend(&programme, &[], &shows, later, &mut ids);
        assert!(!more.is_empty());
        shows.extend(more);
        assert_shows_back_to_back(&shows);
        assert!(shows.last().unwrap().plan.end >= later + HORIZON);
        let mut session_ids: Vec<_> = shows.iter().map(|s| &s.session_id).collect();
        session_ids.dedup();
        assert_eq!(session_ids.len(), shows.len(), "every show its own id");
    }

    #[test]
    fn after_a_long_gap_shows_start_again_from_now() {
        let programme = Programme::default();
        let now = utc("2026-10-23T12:30:00Z");
        let shows = extend(&programme, &[], &[], now, &mut counter());
        let much_later = now + Duration::from_secs(10 * 86_400);
        let more = extend(&programme, &[], &shows, much_later, &mut counter());
        assert!(more[0].plan.start <= much_later && much_later < more[0].plan.end);
    }

    #[test]
    fn the_listed_shows_are_frozen_and_the_rest_can_be_planned_again() {
        let programme = Programme::default();
        let now = utc("2026-10-23T12:30:00Z");
        let shows = extend(&programme, &[], &[], now, &mut counter());
        // Running and next, as the Grid lists them.
        assert_eq!(frozen(&shows, now), 2);
        // A minute after the first show ended: it is history, and two more are listed.
        let later = shows[0].plan.end + Duration::from_secs(60);
        assert_eq!(frozen(&shows, later), 3);
        assert_eq!(frozen(&shows[..1], now), 1);
        assert_eq!(frozen(&[], now), 0);

        // Tomorrow becomes a special: the kept shows stay, the plan continues after them.
        let tomorrow = programme.zone.date_of(now).next_day().unwrap();
        let special = SpecialEpisode {
            date: tomorrow,
            at: LocalTime::from_hm(20, 0).unwrap(),
            title: "Quiz Night".into(),
            category: "otdb:9".into(),
            length: minutes(90),
            mode: crate::schedule::Mode::Primetime,
            artwork: None,
        };
        let kept = &shows[..frozen(&shows, now)];
        let replanned: Vec<_> = kept
            .iter()
            .cloned()
            .chain(extend(&programme, &[special], kept, now, &mut counter()))
            .collect();
        assert_eq!(replanned[..2], shows[..2]);
        assert_shows_back_to_back(&replanned);
        let specials: Vec<_> = replanned
            .iter()
            .filter(|s| s.plan.category() == Some("otdb:9"))
            .collect();
        assert_eq!(specials.len(), 1);
        assert_eq!(specials[0].plan.minutes(), 90);
    }

    #[test]
    fn a_kept_show_that_runs_into_the_plan_shortens_the_next_show() {
        // A hand-started 30-minute show at 19:50 New York time runs to 20:20: Primetime that day
        // keeps its end and starts when the hand-started show ends.
        let programme = Programme::default();
        let start = utc("2026-10-23T23:50:00Z");
        let manual = Show {
            session_id: "manual".into(),
            episode_id: "manual-episode".into(),
            plan: PlannedShow {
                start,
                end: start + Duration::from_secs(30 * 60),
                kind: ShowKind::ExtendedPlay,
            },
            manual: true,
        };
        let next = extend(
            &programme,
            &[],
            std::slice::from_ref(&manual),
            start,
            &mut counter(),
        );
        assert_eq!(next[0].plan.start, utc("2026-10-24T00:20:00Z"));
        assert_eq!(next[0].plan.end, utc("2026-10-24T01:00:00Z"));
        assert_eq!(next[0].plan.mode(), Mode::Primetime);
    }

    #[test]
    fn lengths_are_validated() {
        let mut programme = Programme::default();
        assert_eq!(programme.validate(), Ok(()));
        programme.extended_play = minutes(10);
        assert_eq!(
            programme.validate(),
            Err(ValidationError::Length {
                setting: "Extended Play",
                length: minutes(10)
            })
        );
        programme.extended_play = minutes(120);
        assert_eq!(
            programme.validate(),
            Err(ValidationError::ExtendedPlayLength {
                length: minutes(120)
            })
        );
        programme.extended_play = EP_EPISODE;
        programme.min_extended_play = minutes(40);
        assert_eq!(
            programme.validate(),
            Err(ValidationError::ShortestLength {
                shortest: minutes(40),
                whole: EP_EPISODE
            })
        );
        programme.min_extended_play = minutes(30);
        programme.primetime = MAX_SHOW + Duration::from_secs(60);
        assert_eq!(
            programme.validate(),
            Err(ValidationError::Length {
                setting: "Primetime",
                length: MAX_SHOW + Duration::from_secs(60)
            })
        );
    }
}
