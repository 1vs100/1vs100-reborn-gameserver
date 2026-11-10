//! The consoles' saved error reports (`Log.UserError`).

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{ErrorResponse, Query, Reply, query_as};
use crate::{app::App, sbg::user_error, storage::SavedErrorReport, sync::lock, time::Utc};

/// Reports listed unless the request asks for another number.
const DEFAULT_REPORTS: usize = 200;
/// The most reports one request may ask for.
const MAX_REPORTS: usize = 5000;

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ErrorReportsParams {
    /// Maximum reports to return, newest first (default 200, capped at 5000).
    limit: Option<usize>,
}

/// One saved error report.
#[derive(Serialize, ToSchema)]
pub struct ErrorReportInfo {
    /// Later reports have larger ids.
    id: i64,
    received_unix_ms: i64,
    /// The address of the console that posted it.
    console: String,
    key: String,
    /// The report's fields; absent for a report in an unknown format.
    fields: Option<ErrorReportFields>,
    /// The posted document. Bytes that are not UTF-8 show as replacement characters.
    document: String,
}

/// What the title wrote in a report of the known format.
#[derive(Serialize, ToSchema)]
pub struct ErrorReportFields {
    name: String,
    reason: String,
    /// `No` or `None` for a report, `Fatal`, `Exit` or `Crash` when the title gives up.
    fatal: String,
    is_fatal: bool,
    /// The console's clock when it wrote the report; absent when the report has no `UTC` time.
    console_utc_unix_ms: Option<i64>,
    /// The console's clock less the server's when the report arrived; the time the report took
    /// to arrive makes a console look that much behind.
    clock_offset_ms: Option<i64>,
    state: String,
    substate: String,
    round: String,
    question: String,
    /// The SBG key the error is about (`NO_MSG` when none).
    about_key: String,
}

#[derive(Serialize, ToSchema)]
pub struct ErrorReportsClearResponse {
    removed: usize,
}

fn info(saved: SavedErrorReport) -> ErrorReportInfo {
    let report = saved.report;
    let text = user_error::text(&report.document);
    let fields = text.as_deref().and_then(user_error::parse).map(|parsed| {
        let console_utc = parsed.utc.and_then(Utc::parse);
        ErrorReportFields {
            name: parsed.name.into(),
            reason: parsed.reason.into(),
            fatal: parsed.fatal.into(),
            is_fatal: parsed.is_fatal(),
            console_utc_unix_ms: console_utc.map(Utc::unix_ms),
            clock_offset_ms: console_utc.map(|at| at.unix_ms() - report.received.unix_ms()),
            state: parsed.state.into(),
            substate: parsed.substate.into(),
            round: parsed.round.into(),
            question: parsed.question.into(),
            about_key: parsed.key.into(),
        }
    });
    ErrorReportInfo {
        id: saved.id.0,
        received_unix_ms: report.received.unix_ms(),
        console: report.console.to_string(),
        key: report.key,
        fields,
        document: String::from_utf8_lossy(&report.document).into_owned(),
    }
}

/// The consoles' error reports, newest first
#[utoipa::path(get, path = "/admin/errors", params(ErrorReportsParams), responses((status = 200, description = "The newest `limit` saved error reports, newest first", body = Vec<ErrorReportInfo>), (status = 400, description = "The limit is not a number", body = ErrorResponse), (status = 500, description = "Storage failure", body = ErrorResponse)))]
pub(super) fn error_reports(state: &App, query: &Query) -> Reply {
    let limit = match query_as::<ErrorReportsParams>(query) {
        Ok(params) => params.limit.unwrap_or(DEFAULT_REPORTS).min(MAX_REPORTS),
        Err(reply) => return reply,
    };
    let storage = lock(&state.store).storage();
    match storage.error_reports(limit) {
        Ok(reports) => Reply::json(&reports.into_iter().map(info).collect::<Vec<_>>()),
        Err(error) => Reply::error(500, error.to_string()),
    }
}

/// Remove every saved error report
#[utoipa::path(post, path = "/admin/errors/clear", responses((status = 200, description = "How many reports were removed", body = ErrorReportsClearResponse), (status = 500, description = "Storage failure", body = ErrorResponse)))]
pub(super) fn error_reports_clear(state: &App) -> Reply {
    let storage = lock(&state.store).storage();
    match storage.clear_error_reports() {
        Ok(removed) => {
            tracing::info!(removed, "admin cleared the error reports");
            Reply::json(&ErrorReportsClearResponse { removed })
        }
        Err(error) => Reply::error(500, error.to_string()),
    }
}
