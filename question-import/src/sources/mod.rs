//! The sources the tool can import from, by id.

pub mod otdb;

use crate::source::QuestionSource;

/// The ids `--source` takes.
pub const IDS: &[&str] = &["otdb"];

/// The source with `id`, talking to the real service.
pub fn by_id(id: &str) -> Option<Box<dyn QuestionSource>> {
    match id {
        "otdb" => Some(Box::new(otdb::OpenTriviaDb::new(
            Box::new(otdb::UreqHttp::new()),
            otdb::PACING,
        ))),
        _ => None,
    }
}
