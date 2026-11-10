//! The bank file the server's `POST /admin/questions/import` takes, and the import that fills it:
//! every category of a source, fitted (`fit.rs`), with the source's attribution and a report.

use onevs100_gameserver::questions::{BANK_FILE_VERSION, BankFile, Category, QuestionBank, Source};

use crate::{
    fit::{Fitter, Limits},
    source::{FetchReport, QuestionSource, SkipReason, SourceError, SourceQuestion, fetch_each},
};

/// What an import made, and what it left out.
pub struct Import {
    /// Validated as the server validates it, so the server takes it.
    pub bank: QuestionBank,
    pub report: Report,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub categories: Vec<CategoryReport>,
    /// Left out before a category was known (a result of a category not in the list).
    pub unsorted: Vec<(SkipReason, usize)>,
}

/// One category of the source, and the one of ours it filled.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CategoryReport {
    /// Ours (`vehicles`), or one of its own (`otdb-33`) when the source does not map it.
    pub id: String,
    pub name: String,
    /// The source's (`28`).
    pub source_id: String,
    /// The source maps it onto none of ours (`categories.rs`), so it got one of its own.
    pub unmapped: bool,
    /// Questions the source handed over.
    pub fetched: usize,
    /// Kept as new questions of this category.
    pub kept: usize,
    /// Left out, by the source or the fitting, counted per reason.
    pub skipped: Vec<(SkipReason, usize)>,
}

/// Import every category of `source` (or those whose source id is in `only`). `progress` hears
/// what is being fetched.
pub fn import(
    source: &mut dyn QuestionSource,
    only: Option<&[String]>,
    limits: Limits,
    progress: &mut dyn FnMut(&str),
) -> Result<Import, SourceError> {
    let source_id = source.id();
    let attribution = source.attribution();
    let mut categories = source.categories()?;
    if let Some(only) = only {
        if let Some(unknown) = only
            .iter()
            .find(|id| !categories.iter().any(|c| &c.id == *id))
        {
            return Err(SourceError::UnknownCategory {
                source: source_id,
                category: unknown.clone(),
            });
        }
        categories.retain(|category| only.contains(&category.id));
    }
    let mut fitter = Fitter::new(source_id, limits);
    let mut entries: Vec<CategoryReport> = categories
        .iter()
        .map(|category| {
            let (id, name) = match category.ours {
                Some(ours) => (
                    ours.to_owned(),
                    crate::categories::name(ours)
                        .unwrap_or(&category.name)
                        .to_owned(),
                ),
                None => (
                    format!("{source_id}-{}", category.id),
                    category.name.clone(),
                ),
            };
            CategoryReport {
                id,
                name,
                source_id: category.id.clone(),
                unmapped: category.ours.is_none(),
                ..CategoryReport::default()
            }
        })
        .collect();
    // Ours, each once (two of the source's may map onto one), in the source's order.
    let mut ours: Vec<Category> = Vec::new();
    for entry in &entries {
        if !ours.iter().any(|category| category.id == entry.id) {
            ours.push(Category {
                id: entry.id.clone(),
                name: entry.name.clone(),
            });
        }
    }
    let mut not_fitted = vec![FetchReport::default(); categories.len()];
    let mut found = |index: usize, question: SourceQuestion| {
        let entry = &mut entries[index];
        entry.fetched += 1;
        match fitter.add(&entry.id, &entry.source_id, &question) {
            Ok(_) => entry.kept += 1,
            Err(skip) => not_fitted[index].skip(SkipReason::Fit(skip)),
        }
    };
    // A few categories one by one; all of them in one go, where the source can (fewer calls).
    let fetched = if only.is_some() {
        progress(&format!(
            "fetching {} categories one by one",
            categories.len()
        ));
        fetch_each(source, &categories, &mut found)?
    } else {
        progress(&format!("fetching all {} categories", categories.len()));
        source.fetch_all(&categories, &mut found)?
    };
    let mut report = Report {
        categories: entries,
        unsorted: fetched.unsorted.skipped,
    };
    for ((entry, by_source), by_fitting) in report
        .categories
        .iter_mut()
        .zip(fetched.by_category)
        .zip(not_fitted)
    {
        entry.skipped = by_source.skipped;
        entry.skipped.extend(by_fitting.skipped);
    }
    let file = BankFile {
        version: BANK_FILE_VERSION,
        sources: vec![Source {
            id: source_id.to_owned(),
            name: attribution.name,
            licence: attribution.licence,
            url: attribution.url,
        }],
        categories: ours,
        questions: fitter.into_questions(),
    };
    let bank = QuestionBank::from_file(file).map_err(SourceError::Bank)?;
    Ok(Import { bank, report })
}

impl Report {
    /// One line per category: kept of fetched, the counts per difficulty, whether a special can
    /// be of it (one Primetime round), and what was left out.
    pub fn render(&self, bank: &QuestionBank) -> String {
        let mut out = String::new();
        for category in &self.categories {
            let counts = bank.counts(&category.id);
            let primetime = match bank.primetime_shortfall(&category.id) {
                None => "enough for a special".to_owned(),
                Some(shortfall) => format!("not enough for a special: {shortfall}"),
            };
            let unmapped = if category.unmapped {
                " (not mapped onto one of ours: categories.rs)"
            } else {
                ""
            };
            out.push_str(&format!(
                "{} {} (source {}){unmapped}: kept {} of {}, by difficulty {:?}, {primetime}\n",
                category.id,
                category.name,
                category.source_id,
                category.kept,
                category.fetched,
                counts
            ));
            for (reason, count) in &category.skipped {
                out.push_str(&format!("    skipped {count}: {}\n", reason.describe()));
            }
        }
        for (reason, count) in &self.unsorted {
            out.push_str(&format!("skipped {count}: {}\n", reason.describe()));
        }
        out.push_str(&format!(
            "{} questions in {} categories\n",
            bank.questions().len(),
            bank.categories().len()
        ));
        out
    }
}

/// The bank file as the server's import takes it.
pub fn to_json(bank: &QuestionBank) -> String {
    serde_json::to_string_pretty(&bank.to_file()).expect("a bank file serialises")
}
