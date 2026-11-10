//! Public status API (`/status/v1/...`): read-only, for a site on the open internet. It carries
//! no XUIDs, addresses or session ids, and names only players whose privacy setting allows it.

mod cache;
mod http;
mod leaderboard;
mod live;
mod ranking;
mod schedule;
mod user_storage;

use std::{sync::Arc, time::Duration};

use serde::Serialize;
use utoipa::{IntoParams, OpenApi, ToSchema};

pub use http::serve;

use crate::{app::App, build_info::BuildInfo, ids::Round};

/// Shared by every connection of the status listener.
pub struct Status {
    app: Arc<App>,
    build: BuildInfo,
    leaderboard: leaderboard::Leaderboard,
    /// The serialized `/live` reply.
    live: cache::Cached<Arc<Vec<u8>>>,
    /// The serialized `/schedule` reply.
    schedule: cache::Cached<Arc<Vec<u8>>>,
}

/// `/live` takes the game's roster, schedule and store locks, so polls share one reading.
const LIVE_LIFETIME: Duration = Duration::from_secs(2);
/// `/schedule` takes the schedule lock and may save shows, so polls share one reading.
const SCHEDULE_LIFETIME: Duration = Duration::from_secs(30);

impl Status {
    pub fn new(app: Arc<App>) -> Self {
        Self {
            app,
            build: crate::build_info::current(),
            leaderboard: leaderboard::Leaderboard::default(),
            live: cache::Cached::default(),
            schedule: cache::Cached::default(),
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "1 vs 100 revival - status API",
        description = "Public, read-only state of one onevs100-gameserver: the server, the show on now, the shows to come, and the all-time leaderboard.",
        version = "1.0.0",
    ),
    paths(server, live_status, schedule_list, leaderboard_page),
    components(schemas(
        ServerInfo,
        LiveInfo,
        ShowSummary,
        ShowType,
        GameType,
        Progress,
        Standings,
        StandingsScope,
        StandingInfo,
        LeaderboardPage,
        ErrorResponse,
    ))
)]
pub struct StatusApiDoc;

pub fn openapi_json() -> String {
    let mut document = StatusApiDoc::openapi();
    // utoipa fills in the crate's licence, and the crate declares none.
    document.info.license = None;
    document
        .to_pretty_json()
        .expect("OpenAPI document serializes")
}

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    error: String,
}

#[derive(Serialize, ToSchema)]
pub struct ServerInfo {
    uptime_secs: u64,
    /// When this binary was built, UTC.
    built_utc: String,
    /// The git commit it was built from, when known.
    commit: Option<String>,
    /// The title build this server speaks: `1149` (v2.0.1149.0) or `1317` (v2.0.1317.0).
    title_build: &'static str,
}

#[derive(Serialize, ToSchema)]
pub struct LiveInfo {
    /// Consoles in a matchmaking session.
    consoles: usize,
    /// Signed-in players on those consoles.
    players: usize,
    /// The show on now; `null` between shows.
    current: Option<ShowSummary>,
    /// The show after the current one.
    next: Option<ShowSummary>,
    /// How far the current show is; `null` when no game is running.
    progress: Option<Progress>,
    /// The current show's latest score board; `null` before its first posted score.
    standings: Option<Standings>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShowType {
    Primetime,
    ExtendedPlay,
    /// A themed episode with questions of one category.
    Special,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GameType {
    /// The One against the Mob.
    Primetime,
    /// Everyone plays as the Mob.
    ExtendedPlay,
}

#[derive(Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct ShowSummary {
    show_type: ShowType,
    game_type: GameType,
    /// A special episode's title.
    title: Option<String>,
    /// A special episode's question category.
    topic: Option<String>,
    /// The questions' language, BCP 47.
    language: String,
    start_utc: String,
    end_utc: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct Progress {
    /// `null` in Extended Play, which has no rounds.
    #[schema(value_type = Option<u32>)]
    round: Option<Round>,
    // The console's `QuestionNumber`: 0 before the round's first question.
    question: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StandingsScope {
    /// Primetime scores start again each round.
    Round,
    /// Extended Play scores run for the whole show.
    Show,
}

#[derive(Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct Standings {
    scope: StandingsScope,
    #[schema(value_type = Option<u32>)]
    round: Option<Round>,
    /// The question after which the scores were posted.
    question: u32,
    /// At most ten.
    entries: Vec<StandingInfo>,
}

#[derive(Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct StandingInfo {
    /// Equal points share a rank.
    rank: usize,
    gamertag: String,
    points: u64,
}

impl From<&ranking::Standing> for StandingInfo {
    fn from(standing: &ranking::Standing) -> Self {
        Self {
            rank: standing.rank.0,
            gamertag: standing.gamertag.as_str().to_owned(),
            points: standing.points.0,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct LeaderboardPage {
    /// Players on the board.
    total: usize,
    offset: usize,
    entries: Vec<StandingInfo>,
}

#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code, reason = "documents the query parameters only")]
struct LeaderboardParams {
    /// First entry to return (default 0).
    offset: Option<usize>,
    /// Entries to return (default 50, at most 100).
    limit: Option<usize>,
}

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;

struct Reply {
    code: u16,
    body: Vec<u8>,
    /// Seconds a cache may keep the reply; `None`: not to be kept.
    max_age_secs: Option<u32>,
}

impl Reply {
    fn json(value: &impl Serialize, max_age_secs: u32) -> Self {
        Self::serialized(
            serde_json::to_vec(value).expect("status replies serialize"),
            max_age_secs,
        )
    }

    fn serialized(body: Vec<u8>, max_age_secs: u32) -> Self {
        Self {
            code: 200,
            body,
            max_age_secs: Some(max_age_secs),
        }
    }

    fn error(code: u16, message: impl Into<String>) -> Self {
        Self {
            code,
            body: serde_json::to_vec(&ErrorResponse {
                error: message.into(),
            })
            .expect("status replies serialize"),
            max_age_secs: None,
        }
    }
}

fn route(status: &Status, method: &str, target: &str) -> Reply {
    if !matches!(method, "GET" | "HEAD") {
        return Reply::error(405, "only GET and HEAD are answered");
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/status/v1/server" => server(status),
        "/status/v1/live" => live_status(status),
        "/status/v1/schedule" => schedule_list(status),
        "/status/v1/leaderboard" => leaderboard_page(status, query),
        _ => Reply::error(404, "no such status resource"),
    }
}

/// The server and its build
#[utoipa::path(get, path = "/status/v1/server", responses((status = 200, body = ServerInfo)))]
fn server(status: &Status) -> Reply {
    Reply::json(
        &ServerInfo {
            uptime_secs: status.app.started.elapsed().as_secs(),
            built_utc: status.build.built.to_grid(),
            commit: status.build.commit.map(|commit| commit.as_str().to_owned()),
            title_build: status.app.sbg.protocol.build(),
        },
        30,
    )
}

/// Players online, the show on now and the next one, and the current scores
#[utoipa::path(get, path = "/status/v1/live", responses((status = 200, body = LiveInfo)))]
fn live_status(status: &Status) -> Reply {
    let body = status.live.get(|| {
        let live = live::snapshot(&status.app, std::time::SystemTime::now());
        let body = serde_json::to_vec(&live).expect("status replies serialize");
        (Arc::new(body), LIVE_LIFETIME)
    });
    Reply::serialized(body.as_ref().clone(), 5)
}

/// The running show and those after it, at most 20; empty without a programme
#[utoipa::path(get, path = "/status/v1/schedule", responses((status = 200, body = Vec<ShowSummary>)))]
fn schedule_list(status: &Status) -> Reply {
    let body = status.schedule.get(|| {
        let shows = schedule::snapshot(&status.app, std::time::SystemTime::now());
        let body = serde_json::to_vec(&shows).expect("status replies serialize");
        (Arc::new(body), SCHEDULE_LIFETIME)
    });
    Reply::serialized(body.as_ref().clone(), 30)
}

/// All-time leaderboard by lifetime score; lists only players whose in-game privacy setting is
/// public
#[utoipa::path(
    get,
    path = "/status/v1/leaderboard",
    params(LeaderboardParams),
    responses(
        (status = 200, body = LeaderboardPage),
        (status = 400, description = "Invalid offset or limit", body = ErrorResponse),
        (status = 503, description = "Player data cannot be read", body = ErrorResponse),
    )
)]
fn leaderboard_page(status: &Status, query: &str) -> Reply {
    let (mut offset, mut limit) = (0, DEFAULT_LIMIT);
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let target = match name {
            "offset" => &mut offset,
            "limit" => &mut limit,
            _ => continue,
        };
        match value.parse() {
            Ok(value) => *target = value,
            Err(_) => return Reply::error(400, format!("{name} must be a whole number")),
        }
    }
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Reply::error(400, format!("limit must be 1 to {MAX_LIMIT}"));
    }
    let storage = crate::sync::lock(&status.app.store).storage();
    let Ok(board) = status.leaderboard.board(storage.as_ref()) else {
        return Reply::error(503, "player data cannot be read");
    };
    Reply::json(
        &LeaderboardPage {
            total: board.len(),
            offset,
            entries: board
                .iter()
                .skip(offset)
                .take(limit)
                .map(StandingInfo::from)
                .collect(),
        },
        60,
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::ids::Xuid;

    pub(super) fn status() -> Status {
        let storage: Arc<dyn crate::storage::Storage> =
            Arc::new(crate::storage::MemoryStorage::default());
        let schedule =
            crate::schedule::Schedule::load(Default::default(), Arc::clone(&storage)).unwrap();
        let app = App::new(
            crate::sbg::Settings::new(crate::wire::Protocol::V1317),
            Arc::new(std::sync::Mutex::new(schedule)),
            crate::store::Store::bootstrap().with_storage(storage),
            crate::matchmaking::Members::default(),
        );
        Status::new(Arc::new(app))
    }

    fn json(reply: &Reply) -> serde_json::Value {
        serde_json::from_slice(&reply.body).unwrap()
    }

    #[test]
    fn only_reads_are_answered() {
        let status = status();
        assert_eq!(route(&status, "POST", "/status/v1/server").code, 405);
        assert_eq!(route(&status, "GET", "/admin/health").code, 404);
        assert_eq!(route(&status, "HEAD", "/status/v1/server").code, 200);
    }

    #[test]
    fn leaderboard_pages_are_cut_from_the_board() {
        let status = status();
        let storage = crate::sync::lock(&status.app.store).storage();
        for (xuid, gamertag, score) in [
            (Xuid(1), "Alpha", "30"),
            (Xuid(2), "Bravo", "20"),
            (Xuid(3), "Charlie", "10"),
        ] {
            leaderboard::tests::player(
                storage.as_ref(),
                xuid,
                gamertag,
                Some(leaderboard::tests::document(score, "Public")),
            );
        }
        let get = |target: &str| route(&status, "GET", target);
        insta::assert_json_snapshot!(json(&get("/status/v1/leaderboard?offset=1&limit=1")));
        assert_eq!(
            json(&get("/status/v1/leaderboard?offset=9"))["entries"],
            serde_json::json!([])
        );
        for bad in ["limit=0", "limit=101", "offset=-1", "limit=many"] {
            assert_eq!(
                get(&format!("/status/v1/leaderboard?{bad}")).code,
                400,
                "{bad}"
            );
        }
    }

    /// The published contract of the public API; a change here is a change for every site
    /// reading it.
    #[test]
    fn the_api_document_is_stable() {
        let document: serde_json::Value = serde_json::from_str(&openapi_json()).unwrap();
        insta::assert_json_snapshot!(document);
    }
}
