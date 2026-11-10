//! The question bank's tables.

use std::collections::BTreeMap;

use crate::{
    ids::QuestionId,
    questions::{BankFile, Category, Choice, Difficulty, Origin, Question, QuestionBank, Source},
    storage::{Result, StorageError},
};

pub(super) fn load_questions(conn: &rusqlite::Connection) -> Result<QuestionBank> {
    let mut stmt =
        conn.prepare("SELECT id, name, licence, url FROM question_sources ORDER BY position")?;
    let sources = stmt
        .query_map([], |row| {
            Ok(Source {
                id: row.get(0)?,
                name: row.get(1)?,
                licence: row.get(2)?,
                url: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT id, name FROM categories ORDER BY position")?;
    let categories = stmt
        .query_map([], |row| {
            Ok(Category {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut links: BTreeMap<QuestionId, Vec<String>> = BTreeMap::new();
    let mut stmt =
        conn.prepare("SELECT question_id, category FROM question_categories ORDER BY position")?;
    for row in stmt.query_map([], |row| {
        Ok((QuestionId(row.get::<_, u32>(0)?), row.get::<_, String>(1)?))
    })? {
        let (id, category) = row?;
        links.entry(id).or_default().push(category);
    }
    let mut stmt = conn.prepare(
        "SELECT id, locale, difficulty, text, choice1, choice2, choice3, correct, source,
                origin_category, origin_id
         FROM questions ORDER BY position",
    )?;
    let questions = stmt
        .query_map([], |row| {
            Ok(Question {
                id: QuestionId(row.get(0)?),
                locale: row.get(1)?,
                difficulty: difficulty(row, 2)?,
                text: row.get(3)?,
                choices: [row.get(4)?, row.get(5)?, row.get(6)?],
                correct: correct_choice(row, 7)?,
                categories: Vec::new(),
                source: row.get(8)?,
                origin: Origin {
                    category: row.get(9)?,
                    id: row.get(10)?,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|mut question| {
            question.categories = links.remove(&question.id).unwrap_or_default();
            question
        })
        .collect();
    QuestionBank::from_file(BankFile {
        version: crate::questions::BANK_FILE_VERSION,
        sources,
        categories,
        questions,
    })
    .map_err(|source| StorageError::InvalidBank { source })
}

/// A question's difficulty; a number outside 1 to 5 is not a bank this server wrote.
fn difficulty(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Difficulty> {
    let number: i64 = row.get(index)?;
    u32::try_from(number)
        .ok()
        .and_then(Difficulty::from_number)
        .ok_or_else(|| rusqlite::Error::IntegralValueOutOfRange(index, number))
}

/// A question's correct choice; a number outside 1 to 3 is not a bank this server wrote.
fn correct_choice(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Choice> {
    let number: u8 = row.get(index)?;
    Choice::new(number)
        .ok_or_else(|| rusqlite::Error::IntegralValueOutOfRange(index, i64::from(number)))
}

/// Replace the saved question bank with `bank`, in bank order.
pub(super) fn write_questions(conn: &rusqlite::Connection, bank: &QuestionBank) -> Result<()> {
    conn.execute_batch(
        "DELETE FROM question_categories; DELETE FROM questions; DELETE FROM categories;
         DELETE FROM question_sources;",
    )?;
    for (position, source) in bank.sources().iter().enumerate() {
        conn.execute(
            "INSERT INTO question_sources (id, position, name, licence, url)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                source.id,
                position as i64,
                source.name,
                source.licence,
                source.url
            ],
        )?;
    }
    for (position, category) in bank.categories().iter().enumerate() {
        conn.execute(
            "INSERT INTO categories (id, position, name) VALUES (?1, ?2, ?3)",
            rusqlite::params![category.id, position as i64, category.name],
        )?;
    }
    let mut link = 0;
    for (position, question) in bank.questions().iter().enumerate() {
        let [a, b, c] = &question.choices;
        conn.execute(
            "INSERT INTO questions (id, position, locale, difficulty, text, choice1, choice2,
                                    choice3, correct, source, origin_category, origin_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                question.id.0,
                position as i64,
                question.locale,
                question.difficulty.number(),
                question.text,
                a,
                b,
                c,
                question.correct.get(),
                question.source,
                question.origin.category,
                question.origin.id
            ],
        )?;
        for category in &question.categories {
            conn.execute(
                "INSERT INTO question_categories (question_id, category, position)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![question.id.0, category, link],
            )?;
            link += 1;
        }
    }
    Ok(())
}
