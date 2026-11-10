//! Admin API (`/admin/...`); unauthenticated, defaults to loopback.

mod data;
mod errors;
mod http;
mod programme;
mod query;
mod questions;
mod sbg;
mod server;
mod store;

use serde::Serialize;
use utoipa::{OpenApi, ToSchema};

pub use http::serve;
use http::{parse_query, percent_decode, reason};
use query::*;
use {data::*, errors::*, programme::*, questions::*, sbg::*, server::*, store::*};

use crate::app::App;

/// The body of every error reply.
#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    /// What went wrong, for a person to read.
    error: String,
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "1 vs 100 revival - admin API",
        description = "Introspection and recovery controls for the local onevs100-gameserver LAN backend: the live GameState/session store, matchmaking roster, log tails, and clearing a single wedged key without restarting the server.",
        version = "0.1.0",
        license(name = "Unlicensed - internal tool", identifier = "UNLICENSED"),
    ),
    paths(
        health,
        logs,
        log_tail,
        sessions,
        data_players,
        data_delete_player,
        error_reports,
        error_reports_clear,
        sbg_capture,
        sbg_events,
        sbg_recording_start,
        sbg_recording_stop,
        sbg_recordings,
        sbg_recording_file,
        groups_move,
        reset_volatile,
        grid_window_get,
        programme_get,
        programme_set,
        programme_plan,
        shows_get,
        shows_now,
        specials_get,
        special_put,
        special_delete,
        data_export,
        data_import,
        questions_import,
        questions_export,
        categories_get,
        questions_get,
        game_states,
        list_keys,
        get_key,
        clear_key,
        timeline,
    ),
    components(schemas(
        HealthResponse,
        LogFileInfo,
        SessionMemberInfo,
        SessionPlayerInfo,
        SessionReservationInfo,
        ResetInfo,
        GridShowInfo,
        GridWindowInfo,
        ProgrammeInfo,
        PlannedShowInfo,
        ShowInfo,
        SpecialInfo,
        SessionInfo,
        PlayerInfo,
        DeleteResponse,
        ErrorReportInfo,
        ErrorReportFields,
        ErrorReportsClearResponse,
        SbgCaptureInfo,
        SbgRecordingInfo,
        SbgRecordingFailure,
        SbgRecordingFileInfo,
        SbgEventInfo,
        SbgEventKind,
        ImportResponse,
        ErrorResponse,
        GameStatesInfo,
        GameStateInfo,
        QuestionsImportResponse,
        CategoryInfo,
        QuestionInfo,
        KeyInfo,
        StoreValue,
        ClearResponse,
        TimelineEvent,
    ))
)]
pub struct ApiDoc;

pub fn openapi_json() -> String {
    ApiDoc::openapi()
        .to_pretty_json()
        .expect("OpenAPI document serializes")
}

struct Reply {
    code: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Reply {
    fn json(value: &impl Serialize) -> Self {
        Self {
            code: 200,
            content_type: "application/json",
            body: serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec()),
        }
    }

    fn text(code: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            code,
            content_type: "text/plain; charset=utf-8",
            body: body.into(),
        }
    }

    /// Every error is `{"error": "..."}`, so the generated client reads it like any reply.
    fn error(code: u16, message: impl Into<String>) -> Self {
        Self {
            code,
            content_type: "application/json",
            body: serde_json::to_vec(&ErrorResponse {
                error: message.into(),
            })
            .unwrap_or_else(|_| b"null".to_vec()),
        }
    }

    /// An error that needs no more words than its reason phrase.
    fn status(code: u16) -> Self {
        Self::error(code, reason(code))
    }
}

#[cfg(test)]
fn route(state: &App, method: &str, target: &str) -> Reply {
    route_with_body(state, method, target, &[])
}

fn route_with_body(state: &App, method: &str, target: &str, body: &[u8]) -> Reply {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = parse_query(query);
    let Some(path) = percent_decode(path) else {
        return Reply::status(400);
    };
    tracing::trace!(method, %path, "admin request");
    if let Some(rest) = path.strip_prefix("/admin/") {
        return route_api(state, method, rest, &query, target, body);
    }
    // The frontend is served standalone (nginx); this daemon only speaks the API.
    Reply::status(404)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    /// GET, and HEAD (the same reply without its body).
    Get,
    Post,
}

/// What a handler is given: the path's `{...}` segments in order, the query and the body.
struct Call<'a> {
    params: Vec<&'a str>,
    query: &'a Query,
    body: &'a [u8],
}

impl Call<'_> {
    fn param(&self, index: usize) -> &str {
        self.params[index]
    }
}

struct Route {
    method: Method,
    /// Below `/admin/`, with `{name}` for a segment the handler is given, named as in the
    /// OpenAPI document.
    path: &'static str,
    handler: fn(&App, &Call) -> Reply,
}

/// Every admin call; literals come before patterns they overlap.
const ROUTES: &[Route] = &[
    Route {
        method: Method::Get,
        path: "health",
        handler: |state, _| health(state),
    },
    Route {
        method: Method::Get,
        path: "logs",
        handler: |_, _| logs(),
    },
    Route {
        method: Method::Get,
        path: "log/{name}",
        handler: |_, call| log_tail(call.param(0), call.query),
    },
    Route {
        method: Method::Get,
        path: "sessions",
        handler: |state, _| sessions(state),
    },
    Route {
        method: Method::Get,
        path: "timeline",
        handler: |_, call| timeline(call.query),
    },
    Route {
        method: Method::Get,
        path: "gamestates",
        handler: |state, _| game_states(state),
    },
    Route {
        method: Method::Get,
        path: "store/keys",
        handler: |state, call| list_keys(state, call.query),
    },
    Route {
        method: Method::Get,
        path: "store/{key}",
        handler: |state, call| get_key(state, call.param(0)),
    },
    Route {
        method: Method::Post,
        path: "store/{key}/clear",
        handler: |state, call| clear_key(state, call.param(0)),
    },
    Route {
        method: Method::Post,
        path: "groups/move",
        handler: |state, call| groups_move(state, call.query),
    },
    Route {
        method: Method::Post,
        path: "reset",
        handler: |state, _| reset_volatile(state),
    },
    Route {
        method: Method::Get,
        path: "grid/window",
        handler: |state, _| grid_window_get(state),
    },
    Route {
        method: Method::Get,
        path: "programme",
        handler: |state, _| programme_get(state),
    },
    Route {
        method: Method::Post,
        path: "programme",
        handler: |state, call| programme_set(state, call.query),
    },
    Route {
        method: Method::Get,
        path: "programme/plan",
        handler: |state, call| programme_plan(state, call.query),
    },
    Route {
        method: Method::Get,
        path: "shows",
        handler: |state, _| shows_get(state),
    },
    Route {
        method: Method::Post,
        path: "shows/now",
        handler: |state, call| shows_now(state, call.query),
    },
    Route {
        method: Method::Get,
        path: "specials",
        handler: |state, _| specials_get(state),
    },
    Route {
        method: Method::Post,
        path: "specials/{date}/{at}",
        handler: |state, call| special_put(state, call.param(0), call.param(1), call.query),
    },
    Route {
        method: Method::Post,
        path: "specials/{date}/{at}/delete",
        handler: |state, call| special_delete(state, call.param(0), call.param(1)),
    },
    Route {
        method: Method::Get,
        path: "data/players",
        handler: |state, _| data_players(state),
    },
    Route {
        method: Method::Post,
        path: "data/players/{xuid}/delete",
        handler: |state, call| data_delete_player(state, call.param(0)),
    },
    Route {
        method: Method::Get,
        path: "errors",
        handler: |state, call| error_reports(state, call.query),
    },
    Route {
        method: Method::Post,
        path: "errors/clear",
        handler: |state, _| error_reports_clear(state),
    },
    Route {
        method: Method::Get,
        path: "sbg/capture",
        handler: |state, _| sbg_capture(state),
    },
    Route {
        method: Method::Get,
        path: "sbg/events",
        handler: |state, call| sbg_events(state, call.query),
    },
    Route {
        method: Method::Post,
        path: "sbg/recording/start",
        handler: |state, _| sbg_recording_start(state),
    },
    Route {
        method: Method::Post,
        path: "sbg/recording/stop",
        handler: |state, _| sbg_recording_stop(state),
    },
    Route {
        method: Method::Get,
        path: "sbg/recordings",
        handler: |_, _| sbg_recordings(),
    },
    Route {
        method: Method::Get,
        path: "sbg/recordings/{name}",
        handler: |_, call| sbg_recording_file(call.param(0)),
    },
    Route {
        method: Method::Get,
        path: "data/export",
        handler: |state, _| data_export(state),
    },
    Route {
        method: Method::Post,
        path: "data/import",
        handler: |state, call| data_import(state, call.query, call.body),
    },
    Route {
        method: Method::Get,
        path: "questions",
        handler: |state, call| questions_get(state, call.query),
    },
    Route {
        method: Method::Get,
        path: "questions/export",
        handler: |state, _| questions_export(state),
    },
    Route {
        method: Method::Post,
        path: "questions/import",
        handler: |state, call| questions_import(state, call.query, call.body),
    },
    Route {
        method: Method::Get,
        path: "categories",
        handler: |state, _| categories_get(state),
    },
];

/// The `{...}` segments of `segments` if they fit `pattern`.
fn matches<'a>(pattern: &str, segments: &[&'a str]) -> Option<Vec<&'a str>> {
    let parts: Vec<&str> = pattern.split('/').collect();
    if parts.len() != segments.len() {
        return None;
    }
    let mut params = Vec::new();
    for (part, segment) in parts.iter().zip(segments) {
        if part.starts_with('{') {
            params.push(*segment);
        } else if part != segment {
            return None;
        }
    }
    Some(params)
}

fn route_api(
    state: &App,
    method: &str,
    rest: &str,
    query: &Query,
    raw_target: &str,
    body: &[u8],
) -> Reply {
    let method = match method {
        "GET" | "HEAD" => Some(Method::Get),
        "POST" => Some(Method::Post),
        _ => None,
    };
    // Keys are matched on the decoded path; they never contain `/`.
    let segments: Vec<_> = rest.split('/').collect();
    let mut path_known = false;
    for route in ROUTES {
        let Some(params) = matches(route.path, &segments) else {
            continue;
        };
        if Some(route.method) == method {
            return (route.handler)(
                state,
                &Call {
                    params,
                    query,
                    body,
                },
            );
        }
        path_known = true;
    }
    if path_known {
        return Reply::status(405);
    }
    tracing::debug!(raw_target, "unknown admin route");
    Reply::status(404)
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::{http::answer, *};
    use crate::{
        ids::Xuid, matchmaking::Members, questions::QuestionBank, store::Store, sync::lock,
        time::Utc,
    };

    /// No programme, as a fresh database before the server seeds one; the schedule saves into
    /// the store's storage, as on a server.
    fn state() -> App {
        let store = Store::bootstrap();
        let schedule =
            crate::schedule::Schedule::load(Default::default(), store.storage()).unwrap();
        App::new(
            crate::sbg::Settings::new(crate::wire::Protocol::V1317),
            std::sync::Arc::new(std::sync::Mutex::new(schedule)),
            store,
            Members::default(),
        )
    }

    /// With the default programme.
    fn programmed() -> App {
        let state = state();
        assert_eq!(route(&state, "POST", "/admin/programme").code, 200);
        state
    }

    /// State on SQLite in a fresh directory, so a test can break the schedule's storage; with
    /// the directory to remove while the state is dropped.
    fn sqlite_state(name: &str) -> (App, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("onevs100-admin-{name}-{}", std::process::id()));
        let path = dir.join("onevs100.db");
        let storage: std::sync::Arc<dyn crate::storage::Storage> = std::sync::Arc::new(
            crate::storage::SqliteStorage::open(&path).expect("a new database"),
        );
        let schedule =
            crate::schedule::Schedule::load(Default::default(), std::sync::Arc::clone(&storage))
                .unwrap();
        let store = Store::bootstrap().with_storage(storage);
        let state = App::new(
            crate::sbg::Settings::new(crate::wire::Protocol::V1317),
            std::sync::Arc::new(std::sync::Mutex::new(schedule)),
            store,
            Members::default(),
        );
        (state, dir)
    }

    fn get(state: &App, target: &str) -> Reply {
        route(state, "GET", target)
    }

    #[test]
    fn data_endpoints_list_export_import_and_delete_players() {
        let state = state();
        let storage = lock(&state.store).storage();
        storage
            .touch_player(
                Xuid(0xE000_0690_52E8_AD19),
                Some("Player1"),
                crate::time::Utc::from_unix_secs(100),
            )
            .unwrap();
        storage
            .set_avatar(
                Xuid(0xE000_0690_52E8_AD19),
                &[1; 1000],
                crate::time::Utc::from_unix_secs(110),
            )
            .unwrap();
        storage
            .put_blob(
                crate::storage::BlobKind::UserStorage,
                Xuid(0xE000_0690_52E8_AD19),
                b"<doc/>",
                crate::time::Utc::from_unix_secs(120),
            )
            .unwrap();
        storage
            .counter_add("v3.Stats.Sum.TotalSeen.1", 3.0)
            .unwrap();

        let players = get(&state, "/admin/data/players");
        assert_eq!(players.code, 200);
        let players: serde_json::Value = serde_json::from_str(&body(&players)).unwrap();
        assert_eq!(players[0]["xuid_hex"], "e000069052e8ad19");
        assert_eq!(players[0]["gamertag"], "Player1");
        assert_eq!(players[0]["avatar_bytes"], 1000);
        assert_eq!(players[0]["first_seen_unix_secs"], 100);
        assert_eq!(players[0]["last_seen_unix_secs"], 110);
        assert_eq!(players[0]["user_storage_bytes"], 6);
        assert_eq!(players[0]["user_stats_bytes"], 0);

        let export = get(&state, "/admin/data/export");
        assert_eq!(export.code, 200);
        let exported = body(&export);
        assert!(exported.contains("\"version\": 1") || exported.contains("\"version\":1"));

        // Delete, then restore from the export.
        let deleted = route(
            &state,
            "POST",
            "/admin/data/players/e000069052e8ad19/delete",
        );
        assert!(body(&deleted).contains("true"));
        assert_eq!(body(&get(&state, "/admin/data/players")), "[]");
        let restored = route_with_body(
            &state,
            "POST",
            "/admin/data/import?mode=merge",
            exported.as_bytes(),
        );
        assert_eq!(restored.code, 200, "{}", body(&restored));
        assert!(body(&restored).contains("\"players\":1"));
        assert!(body(&get(&state, "/admin/data/players")).contains("Player1"));

        // Bad input is a 400 and changes nothing.
        assert_eq!(
            route_with_body(&state, "POST", "/admin/data/import?mode=merge", b"{nope").code,
            400
        );
        assert_eq!(
            route_with_body(
                &state,
                "POST",
                "/admin/data/import?mode=sideways",
                exported.as_bytes()
            )
            .code,
            400
        );
        assert_eq!(
            route(&state, "POST", "/admin/data/import?mode=merge").code,
            400
        );
        assert_eq!(
            route(&state, "POST", "/admin/data/players/zz/delete").code,
            400
        );
        assert_eq!(get(&state, "/admin/data/import").code, 405);
    }

    #[test]
    fn an_imported_schedule_is_served_without_a_restart() {
        let source = programmed();
        let dump = body(&get(&source, "/admin/data/export"));

        let state = state();
        assert!(!lock(&state.schedule).is_enabled());
        let reply = route_with_body(
            &state,
            "POST",
            "/admin/data/import?mode=replace",
            dump.as_bytes(),
        );
        assert_eq!(reply.code, 200, "{}", body(&reply));
        assert!(lock(&state.schedule).is_enabled());
        assert_eq!(
            body(&get(&state, "/admin/programme")),
            body(&get(&source, "/admin/programme"))
        );
    }

    #[test]
    fn an_imported_question_bank_is_served_without_a_restart() {
        let source = state();
        let mut bank = QuestionBank::seed().to_file();
        bank.questions.truncate(5);
        let bank = QuestionBank::from_file(bank).unwrap();
        lock(&source.store)
            .storage()
            .import_questions(&bank, crate::storage::ImportMode::Replace)
            .unwrap();
        let dump = body(&get(&source, "/admin/data/export"));

        let state = state();
        let reply = route_with_body(
            &state,
            "POST",
            "/admin/data/import?mode=replace",
            dump.as_bytes(),
        );
        assert_eq!(reply.code, 200, "{}", body(&reply));
        let summary: serde_json::Value = serde_json::from_str(&body(&reply)).unwrap();
        assert_eq!(summary["questions"], 5);
        assert_eq!(lock(&state.store).questions().questions().len(), 5);
    }

    const BANK: &str = r#"{"version":1,
        "sources":[{"id":"otdb","name":"Open Trivia Database","licence":"CC BY-SA 4.0",
                    "url":"https://opentdb.com"}],
        "categories":[{"id":"otdb:9","name":"General Knowledge","source":"otdb"},
                      {"id":"otdb:11","name":"Film","source":"otdb"}],
        "questions":[
          {"id":1,"difficulty":1,"text":"One","choices":["A","B","C"],"correct":1,
           "categories":["otdb:9"],"source":"otdb","origin":{"category":"9"}},
          {"id":2,"difficulty":2,"text":"Two","choices":["A","B","C"],"correct":2,
           "categories":["otdb:9","otdb:11"],"source":"otdb"},
          {"id":3,"difficulty":2,"text":"Three","choices":["A","B","C"],"correct":3,
           "categories":["otdb:11"],"source":"otdb"}]}"#;

    #[test]
    fn questions_are_imported_listed_counted_and_exported() {
        let state = state();
        let json = |reply: Reply| -> serde_json::Value {
            assert_eq!(reply.code, 200, "{}", body(&reply));
            serde_json::from_str(&body(&reply)).unwrap()
        };
        let exported = json(get(&state, "/admin/questions/export"));
        assert_eq!(
            exported["questions"],
            serde_json::json!([]),
            "nothing imported yet"
        );
        assert_eq!(
            json(get(&state, "/admin/questions?limit=1000"))
                .as_array()
                .unwrap()
                .len(),
            41,
            "the built-in bank is served"
        );
        assert_eq!(
            json(get(&state, "/admin/categories")),
            serde_json::json!([])
        );

        let import =
            |target: &str, bank: &str| route_with_body(&state, "POST", target, bank.as_bytes());
        let invalid = BANK.replace(r#""correct":3"#, r#""correct":4"#);
        let refused = import("/admin/questions/import", &invalid);
        assert_eq!(refused.code, 400);
        assert!(
            body(&refused).contains("4 names no answer choice"),
            "{}",
            body(&refused)
        );
        assert_eq!(import("/admin/questions/import", "{nope").code, 400);
        assert_eq!(
            import("/admin/questions/import?mode=sideways", BANK).code,
            400
        );

        let summary = json(import("/admin/questions/import?mode=replace", BANK));
        assert_eq!(summary, serde_json::json!({"imported": 3, "questions": 3}));
        assert_eq!(
            lock(&state.store).questions().questions().len(),
            3,
            "served at once"
        );

        let categories = json(get(&state, "/admin/categories"));
        assert_eq!(categories[0]["id"], "otdb:9");
        assert_eq!(categories[0]["counts"], serde_json::json!([1, 1, 0, 0, 0]));
        assert_eq!(categories[1]["counts"], serde_json::json!([0, 2, 0, 0, 0]));

        let ids = |target: &str| -> Vec<u64> {
            json(get(&state, target))
                .as_array()
                .unwrap()
                .iter()
                .map(|q| q["id"].as_u64().unwrap())
                .collect()
        };
        assert_eq!(ids("/admin/questions"), [1, 2, 3]);
        assert_eq!(ids("/admin/questions?category=otdb:11"), [2, 3]);
        assert_eq!(ids("/admin/questions?category=otdb:9&difficulty=2"), [2]);
        assert_eq!(ids("/admin/questions?limit=1"), [1]);
        let first = &json(get(&state, "/admin/questions?limit=1"))[0];
        assert_eq!(first["source_name"], "Open Trivia Database");
        assert_eq!(first["origin_category"], "9");
        assert_eq!(
            first["origin_id"],
            serde_json::Value::Null,
            "OpenTriviaDB has none"
        );
        assert_eq!(first["licence"], "CC BY-SA 4.0");
        assert_eq!(first["url"], "https://opentdb.com");
        assert_eq!(get(&state, "/admin/questions?difficulty=6").code, 400);
        assert_eq!(get(&state, "/admin/questions?category=otdb:99").code, 400);

        let more = BANK
            .replace(r#""text":"Three""#, r#""text":"Three again""#)
            .replace(r#""id":1,"#, r#""id":4,"#);
        let summary = json(import("/admin/questions/import", &more));
        assert_eq!(
            summary,
            serde_json::json!({"imported": 3, "questions": 4}),
            "merged by id"
        );
        let exported = json(get(&state, "/admin/questions/export"));
        assert_eq!(exported["questions"][2]["text"], "Three again");
        assert_eq!(exported["questions"][3]["id"], 4);
        let round_trip = route_with_body(
            &state,
            "POST",
            "/admin/questions/import?mode=replace",
            exported.to_string().as_bytes(),
        );
        assert_eq!(json(round_trip)["questions"], 4);

        assert_eq!(route(&state, "POST", "/admin/questions").code, 405);
        assert_eq!(get(&state, "/admin/questions/import").code, 405);
        assert_eq!(route(&state, "POST", "/admin/categories").code, 405);
        let openapi = openapi_json();
        for id in [
            "questions_import",
            "questions_export",
            "categories_get",
            "questions_get",
        ] {
            assert!(
                openapi.contains(&format!("\"operationId\": \"{id}\"")),
                "missing {id}"
            );
        }
    }

    /// A bank file whose question names a source the file does not list.
    const BANK_WITH_A_STRANGER: &str = r#"{"version":1,
        "sources":[],
        "categories":[],
        "questions":[
          {"id":7,"difficulty":1,"text":"From elsewhere","choices":["A","B","C"],
           "correct":1,"categories":[],"source":"elsewhere"}]}"#;

    #[test]
    fn a_question_of_a_source_the_bank_does_not_list_imports_with_null_source_fields() {
        let state = state();
        let imported = route_with_body(
            &state,
            "POST",
            "/admin/questions/import?mode=replace",
            BANK_WITH_A_STRANGER.as_bytes(),
        );
        assert_eq!(imported.code, 200, "{}", body(&imported));
        let reply = get(&state, "/admin/questions");
        assert_eq!(reply.code, 200, "{}", body(&reply));
        let question: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(question[0]["source"], "elsewhere");
        assert_eq!(
            question[0]["source_name"],
            serde_json::Value::Null,
            "no name to credit it to"
        );
        assert_eq!(question[0]["licence"], serde_json::Value::Null);
        assert_eq!(question[0]["url"], serde_json::Value::Null);
    }

    /// The message of an error reply, which is always `{"error": "..."}`.
    fn error_of(reply: &Reply) -> String {
        assert_eq!(reply.content_type, "application/json");
        let value: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        value["error"].as_str().unwrap().to_owned()
    }

    #[test]
    fn a_plan_previews_programme_changes_without_saving_them() {
        let state = state();
        assert_eq!(
            get(&state, "/admin/programme/plan").code,
            404,
            "no programme"
        );
        let starts = |target: &str| -> Vec<String> {
            let reply = get(&state, target);
            assert_eq!(reply.code, 200, "{}", body(&reply));
            let shows: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
            shows
                .as_array()
                .unwrap()
                .iter()
                .filter(|show| show["kind"] == "primetime")
                .map(|show| show["start_local"].as_str().unwrap()[11..].to_owned())
                .collect()
        };
        let preview = starts("/admin/programme/plan?days=3&primetime_at=21:30");
        assert!(
            !preview.is_empty() && preview.iter().all(|at| at == "21:30"),
            "{preview:?}"
        );
        assert_eq!(get(&state, "/admin/programme").code, 404, "nothing saved");

        assert_eq!(route(&state, "POST", "/admin/programme").code, 200);
        assert!(
            starts("/admin/programme/plan?days=3")
                .iter()
                .all(|at| at == "20:00")
        );
        let longer = starts("/admin/programme/plan?days=3&primetime_minutes=90&zone=Europe/Berlin");
        assert!(
            longer.iter().all(|at| at == "20:00"),
            "the saved time, in the previewed zone"
        );
        let saved = body(&get(&state, "/admin/programme"));
        assert!(saved.contains("\"primetime_minutes\":60"), "{saved}");
        assert!(saved.contains("America/New_York"), "{saved}");

        let bad = get(&state, "/admin/programme/plan?primetime_minutes=500");
        assert_eq!(bad.code, 400);
        assert!(
            error_of(&bad).contains("Primetime length"),
            "{}",
            error_of(&bad)
        );
        assert_eq!(
            error_of(&get(&state, "/admin/programme/plan?zone=Nowhere")),
            "unknown time zone \"Nowhere\""
        );
    }

    #[test]
    fn every_route_answers_the_other_method_with_405() {
        // From the OpenAPI document, so a route the 405 list forgets is caught.
        let state = programmed();
        let doc: serde_json::Value = serde_json::from_str(&openapi_json()).unwrap();
        let mut checked = 0;
        for (path, methods) in doc["paths"].as_object().unwrap() {
            let concrete: String = path
                .split('/')
                .map(|segment| {
                    if segment.starts_with('{') {
                        "x"
                    } else {
                        segment
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            for method in methods.as_object().unwrap().keys() {
                let other = match method.as_str() {
                    "get" => "POST",
                    "post" => "GET",
                    other => panic!("{path}: unexpected method {other}"),
                };
                if methods.get(other.to_lowercase()).is_some() {
                    continue;
                }
                let reply = route(&state, other, &concrete);
                assert_eq!(reply.code, 405, "{other} {concrete}");
                checked += 1;
            }
        }
        assert!(checked >= 20, "{checked} checked");
        for unknown in ["/admin/schedule", "/admin/schedule/add"] {
            assert_eq!(
                route(&state, "POST", unknown).code,
                404,
                "{unknown} is not an admin call"
            );
        }
    }

    #[test]
    fn game_states_are_read_on_the_server() {
        let state = state();
        let qa = include_bytes!("../../fixtures/native-r12-qa-251317.xml");
        {
            let mut store = lock(&state.store);
            store.set("v3.Session.MGS.1vs100.Store.P.GameState", qa.to_vec());
            store.set(
                "v3.Session.MGS.1vs100.Store.Q.GameState",
                b"not xml".to_vec(),
            );
            store.set("v3.Session.MGS.1vs100.Sum.AnswerCount", b"1".to_vec());
        }
        let reply = get(&state, "/admin/gamestates");
        assert_eq!(reply.code, 200);
        let info: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(info["phases"][0], "ElectHost");
        assert_eq!(info["phases"].as_array().unwrap().len(), 10);
        let states = info["states"].as_array().unwrap();
        assert_eq!(states.len(), 2, "only GameState keys");
        let qa_state = &states[0];
        assert_eq!(
            qa_state["session"], "p",
            "as the key parser reads it (lowercase)"
        );
        assert_eq!(qa_state["name"], "QA");
        assert_eq!(qa_state["known"], true);
        assert_eq!(
            (qa_state["round"].as_u64(), qa_state["question"].as_u64()),
            (Some(1), Some(1))
        );
        assert_eq!(qa_state["start_utc"], "2026-09-30T14:11:30Z");
        assert_eq!(qa_state["end_utc"], "2026-09-30T14:11:54Z");
        assert_eq!(qa_state["xml"].as_str().unwrap().as_bytes(), qa);
        let unreadable = &states[1];
        assert_eq!(
            (unreadable["name"].is_null(), unreadable["known"].as_bool()),
            (true, Some(false))
        );
    }

    #[test]
    fn queries_are_read_through_their_documented_structs() {
        // The messages are the ones the hand-written parsing gave.
        let state = programmed();
        let error = |method: &str, target: &str| {
            let reply = route(&state, method, target);
            assert_eq!(reply.code, 400, "{method} {target}");
            error_of(&reply)
        };
        assert_eq!(
            error("GET", "/admin/timeline?limit=many"),
            "invalid `limit`"
        );
        assert_eq!(
            error("POST", "/admin/programme?primetime_minutes=-5"),
            "invalid `primetime_minutes`"
        );
        assert_eq!(error("POST", "/admin/groups/move"), "missing `xuid`");
        assert_eq!(
            error("POST", "/admin/groups/move?xuid=zz&group=2"),
            "xuid must be hex"
        );
        assert_eq!(
            error("POST", "/admin/groups/move?xuid=e000000000000001"),
            "missing `group`"
        );
        assert_eq!(
            error(
                "POST",
                "/admin/specials/2030-01-01/20:00?title=&category=music"
            ),
            "missing `title`"
        );
        assert_eq!(
            error("POST", "/admin/specials/2030-01-01/20:00?title=x"),
            "missing `category`"
        );
        assert_eq!(
            error("POST", "/admin/shows/now?mode=xx"),
            "mode must be pt or ep"
        );
        assert_eq!(
            error("POST", "/admin/questions/import?mode=sideways"),
            "mode must be merge or replace"
        );
        assert_eq!(
            error("GET", "/admin/questions?difficulty=9"),
            "difficulty must be 1-5"
        );
        // Names a call does not take are ignored; `+` is a space.
        assert_eq!(
            get(&state, "/admin/programme/plan?days=3&colour=blue").code,
            200
        );
        let keys = get(&state, "/admin/store/keys?prefix=no+such+prefix");
        assert_eq!(body(&keys), "[]");
        // The documentation comes from the same structs.
        let doc = openapi_json();
        for name in [
            "primetime_minutes",
            "min_extended_play_minutes",
            "days",
            "title",
            "group",
            "mode",
            "lines",
            "prefix",
        ] {
            assert!(
                doc.contains(&format!("\"name\": \"{name}\"")),
                "{name} documented"
            );
        }
    }

    #[test]
    fn a_group_move_answers_with_the_why_of_its_refusal() {
        let state = state();
        {
            let mut members = lock(&state.members);
            members.insert_test_console("s", Xuid(1), &["P1", "P2", "P3", "P4"]);
            members.insert_test_console("s", Xuid(5), &["P5", "P6"]);
        }
        let moved = route(
            &state,
            "POST",
            "/admin/groups/move?xuid=0000000000000001&group=2",
        );
        assert_eq!((moved.code, body(&moved).as_str()), (200, "moved"));
        let full = route(
            &state,
            "POST",
            "/admin/groups/move?xuid=0000000000000005&group=2",
        );
        assert_eq!(
            (full.code, error_of(&full).as_str()),
            (409, "group 2 is full")
        );
        let invalid = route(
            &state,
            "POST",
            "/admin/groups/move?xuid=0000000000000005&group=0",
        );
        assert_eq!(
            (invalid.code, error_of(&invalid).as_str()),
            (400, "group 0 is invalid")
        );
        let unknown = route(
            &state,
            "POST",
            "/admin/groups/move?xuid=0000000000000099&group=1",
        );
        assert_eq!(
            (unknown.code, error_of(&unknown).as_str()),
            (404, "no console with first player 0000000000000099")
        );
    }

    #[test]
    fn every_error_is_json_with_a_message() {
        let state = state();
        let unknown_zone = route(&state, "POST", "/admin/programme?zone=Mars/Olympus");
        assert_eq!(unknown_zone.code, 400);
        assert_eq!(
            error_of(&unknown_zone),
            "unknown time zone \"Mars/Olympus\""
        );
        let missing = get(&state, "/admin/no-such-thing");
        assert_eq!(
            (missing.code, error_of(&missing).as_str()),
            (404, "Not Found")
        );
        let wrong_method = route(&state, "POST", "/admin/health");
        assert_eq!(
            (wrong_method.code, error_of(&wrong_method).as_str()),
            (405, "Method Not Allowed")
        );
        let none = get(&state, "/admin/programme");
        assert_eq!(
            error_of(&none),
            "no programme: the unscheduled show is served"
        );
        assert!(
            openapi_json().contains("\"ErrorResponse\""),
            "the error schema is documented"
        );
    }

    fn body(reply: &Reply) -> String {
        String::from_utf8(reply.body.clone()).unwrap()
    }

    #[test]
    fn health_reports_uptime_and_no_sbg_activity_for_fresh_state() {
        let reply = get(&state(), "/admin/health");
        assert_eq!(reply.code, 200);
        assert!(body(&reply).contains("\"uptime_secs\":"));
    }

    #[test]
    fn error_reports_are_listed_with_their_fields_and_cleared() {
        let state = state();
        let storage = lock(&state.store).storage();
        let report = |ms, document: &[u8]| crate::storage::ErrorReport {
            received: Utc::from_unix_ms(ms),
            console: "192.0.2.7".parse().unwrap(),
            key: "v3.Session.MGS.1vs100.Log.UserError.LobbyFPP".into(),
            document: document.to_vec(),
        };
        storage
            .add_error_report(&report(1_000, b"not a report"), 10)
            .unwrap();
        storage
            .add_error_report(&report(2_000, b"<xsor:object>errorName: LobbyFPP, errorReason: a &amp; b, Fatal: Exit, UTC: 1970-01-01T00:00:01.500Z, State: Lobby, Sub: NO_SUBSTATE, R: 0, Q: 0, key: NO_MSG, NB: 0</xsor:object>"), 10)
            .unwrap();
        let listed: serde_json::Value =
            serde_json::from_str(&body(&get(&state, "/admin/errors"))).unwrap();
        insta::assert_json_snapshot!(listed);
        let newest: serde_json::Value =
            serde_json::from_str(&body(&get(&state, "/admin/errors?limit=1"))).unwrap();
        assert_eq!(newest.as_array().unwrap().len(), 1);
        assert_eq!(get(&state, "/admin/errors?limit=many").code, 400);
        let cleared = route(&state, "POST", "/admin/errors/clear");
        assert_eq!(body(&cleared), "{\"removed\":2}");
        assert_eq!(body(&get(&state, "/admin/errors")), "[]");
    }

    #[test]
    fn captured_sbg_frames_are_listed_decoded_and_filtered_by_key() {
        let store = Store::bootstrap();
        let schedule =
            crate::schedule::Schedule::load(Default::default(), store.storage()).unwrap();
        // The recording's build, so its replies decode.
        let state = App::new(
            crate::sbg::Settings::new(crate::wire::Protocol::V1149),
            std::sync::Arc::new(std::sync::Mutex::new(schedule)),
            store,
            Members::default(),
        );
        let text = include_str!("../../fixtures/sessions/2026-10-23-1149-money-or-mob.sbg");
        let recorded = crate::sbg::record::parse(text).unwrap();
        let events = &recorded.events[..12];
        for event in events {
            state
                .capture
                .record(event.conn, event.peer, event.kind, &event.frame);
        }
        let begun = std::time::Instant::now();
        while state.capture.status().recent_events < events.len() {
            assert!(begun.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let listed = |path: &str| {
            let mut listed: serde_json::Value =
                serde_json::from_str(&body(&get(&state, path))).unwrap();
            // Captured now: the time is the test's own.
            for event in listed.as_array_mut().unwrap() {
                event["at_unix_ms"] = 0.into();
            }
            listed
        };
        insta::assert_json_snapshot!(listed("/admin/sbg/events?key=pangolin&limit=4"));
        assert_eq!(
            listed("/admin/sbg/events?conn=99")
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            listed("/admin/sbg/events?limit=3")
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(get(&state, "/admin/sbg/events?peer=nowhere").code, 400);
        assert_eq!(get(&state, "/admin/sbg/events?xuid=zz").code, 400);
        assert_eq!(
            listed("/admin/sbg/events?key=&limit=2")
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let status: serde_json::Value =
            serde_json::from_str(&body(&get(&state, "/admin/sbg/capture"))).unwrap();
        assert_eq!(status["recent_events"], events.len());
        assert_eq!(status["recording"], serde_json::Value::Null);
        assert_eq!(route(&state, "POST", "/admin/sbg/recording/stop").code, 409);
    }

    #[test]
    fn store_key_roundtrip_get_then_clear() {
        let state = state();
        let key = "v3.Setup.MGS.1vs100.Store.Config.Week1";
        let reply = get(&state, &format!("/admin/store/{key}"));
        assert_eq!(reply.code, 200);
        assert!(body(&reply).contains(key));
        let keys = get(
            &state,
            "/admin/store/keys?prefix=v3.Setup.MGS.1vs100.Store.Config",
        );
        assert!(body(&keys).contains(key));

        let cleared = route(&state, "POST", &format!("/admin/store/{key}/clear"));
        assert_eq!(body(&cleared), "{\"removed\":true}");
        assert_eq!(get(&state, &format!("/admin/store/{key}")).code, 404);
        let again = route(&state, "POST", &format!("/admin/store/{key}/clear"));
        assert_eq!(body(&again), "{\"removed\":false}");
    }

    #[test]
    fn methods_and_unknown_routes_are_rejected() {
        let state = state();
        assert_eq!(route(&state, "POST", "/admin/health").code, 405);
        assert_eq!(get(&state, "/admin/reset").code, 405);
        assert_eq!(get(&state, "/admin/nope").code, 404);
        assert_eq!(get(&state, "/").code, 404);
    }

    #[test]
    fn error_statuses_carry_their_reason_phrase() {
        for (code, phrase) in [
            (409, "Conflict"),
            (413, "Payload Too Large"),
            (500, "Internal Server Error"),
        ] {
            assert_eq!(reason(code), phrase);
        }
    }

    /// The status line of the reply to `request` as it came off the socket.
    fn status_line(state: &App, request: &str) -> String {
        let reply = answer(state, request.as_bytes());
        let reply = String::from_utf8_lossy(&reply);
        reply.lines().next().unwrap_or_default().to_owned()
    }

    #[test]
    fn a_request_body_needs_one_valid_content_length() {
        let state = state();
        let request = |headers: &str, body: &str| {
            format!("POST /admin/reset HTTP/1.1\r\nHost: x\r\n{headers}\r\n{body}")
        };
        assert_eq!(
            status_line(&state, &request("Content-Length: 2\r\n", "{}")),
            "HTTP/1.1 200 OK"
        );
        assert_eq!(status_line(&state, &request("", "")), "HTTP/1.1 200 OK");
        for bad in [
            "Content-Length: twelve\r\n",
            "Content-Length: 2\r\nContent-Length: 2\r\n",
            "Transfer-Encoding: chunked\r\n",
        ] {
            assert_eq!(
                status_line(&state, &request(bad, "{}")),
                "HTTP/1.1 400 Bad Request",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn admin_requests_are_read_by_the_consoles_strict_parser() {
        let state = state();
        for bad in [
            "GET /admin/health HTTP/1.1\r\nno colon here\r\n\r\n",
            "GET /admin/health HTTP/2\r\n\r\n",
            "GET admin/health HTTP/1.1\r\n\r\n",
            "GET /admin/health HTTP/1.1 extra\r\n\r\n",
        ] {
            assert_eq!(
                status_line(&state, bad),
                "HTTP/1.1 400 Bad Request",
                "{bad:?}"
            );
        }
        assert_eq!(
            status_line(&state, "GET /admin/health?x=1 HTTP/1.1\r\nHost: x\r\n\r\n"),
            "HTTP/1.1 200 OK",
            "the query reaches the router"
        );
        let large = format!(
            "POST /admin/reset HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            32 * 1024 * 1024 + 1
        );
        assert_eq!(
            status_line(&state, &large),
            "HTTP/1.1 413 Payload Too Large"
        );
        let head = format!(
            "GET /admin/health HTTP/1.1\r\nX: {}\r\n\r\n",
            "a".repeat(17 * 1024)
        );
        assert_eq!(
            status_line(&state, &head),
            "HTTP/1.1 431 Request Header Fields Too Large"
        );
    }

    #[test]
    fn log_names_cannot_escape_the_session_directory() {
        let state = state();
        assert_eq!(get(&state, "/admin/log/..%2Fsecret").code, 404);
        assert_eq!(get(&state, "/admin/log/a%5Cb").code, 400);
        assert_eq!(get(&state, "/admin/log/.hidden").code, 400);
    }

    #[test]
    fn daemon_does_not_serve_the_frontend() {
        assert_eq!(get(&state(), "/").code, 404);
        assert_eq!(get(&state(), "/index.html").code, 404);
        assert_eq!(route(&state(), "POST", "/").code, 404);
    }

    #[test]
    fn the_route_table_and_the_openapi_document_name_the_same_calls() {
        let doc: serde_json::Value = serde_json::from_str(&openapi_json()).unwrap();
        use std::collections::BTreeSet;
        let documented: BTreeSet<(String, String)> = doc["paths"]
            .as_object()
            .unwrap()
            .iter()
            .flat_map(|(path, methods)| {
                methods
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(move |method| (method.to_uppercase(), path.clone()))
            })
            .collect();
        let routed: BTreeSet<(String, String)> = ROUTES
            .iter()
            .map(|route| {
                let method = match route.method {
                    Method::Get => "GET",
                    Method::Post => "POST",
                };
                (method.to_owned(), format!("/admin/{}", route.path))
            })
            .collect();
        let undocumented: Vec<_> = routed.difference(&documented).collect();
        let unrouted: Vec<_> = documented.difference(&routed).collect();
        assert!(
            undocumented.is_empty(),
            "routes without an OpenAPI operation: {undocumented:?}"
        );
        assert!(
            unrouted.is_empty(),
            "OpenAPI operations without a route: {unrouted:?}"
        );
        assert_eq!(routed.len(), ROUTES.len(), "a route listed twice");
    }

    /// The published contract of the admin API, from which the frontend's client is generated;
    /// a change here is a change for every caller.
    #[test]
    fn the_api_document_is_stable() {
        let document: serde_json::Value = serde_json::from_str(&openapi_json()).unwrap();
        insta::assert_json_snapshot!(document);
    }

    #[test]
    fn the_programme_is_read_changed_and_validated() {
        let state = state();
        let body = |reply: Reply| String::from_utf8_lossy(&reply.body).into_owned();
        assert_eq!(get(&state, "/admin/programme").code, 404, "none saved yet");
        assert_eq!(get(&state, "/admin/shows").code, 404);
        assert_eq!(
            route(&state, "POST", "/admin/programme").code,
            200,
            "the defaults"
        );
        let got = body(get(&state, "/admin/programme"));
        assert!(got.contains("\"zone\":\"America/New_York\""), "{got}");
        assert!(got.contains("\"primetime_at\":[\"20:00\"]"), "{got}");

        let changed = route(
            &state,
            "POST",
            "/admin/programme?zone=America/Chicago&primetime_at=22:00,19:00",
        );
        assert_eq!(changed.code, 200);
        let saved = lock(&state.store).storage().programme().unwrap().unwrap();
        assert_eq!(saved.zone.name(), "America/Chicago");
        assert_eq!(saved.primetime_at.to_string(), "19:00,22:00");
        assert_eq!(
            saved.extended_play,
            crate::programme::EP_EPISODE,
            "left out, kept"
        );

        for bad in [
            "zone=Mars/Olympus",
            "primetime_at=7pm",
            "primetime_at=19:00,19:00",
            "primetime_at=19:00,19:30",
            "primetime_at=19:00,20:15",
            "primetime_minutes=5",
            "extended_play_minutes=x",
            "extended_play_minutes=45",
            "min_extended_play_minutes=200",
        ] {
            let reply = route(&state, "POST", &format!("/admin/programme?{bad}"));
            assert_eq!(reply.code, 400, "{bad}");
        }
        let kept = lock(&state.store).storage().programme().unwrap().unwrap();
        assert_eq!(kept, saved, "a refused change changes nothing");
    }

    /// A bank file whose `otdb:11` has exactly what one Primetime round asks (2, 3, 5, 4, 7 at
    /// difficulties 1-5), and whose `otdb:9` has a single question at difficulty 1.
    fn primetime_bank() -> String {
        let mut questions = Vec::new();
        let mut id = 1000;
        for (difficulty, needs) in crate::questions::primetime_round().iter().enumerate() {
            for _ in 0..*needs {
                id += 1;
                questions.push(format!(
                    r#"{{"id":{id},"difficulty":{},"text":"Film {id}","choices":["A","B","C"],
                        "correct":1,"categories":["otdb:11"],"source":"otdb"}}"#,
                    difficulty + 1
                ));
            }
        }
        questions.push(
            r#"{"id":1,"difficulty":1,"text":"General","choices":["A","B","C"],"correct":1,
                "categories":["otdb:9"],"source":"otdb"}"#
                .to_owned(),
        );
        format!(
            r#"{{"version":1,"sources":[{{"id":"otdb","name":"Open Trivia Database"}}],
                "categories":[{{"id":"otdb:9","name":"General Knowledge","source":"otdb"}},
                              {{"id":"otdb:11","name":"Film","source":"otdb"}}],
                "questions":[{}]}}"#,
            questions.join(",")
        )
    }

    fn import_bank(state: &App, bank: &str) {
        let reply = route_with_body(
            state,
            "POST",
            "/admin/questions/import?mode=replace",
            bank.as_bytes(),
        );
        assert_eq!(reply.code, 200, "{}", body(&reply));
    }

    #[test]
    fn specials_replace_a_days_primetime_show_in_the_plan() {
        let state = programmed();
        import_bank(&state, &primetime_bank());
        let body = |reply: Reply| String::from_utf8_lossy(&reply.body).into_owned();
        let today = {
            let programme = crate::programme::Programme::default();
            programme.zone.date_of(Utc::from(SystemTime::now()))
        };
        let tomorrow = today.next_day().unwrap();
        let put = route(
            &state,
            "POST",
            &format!("/admin/specials/{tomorrow}/20:00?title=Movie%20Night&category=otdb:11"),
        );
        let code = put.code;
        assert_eq!(code, 200, "{}", body(put));
        let specials = body(get(&state, "/admin/specials"));
        assert!(specials.contains("\"title\":\"Movie Night\""), "{specials}");
        assert!(
            specials.contains("\"minutes\":60"),
            "the Primetime length: {specials}"
        );

        let plan = body(get(&state, "/admin/programme/plan?days=3"));
        assert_eq!(plan.matches("\"kind\":\"special\"").count(), 1, "{plan}");
        assert!(plan.contains("\"category\":\"otdb:11\""), "{plan}");
        assert!(plan.contains("\"kind\":\"extended_play\""), "{plan}");
        assert!(
            plan.contains(&format!("\"start_local\":\"{tomorrow} 20:00\"")),
            "{plan}"
        );

        for bad in [
            format!("/admin/specials/{tomorrow}/20:00?title=&category=otdb:11"),
            format!("/admin/specials/{tomorrow}/20:00?title=x"),
            "/admin/specials/2026-02-30/20:00?title=x&category=otdb:11".to_owned(),
            format!("/admin/specials/{tomorrow}/20:00?title=x&category=otdb:11&minutes=999"),
        ] {
            assert_eq!(route(&state, "POST", &bad).code, 400, "{bad}");
        }
        let short = route(
            &state,
            "POST",
            &format!("/admin/specials/{tomorrow}/20:00?title=x&category=otdb:9"),
        );
        assert_eq!(short.code, 400);
        assert_eq!(
            error_of(&short),
            "category \"otdb:9\" has 1 question(s) at difficulty 1, a Primetime round asks 2"
        );
        let categories = body(get(&state, "/admin/categories"));
        assert!(categories.contains(r#""id":"otdb:9","name":"General Knowledge","sources":["otdb"],"counts":[1,0,0,0,0],"primetime":false"#), "{categories}");
        assert!(
            categories.contains(r#""counts":[2,3,5,4,7],"primetime":true"#),
            "{categories}"
        );
        assert_eq!(get(&state, "/admin/programme/plan?days=30").code, 400);

        let path = format!("/admin/specials/{tomorrow}/20:00/delete");
        assert_eq!(route(&state, "POST", &path).code, 200);
        assert_eq!(route(&state, "POST", &path).code, 404);
        let plan = body(get(&state, "/admin/programme/plan?days=3"));
        assert!(!plan.contains("\"kind\":\"special\""), "{plan}");
    }

    #[test]
    fn a_special_without_a_length_needs_a_programme() {
        let state = state();
        import_bank(&state, &primetime_bank());
        let tomorrow = {
            let programme = crate::programme::Programme::default();
            programme
                .zone
                .date_of(Utc::from(SystemTime::now()))
                .next_day()
                .unwrap()
        };
        let reply = route(
            &state,
            "POST",
            &format!("/admin/specials/{tomorrow}/20:00?title=x&category=otdb:11"),
        );
        assert_eq!(reply.code, 400, "{}", error_of(&reply));
        assert_eq!(error_of(&reply), "no programme: shows are not scheduled");
        assert_eq!(
            body(&get(&state, "/admin/specials")),
            "[]",
            "nothing was saved"
        );
    }

    #[test]
    fn the_materialised_shows_are_listed_and_one_can_be_started_by_hand() {
        let state = programmed();
        let body = |reply: Reply| String::from_utf8_lossy(&reply.body).into_owned();
        let shows = body(get(&state, "/admin/shows"));
        assert_eq!(shows.matches("\"state\":\"running\"").count(), 1, "{shows}");
        assert_eq!(shows.matches("\"state\":\"listed\"").count(), 1, "{shows}");
        assert!(shows.contains("\"state\":\"planned\""), "{shows}");
        assert!(shows.contains("\"session_id\":\""), "{shows}");

        let started = body(route(&state, "POST", "/admin/shows/now?mode=ep&minutes=20"));
        assert!(started.contains("\"manual\":true"), "{started}");
        let running = lock(&state.schedule).current_session_id(SystemTime::now());
        assert!(
            started.contains(&format!("\"session_id\":\"{running}\"")),
            "the hand-started show is the one served: {started}"
        );
        for bad in ["mode=xx", "minutes=0", "minutes=x"] {
            let target = format!("/admin/shows/now?{bad}");
            assert_eq!(route(&state, "POST", &target).code, 400, "{bad}");
        }
        assert_eq!(
            route(&self::state(), "POST", "/admin/shows/now").code,
            400,
            "no programme"
        );
    }

    #[test]
    fn schedule_storage_failures_are_500_and_bad_input_is_400() {
        let (state, dir) = sqlite_state("schedule-statuses");
        let path = dir.join("onevs100.db");
        let tomorrow = {
            let programme = crate::programme::Programme::default();
            programme
                .zone
                .date_of(Utc::from(SystemTime::now()))
                .next_day()
                .unwrap()
        };
        assert_eq!(route(&state, "POST", "/admin/programme").code, 200);
        import_bank(&state, &primetime_bank());

        // Input that does not pass the checks is a 400 before storage is written.
        for target in [
            "/admin/programme?primetime_minutes=5".to_owned(),
            "/admin/shows/now?minutes=0".to_owned(),
            format!("/admin/specials/{tomorrow}/20:00?title=x&category=otdb:11&minutes=999"),
            "/admin/specials/2026-02-30/20:00/delete".to_owned(),
        ] {
            let reply = route(&state, "POST", &target);
            assert_eq!(reply.code, 400, "{target}: {}", error_of(&reply));
        }

        // Without the schedule's tables every write it makes fails: a 500, not a 400.
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("DROP TABLE programme; DROP TABLE special_episodes; DROP TABLE shows;")
            .unwrap();
        for target in [
            "/admin/programme".to_owned(),
            "/admin/shows/now".to_owned(),
            format!("/admin/specials/{tomorrow}/20:00?title=x&category=otdb:11"),
            format!("/admin/specials/{tomorrow}/20:00/delete"),
        ] {
            let reply = route(&state, "POST", &target);
            assert_eq!(reply.code, 500, "{target}: {}", error_of(&reply));
            assert!(
                error_of(&reply).starts_with("storage:"),
                "{target}: {}",
                error_of(&reply)
            );
        }
        drop(state);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_clears_volatile_state_and_keeps_persistent_data_and_schedule() {
        let state = state();
        let game_state =
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.GameState";
        let storage = {
            let mut store = lock(&state.store);
            assert!(store.set(game_state, b"<state/>".to_vec()));
            store.storage()
        };
        storage
            .touch_player(
                Xuid(0xE000_0601_484D_4B37),
                Some("Player1"),
                crate::time::Utc::from_unix_secs(1),
            )
            .unwrap();
        storage.counter_add("global.test", 2.0).unwrap();
        assert_eq!(route(&state, "POST", "/admin/programme").code, 200);

        let reset = route(&state, "POST", "/admin/reset");

        assert_eq!(reset.code, 200);
        assert!(lock(&state.store).get(game_state).is_err());
        assert_eq!(storage.players().unwrap().len(), 1);
        assert_eq!(storage.counter("global.test").unwrap(), Some(2.0));
        assert!(
            lock(&state.schedule).is_enabled(),
            "the programme is configuration"
        );
        assert!(storage.programme().unwrap().is_some());
        assert_eq!(get(&state, "/admin/reset").code, 405);
    }
}
