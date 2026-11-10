//! The show's store: its keys, values and GameStates.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{ErrorResponse, Query, Reply, query_as};
use crate::{app::App, ids::Round, sync::lock, time::Utc, timeline};

/// What the consoles' GameState documents say, as the Overview shows it.
#[derive(Serialize, ToSchema)]
pub struct GameStatesInfo {
    /// The states in the order a round passes through them (`GameStateName::ROUND_ORDER`).
    phases: Vec<&'static str>,
    /// Every stored GameState, by key.
    states: Vec<GameStateInfo>,
}

#[derive(Serialize, ToSchema)]
pub struct GameStateInfo {
    key: String,
    /// The session (show) id the key names.
    session: String,
    /// The state's `name`; `null` if the document cannot be read.
    name: Option<String>,
    /// Whether `name` is one of the title's states.
    known: bool,
    #[schema(value_type = Option<u32>)]
    round: Option<Round>,
    // The console's `QuestionNumber`: 0 before the round's first question.
    question: Option<u32>,
    start_utc: Option<String>,
    end_utc: Option<String>,
    /// The document as stored.
    xml: String,
}

#[derive(Serialize, ToSchema)]
pub struct KeyInfo {
    key: String,
    len: usize,
}

#[derive(Serialize, ToSchema)]
pub struct StoreValue {
    key: String,
    text: Option<String>,
    hex: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ClearResponse {
    removed: bool,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListKeysParams {
    /// Only return keys starting with this prefix.
    prefix: Option<String>,
}

/// Every stored GameState, read: its state, round, question and window
#[utoipa::path(get, path = "/admin/gamestates", responses((status = 200, description = "The round's state order and every stored GameState", body = GameStatesInfo)))]
pub(super) fn game_states(state: &App) -> Reply {
    use crate::{
        gamestate::{GameStateDoc, GameStateName},
        sbg::key::{Key, ParsedKey},
    };
    let store = lock(&state.store);
    let states: Vec<_> = store
        .keys()
        .into_iter()
        .filter_map(|(key, _)| {
            let Key::GameState { session } = ParsedKey::parse(key).kind else {
                return None;
            };
            let value = store.get(key).ok()?;
            let doc = GameStateDoc::parse(value);
            let name = doc.as_ref().map(|doc| doc.name().into_owned());
            Some(GameStateInfo {
                key: key.to_owned(),
                session: session.as_str().to_owned(),
                known: name.as_deref().and_then(GameStateName::parse).is_some(),
                name,
                round: doc.as_ref().and_then(GameStateDoc::round),
                question: doc.as_ref().and_then(|doc| doc.reported_question()),
                start_utc: doc.as_ref().and_then(GameStateDoc::start).map(Utc::to_grid),
                end_utc: doc.as_ref().and_then(GameStateDoc::end).map(Utc::to_grid),
                xml: String::from_utf8_lossy(value).into_owned(),
            })
        })
        .collect();
    Reply::json(&GameStatesInfo {
        phases: GameStateName::ROUND_ORDER
            .iter()
            .map(|state| state.as_str())
            .collect(),
        states,
    })
}

/// All stored keys and their byte length
#[utoipa::path(get, path = "/admin/store/keys", params(ListKeysParams), responses((status = 200, description = "All stored keys and their byte length", body = Vec<KeyInfo>)))]
pub(super) fn list_keys(state: &App, query: &Query) -> Reply {
    let prefix = match query_as::<ListKeysParams>(query) {
        Ok(params) => params.prefix.unwrap_or_default(),
        Err(reply) => return reply,
    };
    let store = lock(&state.store);
    let keys: Vec<_> = store
        .keys()
        .into_iter()
        .filter(|(key, _)| key.starts_with(prefix.as_str()))
        .map(|(key, len)| KeyInfo {
            key: key.to_owned(),
            len,
        })
        .collect();
    Reply::json(&keys)
}

/// One stored value
#[utoipa::path(
    get,
    path = "/admin/store/{key}",
    params(("key" = String, Path, description = "The full SBG key, e.g. v3.Session.MGS.1vs100.Store.<program>.GameState")),
    responses(
        (status = 200, description = "Value found - text if valid UTF-8, hex otherwise", body = StoreValue),
        (status = 404, description = "No such key", body = ErrorResponse),
    )
)]
pub(super) fn get_key(state: &App, key: &str) -> Reply {
    let store = lock(&state.store);
    match store.get(key) {
        Ok(bytes) => {
            let (text, hex) = match std::str::from_utf8(bytes) {
                Ok(text) => (Some(text.to_owned()), None),
                Err(_) => (
                    None,
                    Some(bytes.iter().map(|b| format!("{b:02x}")).collect()),
                ),
            };
            Reply::json(&StoreValue {
                key: key.to_owned(),
                text,
                hex,
            })
        }
        Err(_) => Reply::status(404),
    }
}

/// Remove one stored key
#[utoipa::path(
    post,
    path = "/admin/store/{key}/clear",
    params(("key" = String, Path, description = "The full SBG key to remove")),
    responses((status = 200, description = "Whether the key existed and was removed", body = ClearResponse))
)]
pub(super) fn clear_key(state: &App, key: &str) -> Reply {
    let removed = lock(&state.store).remove(key);
    tracing::info!(key, removed, "admin cleared store key");
    timeline::record(
        "admin",
        format!("cleared key {key} (removed={removed})"),
        None,
    );
    Reply::json(&ClearResponse { removed })
}
