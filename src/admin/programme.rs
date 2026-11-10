//! The daily programme, its shows and specials, and what the Grid lists.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{ErrorResponse, Query, Reply, mode, non_empty, query_as};
use crate::{
    app::App,
    programme::{
        self, PlannedShow, PrimetimeSlots, Programme, ShowKind, SpecialEpisode, SpecialSlot,
        ThemeArtwork, ValidationError,
    },
    schedule::{Mode, ScheduleError},
    sync::lock,
    time::{LocalDate, LocalTime, Zone},
    timeline,
};

/// The daily programme: Primetime at local times, Extended Play between them.
#[derive(Serialize, ToSchema)]
pub struct ProgrammeInfo {
    /// IANA time zone, e.g. `America/New_York`.
    zone: String,
    /// Local starts of the daily Primetime shows, `HH:MM`, in order.
    primetime_at: Vec<String>,
    primetime_minutes: u32,
    extended_play_minutes: u32,
    /// The shortest Extended Play show: what whole ones leave before a Primetime show is one
    /// more show, and must be at least this long.
    min_extended_play_minutes: u32,
}

/// One show as the programme plans it.
#[derive(Serialize, ToSchema)]
pub struct PlannedShowInfo {
    start_utc: String,
    end_utc: String,
    /// The start on the programme zone's wall clock, `YYYY-MM-DD HH:MM`.
    start_local: String,
    minutes: u64,
    /// `pt` (Primetime) or `ep` (Extended Play).
    mode: &'static str,
    /// `primetime`, `special` or `extended_play`.
    kind: &'static str,
    /// A special's title.
    title: Option<String>,
    /// A special's question category.
    category: Option<String>,
    /// A special's theme artwork.
    artwork: Option<String>,
}

/// A materialised show: what consoles are or will be served.
#[derive(Serialize, ToSchema)]
pub struct ShowInfo {
    session_id: String,
    episode_id: String,
    /// `running`, `listed` (the Grid lists it: a programme change keeps it), `planned` (a change
    /// plans it again) or `past`.
    state: &'static str,
    /// Started by hand (`POST /admin/shows/now`).
    manual: bool,
    #[serde(flatten)]
    plan: PlannedShowInfo,
}

/// A themed episode in the place of one Primetime show.
#[derive(Serialize, ToSchema)]
pub struct SpecialInfo {
    /// `YYYY-MM-DD` in the programme's zone.
    date: String,
    /// `HH:MM`, the Primetime start it replaces.
    at: String,
    title: String,
    category: String,
    minutes: u32,
    /// `pt` (Primetime) or `ep` (Extended Play).
    mode: &'static str,
    /// Theme artwork of the episode package, e.g. `Community`.
    artwork: Option<String>,
}

/// The programme's values; those left out keep theirs.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProgrammeParams {
    /// IANA time zone, e.g. America/New_York.
    zone: Option<String>,
    /// Local starts of Primetime, HH:MM separated by commas (19:00,22:00).
    primetime_at: Option<String>,
    /// 15-240.
    primetime_minutes: Option<u32>,
    /// 15-240.
    extended_play_minutes: Option<u32>,
    /// 15 up to the Extended Play length.
    min_extended_play_minutes: Option<u32>,
}

impl ProgrammeParams {
    fn is_empty(&self) -> bool {
        self.zone.is_none()
            && self.primetime_at.is_none()
            && self.primetime_minutes.is_none()
            && self.extended_play_minutes.is_none()
            && self.min_extended_play_minutes.is_none()
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PlanParams {
    /// How far ahead, 1-14 days; default 2.
    days: Option<u64>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ShowsNowParams {
    /// pt (default) or ep.
    mode: Option<String>,
    /// 1-240, default 30.
    minutes: Option<u32>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SpecialParams {
    /// The show's theme. Consoles show it on the lobby marquee and the Extended Play screens
    /// only in Extended Play; Primetime keeps its fixed marquee text.
    title: String,
    /// Question category of the served bank, e.g. music; it must have as many questions at each
    /// difficulty as one Primetime round asks (2, 3, 5, 4, 7).
    category: String,
    /// pt (default) or ep.
    mode: Option<String>,
    /// 15-240 for pt, 15-30 for ep; default the programme's length of that mode, so required
    /// while no programme is set.
    minutes: Option<u32>,
    /// Theme artwork of the episode package, e.g. Community, InTheNews or BingGeneral.
    artwork: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct GridShowInfo {
    session_id: String,
    start_utc: String,
    end_utc: String,
    /// `pt` (Primetime) or `ep` (Extended Play).
    mode: &'static str,
}

fn programme_info(programme: &Programme) -> ProgrammeInfo {
    ProgrammeInfo {
        zone: programme.zone.name().to_owned(),
        primetime_at: programme
            .primetime_at
            .times()
            .iter()
            .map(ToString::to_string)
            .collect(),
        primetime_minutes: programme::whole_minutes(programme.primetime),
        extended_play_minutes: programme::whole_minutes(programme.extended_play),
        min_extended_play_minutes: programme::whole_minutes(programme.min_extended_play),
    }
}

fn special_info(special: &SpecialEpisode) -> SpecialInfo {
    SpecialInfo {
        date: special.date.to_string(),
        at: special.at.to_string(),
        title: special.title.clone(),
        category: special.category.clone(),
        minutes: programme::whole_minutes(special.length),
        mode: special.mode.as_str(),
        artwork: special.artwork.as_ref().map(|art| art.as_str().to_owned()),
    }
}

fn planned_info(zone: &Zone, show: &PlannedShow) -> PlannedShowInfo {
    PlannedShowInfo {
        start_utc: show.start.to_grid(),
        end_utc: show.end.to_grid(),
        start_local: zone.local_text(show.start),
        minutes: show.minutes(),
        mode: show.mode().as_str(),
        kind: show.kind.as_str(),
        category: show.category().map(str::to_owned),
        title: match &show.kind {
            ShowKind::Special { title, .. } => Some(title.clone()),
            _ => None,
        },
        artwork: match &show.kind {
            ShowKind::Special {
                artwork: Some(art), ..
            } => Some(art.as_str().to_owned()),
            _ => None,
        },
    }
}

fn no_programme() -> Reply {
    Reply::error(404, "no programme: the unscheduled show is served")
}

/// A schedule change that failed: bad input is 400, storage is 500.
fn schedule_error(error: ScheduleError) -> Reply {
    let code = match &error {
        ScheduleError::Invalid { .. } => 400,
        ScheduleError::Storage { .. } => 500,
    };
    Reply::error(code, error.to_string())
}

/// The daily programme in effect
#[utoipa::path(get, path = "/admin/programme", responses((status = 200, description = "The programme", body = ProgrammeInfo), (status = 404, description = "No programme", body = ErrorResponse)))]
pub(super) fn programme_get(state: &App) -> Reply {
    match lock(&state.schedule).programme() {
        Some(programme) => Reply::json(&programme_info(programme)),
        None => no_programme(),
    }
}

/// Change the daily programme; parameters left out keep their value. The shows the Grid lists
/// stay, the rest is planned again
#[utoipa::path(post, path = "/admin/programme", params(ProgrammeParams), responses((status = 200, description = "The programme after the change", body = ProgrammeInfo), (status = 400, description = "Unknown zone, bad time or length", body = ErrorResponse)))]
pub(super) fn programme_set(state: &App, query: &Query) -> Reply {
    let params = match query_as::<ProgrammeParams>(query) {
        Ok(params) => params,
        Err(reply) => return reply,
    };
    let mut schedule = lock(&state.schedule);
    let programme =
        match with_programme_params(schedule.programme().cloned().unwrap_or_default(), &params) {
            Ok(programme) => programme,
            Err(reply) => return reply,
        };
    if let Err(error) = schedule.set_programme(programme.clone(), crate::time::now()) {
        return schedule_error(error);
    }
    timeline::record(
        "programme",
        format!(
            "Primetime {} {} for {} min, Extended Play {} min",
            programme.primetime_at,
            programme.zone.name(),
            programme::whole_minutes(programme.primetime),
            programme::whole_minutes(programme.extended_play)
        ),
        None,
    );
    Reply::json(&programme_info(&programme))
}

/// The programme parameters of `POST /admin/programme` (and of a plan preview) applied to
/// `programme`; those left out keep its values.
fn with_programme_params(
    mut programme: Programme,
    params: &ProgrammeParams,
) -> Result<Programme, Reply> {
    if let Some(zone) = &params.zone {
        match Zone::get(zone) {
            Some(zone) => programme.zone = zone,
            None => return Err(Reply::error(400, format!("unknown time zone {zone:?}"))),
        }
    }
    if let Some(at) = &params.primetime_at {
        match PrimetimeSlots::parse(at) {
            Ok(at) => programme.primetime_at = at,
            Err(error) => return Err(Reply::error(400, format!("primetime_at: {error}"))),
        }
    }
    for (value, field) in [
        (params.primetime_minutes, &mut programme.primetime),
        (params.extended_play_minutes, &mut programme.extended_play),
        (
            params.min_extended_play_minutes,
            &mut programme.min_extended_play,
        ),
    ] {
        if let Some(value) = value {
            *field = programme::minutes(value);
        }
    }
    Ok(programme)
}

/// The shows the programme plans from now, beyond the materialised ones too. With programme
/// parameters, the shows a programme changed that way would plan (a preview: nothing is saved)
#[utoipa::path(get, path = "/admin/programme/plan", params(PlanParams, ProgrammeParams), responses((status = 200, description = "Planned shows, the running one first", body = Vec<PlannedShowInfo>), (status = 400, description = "Bad days or preview parameters", body = ErrorResponse), (status = 404, description = "No programme, and no preview parameters", body = ErrorResponse)))]
pub(super) fn programme_plan(state: &App, query: &Query) -> Reply {
    let (days, params) = match (
        query_as::<PlanParams>(query),
        query_as::<ProgrammeParams>(query),
    ) {
        (Ok(plan), Ok(params)) => (plan.days.unwrap_or(2), params),
        (Err(reply), _) | (_, Err(reply)) => return reply,
    };
    if !(1..=14).contains(&days) {
        return Reply::error(400, "days must be 1-14");
    }
    let schedule = lock(&state.schedule);
    let preview = !params.is_empty();
    let programme = match (schedule.programme(), preview) {
        (Some(saved), false) => saved.clone(),
        (None, false) => return no_programme(),
        (saved, true) => match with_programme_params(saved.cloned().unwrap_or_default(), &params) {
            Ok(programme) => programme,
            Err(reply) => return reply,
        },
    };
    // The saved programme is shown as it plays; only a preview is held to the checks of a change.
    let checked = if preview {
        programme.validate_new()
    } else {
        programme.validate()
    };
    if let Err(error) = checked {
        return Reply::error(400, error.to_string());
    }
    let programme = &programme;
    let now = crate::time::now();
    let until = now + Duration::from_secs(days * 86_400);
    let shows: Vec<_> = crate::programme::plan(programme, schedule.specials(), now, until)
        .iter()
        .map(|show| planned_info(&programme.zone, show))
        .collect();
    Reply::json(&shows)
}

/// The materialised shows: what consoles are, or will be, served
#[utoipa::path(get, path = "/admin/shows", responses((status = 200, description = "Shows by start, with their ids and state", body = Vec<ShowInfo>), (status = 404, description = "No programme", body = ErrorResponse)))]
pub(super) fn shows_get(state: &App) -> Reply {
    let mut schedule = lock(&state.schedule);
    let Some(zone) = schedule.programme().map(|programme| programme.zone.clone()) else {
        return no_programme();
    };
    let now = crate::time::now();
    let shows = schedule.shows(now);
    let listed = shows.iter().position(|show| show.plan.end > now);
    let infos: Vec<_> = shows
        .iter()
        .enumerate()
        .map(|(index, show)| ShowInfo {
            session_id: show.session_id.clone(),
            episode_id: show.episode_id.clone(),
            state: match listed {
                _ if show.plan.end <= now => "past",
                _ if show.plan.start <= now => "running",
                Some(listed) if index < listed + 2 => "listed",
                _ => "planned",
            },
            manual: show.manual,
            plan: planned_info(&zone, &show.plan),
        })
        .collect();
    Reply::json(&infos)
}

/// LAN tests: the running show ends now and a show of `mode` runs from now; the programme
/// continues after it
#[utoipa::path(post, path = "/admin/shows/now", params(ShowsNowParams), responses((status = 200, description = "The shows after the change", body = Vec<ShowInfo>), (status = 400, description = "Bad mode or length, or no programme", body = ErrorResponse)))]
pub(super) fn shows_now(state: &App, query: &Query) -> Reply {
    let params = match query_as::<ShowsNowParams>(query) {
        Ok(params) => params,
        Err(reply) => return reply,
    };
    let mode = match mode(params.mode.as_deref(), "mode") {
        Ok(mode) => mode.unwrap_or_default(),
        Err(reply) => return reply,
    };
    let minutes = params.minutes.unwrap_or(30);
    let started =
        lock(&state.schedule).start_show_now(crate::time::now(), mode, programme::minutes(minutes));
    match started {
        Ok(show) => {
            timeline::record(
                "programme",
                format!("{} show started by hand for {minutes} min", mode.as_str()),
                Some(&show.session_id),
            );
            shows_get(state)
        }
        Err(error) => schedule_error(error),
    }
}

/// The special episodes
#[utoipa::path(get, path = "/admin/specials", responses((status = 200, description = "Special episodes by date and time", body = Vec<SpecialInfo>)))]
pub(super) fn specials_get(state: &App) -> Reply {
    let schedule = lock(&state.schedule);
    Reply::json(
        &schedule
            .specials()
            .iter()
            .map(special_info)
            .collect::<Vec<_>>(),
    )
}

/// Make the Primetime show at `at` on `date` a special episode (replaces one there); planned
/// again like a programme change
#[utoipa::path(post, path = "/admin/specials/{date}/{at}", params(("date" = String, Path, description = "YYYY-MM-DD in the programme's zone"), ("at" = String, Path, description = "HH:MM, one of the programme's Primetime starts"), SpecialParams), responses((status = 200, description = "Special episodes after the change", body = Vec<SpecialInfo>), (status = 400, description = "Bad date, time, title or length, no Primetime show at that time, a special that runs into the next one, a category with too few questions, or no length and no programme", body = ErrorResponse)))]
pub(super) fn special_put(state: &App, date: &str, at: &str, query: &Query) -> Reply {
    let slot = match special_slot(date, at) {
        Ok(slot) => slot,
        Err(reply) => return reply,
    };
    let params = match query_as::<SpecialParams>(query) {
        Ok(params) => params,
        Err(reply) => return reply,
    };
    let (title, category) = match (
        non_empty(params.title, "title"),
        non_empty(params.category, "category"),
    ) {
        (Ok(title), Ok(category)) => (title, category),
        (Err(reply), _) | (_, Err(reply)) => return reply,
    };
    let mode = match mode(params.mode.as_deref(), "mode") {
        Ok(mode) => mode.unwrap_or_default(),
        Err(reply) => return reply,
    };
    let artwork = match params.artwork.map(ThemeArtwork::new).transpose() {
        Ok(artwork) => artwork,
        Err(error) => return Reply::error(400, error.to_string()),
    };
    let bank = lock(&state.store).questions();
    if let Some(shortfall) = bank.shortfall(&category, mode) {
        return Reply::error(400, format!("category {category:?} has {shortfall}"));
    }
    let mut schedule = lock(&state.schedule);
    let length = match params.minutes {
        Some(minutes) => programme::minutes(minutes),
        // A special without a length is as long as the show it replaces, which needs a programme.
        None => {
            let Some(programme) = schedule.programme() else {
                return schedule_error(ValidationError::NoProgramme.into());
            };
            match mode {
                Mode::Primetime => programme.primetime,
                Mode::ExtendedPlay => programme.extended_play,
            }
        }
    };
    let special = SpecialEpisode {
        date: slot.date,
        at: slot.at,
        title,
        category,
        length,
        mode,
        artwork,
    };
    let record = format!(
        "special \"{}\" on {} at {} ({}, {})",
        special.title,
        slot.date,
        slot.at,
        special.mode.as_str(),
        special.category
    );
    if let Err(error) = schedule.put_special(special, crate::time::now()) {
        return schedule_error(error);
    }
    drop(schedule);
    timeline::record("programme", record, None);
    specials_get(state)
}

/// Give the Primetime show at `at` on `date` back its regular episode
#[utoipa::path(post, path = "/admin/specials/{date}/{at}/delete", params(("date" = String, Path, description = "YYYY-MM-DD"), ("at" = String, Path, description = "HH:MM")), responses((status = 200, description = "Special episodes after the change", body = Vec<SpecialInfo>), (status = 400, description = "Bad date or time", body = ErrorResponse), (status = 404, description = "No special at that date and time", body = ErrorResponse)))]
pub(super) fn special_delete(state: &App, date: &str, at: &str) -> Reply {
    let slot = match special_slot(date, at) {
        Ok(slot) => slot,
        Err(reply) => return reply,
    };
    let deleted = lock(&state.schedule).delete_special(slot, crate::time::now());
    match deleted {
        Ok(true) => {
            timeline::record(
                "programme",
                format!("special on {} at {} removed", slot.date, slot.at),
                None,
            );
            specials_get(state)
        }
        Ok(false) => Reply::status(404),
        Err(error) => schedule_error(error),
    }
}

fn special_slot(date: &str, at: &str) -> Result<SpecialSlot, Reply> {
    let date =
        LocalDate::parse(date).ok_or_else(|| Reply::error(400, "date must be YYYY-MM-DD"))?;
    let at = LocalTime::parse(at).ok_or_else(|| Reply::error(400, "time must be HH:MM"))?;
    Ok(SpecialSlot { date, at })
}

/// What a Grid fetch lists right now.
#[derive(Serialize, ToSchema)]
pub struct GridWindowInfo {
    current: GridShowInfo,
    next: GridShowInfo,
}

fn grid_window_info(state: &App) -> GridWindowInfo {
    let [current, next] =
        crate::grid::served_window(&mut lock(&state.schedule), crate::time::now().into()).map(
            |show| GridShowInfo {
                session_id: show.session_id,
                start_utc: show.start_utc,
                end_utc: show.end_utc,
                mode: show.mode.as_str(),
            },
        );
    GridWindowInfo { current, next }
}

/// The current and next show the Grid lists now
#[utoipa::path(get, path = "/admin/grid/window", responses((status = 200, description = "Current and next show as served", body = GridWindowInfo)))]
pub(super) fn grid_window_get(state: &App) -> Reply {
    Reply::json(&grid_window_info(state))
}
