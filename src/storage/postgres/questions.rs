//! The question bank's tables.

use std::collections::BTreeMap;

use postgres::GenericClient;

use super::unsigned;
use crate::{
    ids::QuestionId,
    questions::{BankFile, Category, Choice, Difficulty, Origin, Question, QuestionBank, Source},
    storage::{Result, StorageError},
};

pub(super) fn load_questions(client: &mut impl GenericClient) -> Result<QuestionBank> {
    let sources = client
        .query(
            "SELECT id, name, licence, url FROM question_sources ORDER BY position",
            &[],
        )?
        .iter()
        .map(|row| {
            Ok(Source {
                id: row.try_get(0)?,
                name: row.try_get(1)?,
                licence: row.try_get(2)?,
                url: row.try_get(3)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let categories = client
        .query("SELECT id, name FROM categories ORDER BY position", &[])?
        .iter()
        .map(|row| {
            Ok(Category {
                id: row.try_get(0)?,
                name: row.try_get(1)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut links: BTreeMap<QuestionId, Vec<String>> = BTreeMap::new();
    for row in client.query(
        "SELECT question_id, category FROM question_categories ORDER BY position",
        &[],
    )? {
        let id = QuestionId(unsigned("question_id", row.try_get(0)?)?);
        links.entry(id).or_default().push(row.try_get(1)?);
    }
    let questions = client
        .query(
            "SELECT id, locale, difficulty, text, choice1, choice2, choice3, correct, source,
                    origin_category, origin_id
             FROM questions ORDER BY position",
            &[],
        )?
        .iter()
        .map(|row| {
            let id = QuestionId(unsigned("id", row.try_get(0)?)?);
            Ok(Question {
                id,
                locale: row.try_get(1)?,
                difficulty: difficulty(row.try_get(2)?)?,
                text: row.try_get(3)?,
                choices: [row.try_get(4)?, row.try_get(5)?, row.try_get(6)?],
                correct: correct_choice(row.try_get(7)?)?,
                categories: links.remove(&id).unwrap_or_default(),
                source: row.try_get(8)?,
                origin: Origin {
                    category: row.try_get(9)?,
                    id: row.try_get(10)?,
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    QuestionBank::from_file(BankFile {
        version: crate::questions::BANK_FILE_VERSION,
        sources,
        categories,
        questions,
    })
    .map_err(|source| StorageError::InvalidBank { source })
}

/// A question's difficulty; a number outside 1 to 5 is not a bank this server wrote.
fn difficulty(number: i16) -> Result<Difficulty> {
    u32::try_from(number)
        .ok()
        .and_then(Difficulty::from_number)
        .ok_or_else(|| StorageError::InvalidField {
            field: "difficulty",
            value: number.to_string(),
        })
}

/// A question's correct choice; a number outside 1 to 3 is not a bank this server wrote.
fn correct_choice(number: i16) -> Result<Choice> {
    u8::try_from(number)
        .ok()
        .and_then(Choice::new)
        .ok_or_else(|| StorageError::InvalidField {
            field: "correct",
            value: number.to_string(),
        })
}

/// Replace the saved question bank with `bank`, in bank order.
pub(super) fn write_questions(client: &mut impl GenericClient, bank: &QuestionBank) -> Result<()> {
    client.batch_execute(
        "DELETE FROM question_categories; DELETE FROM questions; DELETE FROM categories;
         DELETE FROM question_sources;",
    )?;
    let insert_source = client.prepare(
        "INSERT INTO question_sources (id, position, name, licence, url)
         VALUES ($1, $2, $3, $4, $5)",
    )?;
    for (position, source) in bank.sources().iter().enumerate() {
        client.execute(
            &insert_source,
            &[
                &source.id,
                &(position as i64),
                &source.name,
                &source.licence,
                &source.url,
            ],
        )?;
    }
    let insert_category =
        client.prepare("INSERT INTO categories (id, position, name) VALUES ($1, $2, $3)")?;
    for (position, category) in bank.categories().iter().enumerate() {
        client.execute(
            &insert_category,
            &[&category.id, &(position as i64), &category.name],
        )?;
    }
    let insert_question = client.prepare(
        "INSERT INTO questions (id, position, locale, difficulty, text, choice1, choice2,
                                choice3, correct, source, origin_category, origin_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )?;
    let insert_link = client.prepare(
        "INSERT INTO question_categories (question_id, category, position) VALUES ($1, $2, $3)",
    )?;
    let mut link: i64 = 0;
    for (position, question) in bank.questions().iter().enumerate() {
        let [a, b, c] = &question.choices;
        let id = i64::from(question.id.0);
        // Both fit: a difficulty is 1 to 5 and a choice 1 to 3.
        let difficulty = question.difficulty.number() as i16;
        let correct = i16::from(question.correct.get());
        client.execute(
            &insert_question,
            &[
                &id,
                &(position as i64),
                &question.locale,
                &difficulty,
                &question.text,
                a,
                b,
                c,
                &correct,
                &question.source,
                &question.origin.category,
                &question.origin.id,
            ],
        )?;
        for category in &question.categories {
            client.execute(&insert_link, &[&id, category, &link])?;
            link += 1;
        }
    }
    Ok(())
}
