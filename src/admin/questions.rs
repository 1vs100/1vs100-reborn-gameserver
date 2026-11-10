//! The question bank: import, export, categories and questions.

use crate::schedule::Mode;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{ErrorResponse, ImportParams, Query, Reply, import_mode, query_as};
use crate::{
    app::App,
    ids::QuestionId,
    questions::{Choice, Difficulty, QuestionBank},
    sync::lock,
};

const DEFAULT_QUESTIONS: usize = 100;
const MAX_QUESTIONS: usize = 1000;

#[derive(Serialize, ToSchema)]
pub struct QuestionsImportResponse {
    /// Questions in the imported file.
    imported: usize,
    /// Questions saved and served.
    questions: usize,
}

#[derive(Serialize, ToSchema)]
pub struct CategoryInfo {
    /// Ours (`music`); every source's questions of it count.
    id: String,
    name: String,
    /// The sources its questions come from, in bank order.
    sources: Vec<String>,
    /// Questions at difficulty 1 to 5.
    counts: Vec<usize>,
    /// Whether it has as many at each difficulty as one Primetime round asks (2, 3, 5, 4, 7):
    /// only then can a Primetime special be of this category.
    primetime: bool,
    /// The same for one Extended Play episode, for an Extended Play special.
    extended_play: bool,
}

#[derive(Serialize, ToSchema)]
pub struct QuestionInfo {
    #[schema(value_type = u32)]
    id: QuestionId,
    locale: String,
    /// 1 (very easy) to 5 (very hard).
    #[schema(value_type = u32)]
    difficulty: Difficulty,
    text: String,
    choices: Vec<String>,
    /// The right choice, 1 to 3.
    #[schema(value_type = u8)]
    correct: Choice,
    categories: Vec<String>,
    /// The source's id; its name, licence and URL are null while the bank does not list it.
    source: String,
    source_name: Option<String>,
    licence: Option<String>,
    url: Option<String>,
    /// The source's own category and id, where the source has them.
    origin_category: Option<String>,
    origin_id: Option<String>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct QuestionsParams {
    /// Only this category (`music`).
    category: Option<String>,
    /// Only this difficulty, 1-5.
    difficulty: Option<u32>,
    /// At most this many (default 100, capped at 1000).
    limit: Option<usize>,
}

/// Serve the question bank saved in storage (the built-in one while none is).
pub(super) fn serve_saved_questions(state: &App) -> Result<(), Reply> {
    let storage = lock(&state.store).storage();
    let bank = storage.question_bank().map_err(|error| {
        Reply::error(
            500,
            format!("imported, but the question bank not reloaded: {error}"),
        )
    })?;
    lock(&state.store).set_questions(Arc::new(QuestionBank::in_effect(bank)));
    Ok(())
}

/// Import a question bank file
#[utoipa::path(post, path = "/admin/questions/import", params(ImportParams), request_body(content = Object, content_type = "application/json", description = "A bank file: {version, sources, categories, questions}"), responses((status = 200, description = "How many questions were imported and how many are saved now; served at once", body = QuestionsImportResponse), (status = 400, description = "Invalid bank file or mode; nothing was changed", body = ErrorResponse)))]
pub(super) fn questions_import(state: &App, query: &Query, body: &[u8]) -> Reply {
    let mode = match import_mode(query) {
        Ok(mode) => mode,
        Err(reply) => return reply,
    };
    let file: crate::questions::BankFile = match serde_json::from_slice(body) {
        Ok(file) => file,
        Err(error) => return Reply::error(400, format!("not a bank file: {error}")),
    };
    let bank = match QuestionBank::from_file(file) {
        Ok(bank) => bank,
        Err(error) => return Reply::error(400, error.to_string()),
    };
    let storage = lock(&state.store).storage();
    if let Err(error) = storage.import_questions(&bank, mode) {
        return Reply::error(500, error.to_string());
    }
    if let Err(reply) = serve_saved_questions(state) {
        return reply;
    }
    let saved = lock(&state.store).questions().questions().len();
    tracing::info!(
        ?mode,
        imported = bank.questions().len(),
        saved,
        "question import"
    );
    Reply::json(&QuestionsImportResponse {
        imported: bank.questions().len(),
        questions: saved,
    })
}

/// Export the imported question bank
#[utoipa::path(get, path = "/admin/questions/export", responses((status = 200, description = "The imported bank as a bank file; no questions while the built-in bank is served", content_type = "application/json", body = Object)))]
pub(super) fn questions_export(state: &App) -> Reply {
    let storage = lock(&state.store).storage();
    match storage.question_bank() {
        Ok(bank) => Reply::json(&bank.to_file()),
        Err(error) => Reply::error(500, error.to_string()),
    }
}

/// Categories of the served bank, with question counts per difficulty
#[utoipa::path(get, path = "/admin/categories", responses((status = 200, description = "Categories in bank order", body = Vec<CategoryInfo>)))]
pub(super) fn categories_get(state: &App) -> Reply {
    let bank = lock(&state.store).questions();
    let categories: Vec<_> = bank
        .categories()
        .iter()
        .map(|category| CategoryInfo {
            id: category.id.clone(),
            name: category.name.clone(),
            sources: bank.category_sources(&category.id),
            counts: bank.counts(&category.id).to_vec(),
            primetime: bank.primetime_shortfall(&category.id).is_none(),
            extended_play: bank.shortfall(&category.id, Mode::ExtendedPlay).is_none(),
        })
        .collect();
    Reply::json(&categories)
}

/// Questions of the served bank, with their source
#[utoipa::path(get, path = "/admin/questions", params(QuestionsParams), responses((status = 200, description = "Questions in bank order", body = Vec<QuestionInfo>), (status = 400, description = "Invalid difficulty, category or limit", body = ErrorResponse)))]
pub(super) fn questions_get(state: &App, query: &Query) -> Reply {
    let params = match query_as::<QuestionsParams>(query) {
        Ok(params) => params,
        Err(reply) => return reply,
    };
    let difficulty = match params.difficulty {
        Some(number) => match Difficulty::from_number(number) {
            Some(difficulty) => Some(difficulty),
            None => return Reply::error(400, "difficulty must be 1-5"),
        },
        None => None,
    };
    let limit = params.limit.unwrap_or(DEFAULT_QUESTIONS).min(MAX_QUESTIONS);
    let bank = lock(&state.store).questions();
    let category = params.category.as_deref();
    if let Some(category) = category
        && !bank.categories().iter().any(|known| known.id == category)
    {
        return Reply::error(400, format!("unknown category {category:?}"));
    }
    let questions: Vec<_> = bank
        .questions()
        .iter()
        .filter(|question| difficulty.is_none_or(|difficulty| question.difficulty == difficulty))
        .filter(|question| {
            category.is_none_or(|category| question.categories.iter().any(|c| c == category))
        })
        .take(limit)
        .map(|question| {
            let source = bank
                .sources()
                .iter()
                .find(|source| source.id == question.source);
            QuestionInfo {
                id: question.id,
                locale: question.locale.clone(),
                difficulty: question.difficulty,
                text: question.text.clone(),
                choices: question.choices.to_vec(),
                correct: question.correct,
                categories: question.categories.clone(),
                source: question.source.clone(),
                source_name: source.map(|source| source.name.clone()),
                licence: source.map(|source| source.licence.clone()),
                url: source.map(|source| source.url.clone()),
                origin_category: question.origin.category.clone(),
                origin_id: question.origin.id.clone(),
            }
        })
        .collect();
    Reply::json(&questions)
}
