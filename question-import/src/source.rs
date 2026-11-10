//! Where questions come from, behind one trait. A source only fetches: paging, rate limits,
//! tokens and encodings stay inside it; everything that makes a question fit the title is shared
//! (`fit.rs`).

use std::fmt;

use onevs100_gameserver::questions::BankError;

use crate::fit;

/// A place questions come from.
pub trait QuestionSource {
    /// Stable short name ("otdb"): namespaces the source's categories and question ids.
    fn id(&self) -> &'static str;
    /// Who to credit, and under which licence (shown in the admin Questions page).
    fn attribution(&self) -> Attribution;
    fn categories(&mut self) -> Result<Vec<SourceCategory>, SourceError>;
    /// Every question of `category`, handed to `found` as it arrives; the report counts what the
    /// source skipped and why.
    fn fetch(
        &mut self,
        category: &SourceCategory,
        found: &mut dyn FnMut(SourceQuestion),
    ) -> Result<FetchReport, SourceError>;
    /// Every question of every one of `categories`, handed to `found` with the index of its
    /// category. By default one [`fetch`](Self::fetch) per category; a source that can pull all
    /// categories in one go (fewer calls under a rate limit) overrides it.
    fn fetch_all(
        &mut self,
        categories: &[SourceCategory],
        found: &mut dyn FnMut(usize, SourceQuestion),
    ) -> Result<StreamReport, SourceError> {
        fetch_each(self, categories, found)
    }
}

/// [`QuestionSource::fetch`] for each of `categories` in turn.
pub fn fetch_each<S: QuestionSource + ?Sized>(
    source: &mut S,
    categories: &[SourceCategory],
    found: &mut dyn FnMut(usize, SourceQuestion),
) -> Result<StreamReport, SourceError> {
    let mut report = StreamReport::default();
    for (index, category) in categories.iter().enumerate() {
        let fetched = source.fetch(category, &mut |question| found(index, question))?;
        report.by_category.push(fetched);
    }
    Ok(report)
}

/// What [`QuestionSource::fetch_all`] left out: per category (in the order asked), and what it
/// could not place in any of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamReport {
    pub by_category: Vec<FetchReport>,
    pub unsorted: FetchReport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribution {
    pub name: String,
    pub licence: String,
    pub url: String,
}

/// A category as the source names it; `id` is the source's own (`9`), without the source prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCategory {
    pub id: String,
    pub name: String,
    /// Which of our categories (`categories.rs`) it is, as the source maps it; `None`: not mapped
    /// yet, so it gets a category of its own (`otdb-33`).
    pub ours: Option<&'static str>,
}

/// A question as a source has it, before it is fitted to the title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceQuestion {
    /// The source's own id for it, if it has one (OpenTriviaDB has none).
    pub id: Option<String>,
    pub text: String,
    pub correct: String,
    pub incorrect: Vec<String>,
    pub difficulty: SourceDifficulty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDifficulty {
    Easy,
    Medium,
    Hard,
    /// A source that grades 1 to 5, as the title does.
    Title(u32),
}

/// Why a question is not in the bank: the source's own reason, or the fitting's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The source's result is not a multiple choice question.
    NotMultipleChoice,
    /// A difficulty that is neither the source's easy, medium or hard, nor the title's 1 to 5.
    UnknownDifficulty,
    /// A result of a category the import did not ask for.
    UnlistedCategory,
    /// The fitting left it out (`fit.rs`).
    Fit(fit::Skip),
}

impl SkipReason {
    /// How the report prints it.
    pub fn describe(self) -> &'static str {
        match self {
            SkipReason::NotMultipleChoice => "not multiple choice",
            SkipReason::UnknownDifficulty => "unknown difficulty",
            SkipReason::UnlistedCategory => "category not in the category list",
            SkipReason::Fit(skip) => skip.describe(),
        }
    }
}

/// What a source left out of a category, counted per reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchReport {
    /// One entry per reason, in the order the reasons were first seen (the report's order).
    pub skipped: Vec<(SkipReason, usize)>,
}

impl FetchReport {
    pub fn skip(&mut self, reason: SkipReason) {
        match self.skipped.iter_mut().find(|(known, _)| *known == reason) {
            Some((_, count)) => *count += 1,
            None => self.skipped.push((reason, 1)),
        }
    }
}

/// Why an import could not go on.
#[derive(Debug)]
pub enum SourceError {
    /// The HTTP call failed, or its body could not be read.
    Http { url: String, detail: String },
    /// The body is not the JSON the API documents.
    Json {
        url: String,
        error: serde_json::Error,
    },
    /// An `encode=base64` field does not decode.
    Base64 { error: base64::DecodeError },
    /// A decoded field is not UTF-8.
    Utf8 { error: std::string::FromUtf8Error },
    /// The API answered `scope` with a response code the import does not handle; the number
    /// itself, because this error type is shared by every source.
    Response { scope: String, code: u32 },
    /// The API gave no session token, answering with `code`.
    Token { code: u32 },
    /// `source` does not list `category`.
    UnknownCategory {
        source: &'static str,
        category: String,
    },
    /// The bank the import built does not pass the server's checks.
    Bank(BankError),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Http { url, detail } => write!(f, "GET {url}: {detail}"),
            SourceError::Json { url, error } => write!(f, "{url}: {error}"),
            SourceError::Base64 { error } => write!(f, "not Base64: {error}"),
            SourceError::Utf8 { error } => write!(f, "not UTF-8: {error}"),
            SourceError::Response { scope, code } => {
                write!(f, "{scope}: response code {code}")
            }
            SourceError::Token { code } => write!(f, "no session token (code {code})"),
            SourceError::UnknownCategory { source, category } => {
                write!(f, "{source} has no category {category}")
            }
            SourceError::Bank(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SourceError::Json { error, .. } => Some(error),
            SourceError::Bank(error) => Some(error),
            SourceError::Base64 { error } => Some(error),
            SourceError::Utf8 { error } => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit::Skip;

    #[test]
    fn skips_are_counted_per_reason_in_the_order_first_seen() {
        let mut report = FetchReport::default();
        report.skip(SkipReason::NotMultipleChoice);
        report.skip(SkipReason::Fit(Skip::Duplicate));
        report.skip(SkipReason::NotMultipleChoice);
        assert_eq!(
            report.skipped,
            [
                (SkipReason::NotMultipleChoice, 2),
                (SkipReason::Fit(Skip::Duplicate), 1)
            ]
        );
    }
}
