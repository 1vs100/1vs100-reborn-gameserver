//! UTC instants and the text formats they travel in.

use std::{
    collections::BTreeSet,
    ops::{Add, Sub},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime},
};

use ::time::{
    Date, Month, OffsetDateTime, PrimitiveDateTime, format_description::BorrowedFormatItem,
    macros::format_description,
};

/// `YYYY-MM-DDTHH:MM:SS` with an optional fraction, in UTC (`Z`).
const TEXT: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second][optional [.[subsecond]]]Z");
const GRID: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
const XSOR: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");
const DATE: &[BorrowedFormatItem<'_>] = format_description!("[year]-[month]-[day]");
const TIME_OF_DAY: &[BorrowedFormatItem<'_>] = format_description!("[hour]:[minute]");
const RFC1123: &[BorrowedFormatItem<'_>] = format_description!(
    "[weekday repr:short], [day] [month repr:short] [year] [hour]:[minute]:[second] GMT"
);

/// The host's wall clock at this moment.
pub fn now() -> Utc {
    Utc::from(SystemTime::now())
}

/// A UTC instant with millisecond precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Utc {
    unix_ms: i64,
}

impl Utc {
    /// `YYYY-MM-DDTHH:MM:SS[.fff...]Z` with a valid calendar date; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let time = PrimitiveDateTime::parse(text, TEXT).ok()?.assume_utc();
        Some(Self::from_unix_ms(
            i64::try_from(time.unix_timestamp_nanos() / 1_000_000).ok()?,
        ))
    }

    /// Midnight UTC of a calendar date; `None` if the date does not exist.
    pub fn from_date(year: i32, month: u8, day: u8) -> Option<Self> {
        let date = Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()?;
        Some(Self::from_unix_secs(
            date.midnight().assume_utc().unix_timestamp(),
        ))
    }

    pub fn from_unix_secs(secs: i64) -> Self {
        Self::from_unix_ms(secs.saturating_mul(1_000))
    }

    pub fn from_unix_ms(unix_ms: i64) -> Self {
        Self { unix_ms }
    }

    /// Whole seconds, rounded down.
    pub fn unix_secs(self) -> i64 {
        self.unix_ms.div_euclid(1_000)
    }

    /// Whole milliseconds.
    pub fn unix_ms(self) -> i64 {
        self.unix_ms
    }

    pub fn as_secs_f64(self) -> f64 {
        self.unix_ms as f64 / 1_000.0
    }

    /// Whole days since 1970-01-01, rounded down.
    pub fn unix_days(self) -> i64 {
        self.unix_secs().div_euclid(86_400)
    }

    /// How long after `earlier` this is; zero if it is not later.
    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_millis(u64::try_from(self.unix_ms - earlier.unix_ms).unwrap_or(0))
    }

    /// The same instant with the milliseconds dropped.
    pub fn trunc_secs(self) -> Self {
        Self::from_unix_secs(self.unix_secs())
    }

    /// Calendar year and month (1-12).
    pub fn year_month(self) -> (i32, u8) {
        let date = self.date_time().date();
        (date.year(), u8::from(date.month()))
    }

    /// `2026-10-11T01:33:52Z`: the Grid's format, whole seconds.
    pub fn to_grid(self) -> String {
        self.format(GRID)
    }

    /// `2026-10-11T01:33:52.000Z`: the title's own format, as in XSOR documents.
    pub fn to_xsor(self) -> String {
        self.format(XSOR)
    }

    /// `Sun, 11 Oct 2026 01:33:52 GMT`.
    pub fn to_rfc1123(self) -> String {
        self.format(RFC1123)
    }

    fn date_time(self) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(self.unix_ms) * 1_000_000)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }

    fn format(self, format: &[BorrowedFormatItem<'_>]) -> String {
        self.date_time()
            .format(format)
            .expect("every Utc formats in the fixed formats")
    }
}

impl From<Utc> for SystemTime {
    /// Times before 1970 become the epoch.
    fn from(time: Utc) -> Self {
        let ms = u64::try_from(time.unix_ms).unwrap_or(0);
        SystemTime::UNIX_EPOCH + Duration::from_millis(ms)
    }
}

impl From<SystemTime> for Utc {
    /// Times before 1970 become 1970-01-01T00:00:00Z.
    fn from(time: SystemTime) -> Self {
        let ms = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis();
        Self::from_unix_ms(i64::try_from(ms).unwrap_or(i64::MAX))
    }
}

impl Add<Duration> for Utc {
    type Output = Self;

    fn add(self, duration: Duration) -> Self {
        let ms = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
        Self::from_unix_ms(self.unix_ms.saturating_add(ms))
    }
}

impl Sub<Duration> for Utc {
    type Output = Self;

    fn sub(self, duration: Duration) -> Self {
        let ms = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
        Self::from_unix_ms(self.unix_ms.saturating_sub(ms))
    }
}

/// Serde for an instant as whole unix seconds, the shape the admin JSON has.
pub mod unix_secs {
    use super::Utc;
    use serde::{Serialize, Serializer};

    pub fn serialize<S: Serializer>(time: &Utc, serializer: S) -> Result<S::Ok, S::Error> {
        time.unix_secs().serialize(serializer)
    }
}

/// Time source; replays and tests run on their own time.
pub trait Clock: Send + Sync {
    fn now(&self) -> Utc;

    /// Block the calling thread until `duration` has passed on this clock.
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Calendar date without a zone: `2026-10-23`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate(Date);

impl LocalDate {
    /// `YYYY-MM-DD` with a valid calendar date.
    pub fn parse(text: &str) -> Option<Self> {
        Date::parse(text, DATE).ok().map(Self)
    }

    pub fn from_ymd(year: i32, month: u8, day: u8) -> Option<Self> {
        let month = Month::try_from(month).ok()?;
        Date::from_calendar_date(year, month, day).ok().map(Self)
    }

    pub fn next_day(self) -> Option<Self> {
        self.0.next_day().map(Self)
    }

    pub fn previous_day(self) -> Option<Self> {
        self.0.previous_day().map(Self)
    }
}

impl std::fmt::Display for LocalDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.format(DATE).map_err(|_| std::fmt::Error)?)
    }
}

/// A wall-clock time of day, to the minute: `20:00`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalTime(::time::Time);

impl LocalTime {
    /// `HH:MM`, 24-hour.
    pub fn parse(text: &str) -> Option<Self> {
        ::time::Time::parse(text, TIME_OF_DAY).ok().map(Self)
    }

    pub fn from_hm(hour: u8, minute: u8) -> Option<Self> {
        ::time::Time::from_hms(hour, minute, 0).ok().map(Self)
    }

    pub const MIDNIGHT: Self = Self(::time::Time::MIDNIGHT);

    pub fn since_midnight(self) -> std::time::Duration {
        let (hour, minute, _) = self.0.as_hms();
        std::time::Duration::from_secs(u64::from(hour) * 3600 + u64::from(minute) * 60)
    }
}

impl std::fmt::Display for LocalTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.format(TIME_OF_DAY).map_err(|_| std::fmt::Error)?)
    }
}

/// IANA time zone from the compiled-in database.
#[derive(Clone, Debug)]
pub struct Zone {
    name: String,
    zone: jiff::tz::TimeZone,
}

impl Zone {
    /// The zone called `name`; `None` for a name the database does not know.
    pub fn get(name: &str) -> Option<Self> {
        let zone = jiff::tz::TimeZone::get(name).ok()?;
        Some(Self {
            name: name.to_owned(),
            zone,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// `2026-10-23 20:00`: the wall clock in this zone at instant `at`, to the minute.
    pub fn local_text(&self, at: Utc) -> String {
        jiff::Timestamp::from_second(at.unix_secs())
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
            .to_zoned(self.zone.clone())
            .strftime("%Y-%m-%d %H:%M")
            .to_string()
    }

    /// The date in this zone at instant `at`.
    pub fn date_of(&self, at: Utc) -> LocalDate {
        let date = jiff::Timestamp::from_second(at.unix_secs())
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
            .to_zoned(self.zone.clone())
            .date();
        LocalDate::from_ymd(i32::from(date.year()), date.month() as u8, date.day() as u8)
            .expect("a date of the zone database is a calendar date")
    }

    /// Instant the wall clock shows `time` on `date`; skipped times move on by the gap.
    pub fn at(&self, date: LocalDate, time: LocalTime) -> Option<Utc> {
        let civil = jiff::civil::DateTime::new(
            i16::try_from(date.0.year()).ok()?,
            u8::from(date.0.month()) as i8,
            date.0.day() as i8,
            time.0.hour() as i8,
            time.0.minute() as i8,
            0,
            0,
        )
        .ok()?;
        let instant = self.zone.to_ambiguous_timestamp(civil).compatible().ok()?;
        Some(Utc::from_unix_secs(instant.as_second()))
    }
}

impl PartialEq for Zone {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Zone {}

/// Clock for one thread's waits: a wait for a deadline returns at the deadline.
pub struct ManualClock(Mutex<Utc>);

impl ManualClock {
    pub fn starting_at(now: Utc) -> Self {
        Self(Mutex::new(now))
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Utc {
        *crate::sync::lock(&self.0)
    }

    fn sleep(&self, duration: Duration) {
        let mut now = crate::sync::lock(&self.0);
        *now = *now + duration;
    }
}

/// The host's wall clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Utc {
        Utc::from(SystemTime::now())
    }
}

/// Clock for replays: moves only on `advance_to`; sleep on it only under a `Busy` guard.
pub struct VirtualClock {
    state: Mutex<VirtualState>,
    changed: Condvar,
}

struct VirtualState {
    now: Utc,
    busy: usize,
    /// Parked sleepers by wake time; a woken sleeper is removed and counted busy again.
    sleepers: BTreeSet<(Utc, u64)>,
    next_sleeper: u64,
}

impl VirtualClock {
    pub fn starting_at(now: Utc) -> Self {
        Self {
            state: Mutex::new(VirtualState {
                now,
                busy: 0,
                sleepers: BTreeSet::new(),
                next_sleeper: 0,
            }),
            changed: Condvar::new(),
        }
    }

    /// Work in progress; `advance_to` waits for it.
    pub fn busy(self: &Arc<Self>) -> Busy {
        self.lock().busy += 1;
        Busy(Arc::clone(self))
    }

    /// Move to `time`, waking sleepers due on the way once prior work settles.
    pub fn advance_to(&self, time: Utc) {
        let mut state = self.settled();
        loop {
            match state.sleepers.first() {
                Some(&(wake, _)) if wake <= time => {
                    state.now = state.now.max(wake);
                    while let Some(&(wake, _)) = state.sleepers.first()
                        && wake <= state.now
                    {
                        state.sleepers.pop_first();
                        state.busy += 1;
                    }
                    self.changed.notify_all();
                    state = self.wait_settled(state);
                }
                _ => {
                    state.now = state.now.max(time);
                    return;
                }
            }
        }
    }

    /// First sleeper wake time once settled; `None` if nothing sleeps.
    pub fn next_wake(&self) -> Option<Utc> {
        self.settled().sleepers.first().map(|&(wake, _)| wake)
    }

    fn settled(&self) -> MutexGuard<'_, VirtualState> {
        self.wait_settled(self.lock())
    }

    fn wait_settled<'a>(
        &self,
        state: MutexGuard<'a, VirtualState>,
    ) -> MutexGuard<'a, VirtualState> {
        self.changed
            .wait_while(state, |state| state.busy > 0)
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock(&self) -> MutexGuard<'_, VirtualState> {
        crate::sync::lock(&self.state)
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> Utc {
        self.lock().now
    }

    fn sleep(&self, duration: Duration) {
        let mut state = self.lock();
        let sleeper = (state.now + duration, state.next_sleeper);
        state.next_sleeper += 1;
        state.sleepers.insert(sleeper);
        state.busy = state
            .busy
            .checked_sub(1)
            .expect("VirtualClock::sleep outside a Busy section");
        self.changed.notify_all();
        let _woken = self
            .changed
            .wait_while(state, |state| state.sleepers.contains(&sleeper))
            .unwrap_or_else(PoisonError::into_inner);
    }
}

/// Work in progress on a [`VirtualClock`]; see [`VirtualClock::busy`].
pub struct Busy(Arc<VirtualClock>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.lock().busy -= 1; // `VirtualClock::lock`
        self.0.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_york() -> Zone {
        Zone::get("America/New_York").unwrap()
    }

    fn date(text: &str) -> LocalDate {
        LocalDate::parse(text).unwrap()
    }

    #[test]
    fn eight_pm_in_new_york_follows_daylight_saving() {
        let zone = new_york();
        let eight = LocalTime::parse("20:00").unwrap();
        let at = |day| zone.at(date(day), eight).unwrap().to_grid();
        // 2026: EDT from 8 March, EST again from 1 November (both at 02:00 local).
        assert_eq!(at("2026-03-07"), "2026-03-08T01:00:00Z");
        assert_eq!(at("2026-03-08"), "2026-03-09T00:00:00Z");
        assert_eq!(at("2026-10-31"), "2026-11-01T00:00:00Z");
        assert_eq!(at("2026-11-01"), "2026-11-02T01:00:00Z");
    }

    #[test]
    fn the_changeover_days_are_23_and_25_hours_long() {
        let zone = new_york();
        let length = |day: &str| {
            let start = zone.at(date(day), LocalTime::MIDNIGHT).unwrap();
            let end = zone
                .at(date(day).next_day().unwrap(), LocalTime::MIDNIGHT)
                .unwrap();
            end.saturating_duration_since(start).as_secs() / 3600
        };
        assert_eq!(length("2026-03-08"), 23);
        assert_eq!(length("2026-11-01"), 25);
        assert_eq!(length("2026-10-23"), 24);
    }

    #[test]
    fn skipped_and_repeated_wall_times_have_one_answer() {
        let zone = new_york();
        let at = |day, time| {
            zone.at(date(day), LocalTime::parse(time).unwrap())
                .unwrap()
                .to_grid()
        };
        // 02:30 does not exist on 8 March: it is taken as 03:30 EDT.
        assert_eq!(at("2026-03-08", "02:30"), "2026-03-08T07:30:00Z");
        // 01:30 happens twice on 1 November: the first, still EDT.
        assert_eq!(at("2026-11-01", "01:30"), "2026-11-01T05:30:00Z");
    }

    #[test]
    fn a_zones_date_of_an_instant_is_its_local_day() {
        let zone = new_york();
        let at = |text| zone.date_of(Utc::parse(text).unwrap()).to_string();
        assert_eq!(at("2026-11-02T00:30:00Z"), "2026-11-01", "19:30 EST");
        assert_eq!(at("2026-11-02T05:00:00Z"), "2026-11-02", "00:00 EST");
        assert_eq!(
            zone.local_text(Utc::parse("2026-11-02T01:00:00Z").unwrap()),
            "2026-11-01 20:00"
        );
    }

    #[test]
    fn the_compiled_in_database_is_current() {
        // Mexico City dropped daylight saving at the end of 2022; a database from before
        // (as `time-tz` 2.0 bundles, 2022a) would answer 17:00 for a July noon.
        let zone = Zone::get("America/Mexico_City").unwrap();
        let noon = zone
            .at(date("2026-07-01"), LocalTime::parse("12:00").unwrap())
            .unwrap();
        assert_eq!(noon.to_grid(), "2026-07-01T18:00:00Z");
    }

    #[test]
    fn local_dates_times_and_zone_names_parse_strictly() {
        assert_eq!(date("2026-10-23").to_string(), "2026-10-23");
        assert_eq!(
            date("2026-12-31").next_day().unwrap().to_string(),
            "2027-01-01"
        );
        assert_eq!(LocalDate::parse("2026-02-30"), None);
        assert_eq!(LocalDate::parse("2026-10-23T00:00"), None);
        assert_eq!(LocalTime::parse("20:00").unwrap().to_string(), "20:00");
        assert_eq!(LocalTime::parse("24:00"), None);
        assert_eq!(LocalTime::parse("8pm"), None);
        assert!(Zone::get("America/Nowhere").is_none());
        assert_eq!(new_york().name(), "America/New_York");
    }

    #[test]
    fn a_manual_clock_moves_on_by_each_sleep_at_once() {
        let start = Utc::parse("2026-10-11T01:33:52Z").unwrap();
        let clock = ManualClock::starting_at(start);
        let begun = std::time::Instant::now();
        clock.sleep(Duration::from_secs(5));
        clock.sleep(Duration::from_millis(250));
        assert_eq!(clock.now(), start + Duration::from_millis(5_250));
        assert!(begun.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn parses_the_title_and_grid_formats() {
        let whole = Utc::parse("2026-10-11T01:33:52Z").unwrap();
        assert_eq!(whole.unix_secs(), 1_791_682_432);
        assert_eq!(Utc::parse("2026-10-11T01:33:52.000Z"), Some(whole));
        assert_eq!(
            Utc::parse("2026-10-11T01:33:52.337Z"),
            Some(Utc::from_unix_ms(1_791_682_432_337))
        );
        assert_eq!(
            Utc::parse("2026-10-11T01:33:52.5Z"),
            Some(Utc::from_unix_ms(1_791_682_432_500))
        );
    }

    #[test]
    fn rejects_anything_but_a_utc_time_on_a_real_date() {
        for bad in [
            "",
            "2026-10-11",
            "2026-10-11T01:33:52",
            "2026-10-11T01:33:52+00:00",
            "2026-10-11 01:33:52Z",
            "2026-02-30T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-10-11T24:00:00Z",
            "2026-10-11T01:60:00Z",
            "2026-10-11T01:33:52.Z",
            " 2026-10-11T01:33:52Z",
        ] {
            assert_eq!(Utc::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn prints_each_format() {
        let time = Utc::from_unix_ms(1_791_682_432_337);
        assert_eq!(time.to_grid(), "2026-10-11T01:33:52Z");
        assert_eq!(time.to_xsor(), "2026-10-11T01:33:52.337Z");
        assert_eq!(time.trunc_secs().to_xsor(), "2026-10-11T01:33:52.000Z");
        assert_eq!(time.to_rfc1123(), "Sun, 11 Oct 2026 01:33:52 GMT");
        assert_eq!(
            Utc::from_unix_secs(0).to_rfc1123(),
            "Thu, 01 Jan 1970 00:00:00 GMT"
        );
    }

    #[test]
    fn arithmetic_rolls_over_dates() {
        let time = Utc::parse("2026-12-31T23:59:58Z").unwrap() + Duration::from_secs(5);
        assert_eq!(time.to_grid(), "2027-01-01T00:00:03Z");
        assert_eq!(
            (time - Duration::from_secs(5)).to_grid(),
            "2026-12-31T23:59:58Z"
        );
        assert_eq!(
            Utc::parse("2024-02-29T00:00:00Z").unwrap().year_month(),
            (2024, 2)
        );
    }

    #[test]
    fn calendar_dates_and_days() {
        let day = Utc::from_date(2026, 9, 27).unwrap();
        assert_eq!(day.to_grid(), "2026-09-27T00:00:00Z");
        assert_eq!(day.unix_days(), 20_723);
        assert_eq!((day + Duration::from_secs(86_399)).unix_days(), 20_723);
        assert_eq!(Utc::from_date(2026, 2, 29), None);
        assert_eq!(Utc::from_date(2026, 0, 1), None);
    }

    #[test]
    fn converts_from_system_time() {
        let time = SystemTime::UNIX_EPOCH + Duration::from_millis(1_791_682_432_337);
        assert_eq!(Utc::from(time), Utc::from_unix_ms(1_791_682_432_337));
        let before_epoch = SystemTime::UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(Utc::from(before_epoch), Utc::from_unix_secs(0));
    }

    #[test]
    fn a_virtual_clock_wakes_sleepers_in_order_and_only_when_settled() {
        let start = Utc::parse("2026-10-21T19:43:18.423Z").unwrap();
        let clock = Arc::new(VirtualClock::starting_at(start));
        let (woke, order) = std::sync::mpsc::channel();
        for (name, secs) in [("late", 5), ("early", 1), ("never", 60)] {
            let busy = clock.busy();
            let clock = Arc::clone(&clock);
            let woke = woke.clone();
            std::thread::spawn(move || {
                let _busy = busy;
                clock.sleep(Duration::from_secs(secs));
                woke.send((name, clock.now())).unwrap();
            });
        }
        drop(woke);

        clock.advance_to(start + Duration::from_secs(10));
        assert_eq!(clock.now(), start + Duration::from_secs(10));
        // Each sleeper ran (and sent) before the clock moved past it.
        assert_eq!(
            order.try_iter().collect::<Vec<_>>(),
            [
                ("early", start + Duration::from_secs(1)),
                ("late", start + Duration::from_secs(5)),
            ]
        );
        assert_eq!(clock.next_wake(), Some(start + Duration::from_secs(60)));
        clock.advance_to(start + Duration::from_secs(60));
        assert_eq!(order.recv().unwrap().0, "never");
        assert_eq!(clock.next_wake(), None);
    }
}
