//! Making a source's questions fit the title, the same for every source: exactly three choices,
//! one title difficulty, a stable id, the length and character limits, duplicates.
//!
//! Every choice is made from a hash of the source and the question text, so importing the same
//! questions again gives the same ids, difficulties and choice order.

use std::collections::HashMap;

use onevs100_gameserver::{
    ids::QuestionId,
    questions::{Choice, Difficulty, Origin, Question},
};

use crate::source::{SourceDifficulty, SourceQuestion};

/// The title's text limits. The defaults are the longest question (77 characters) and answer
/// (19) of the 740 questions the original show shipped with (the 808 debug build's
/// `ExcelQuestions.lua`), rounded up, and ASCII only, as
/// all of those are. Longer texts may well render: a LAN test with long questions decides, until
/// then these are flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub question_chars: usize,
    pub answer_chars: usize,
    pub ascii_only: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            question_chars: 80,
            answer_chars: 20,
            ascii_only: true,
        }
    }
}

/// Imported ids: clear of the title's own and the built-in ones (251317-251546), and below 2^24,
/// so they survive the title reading them as single-precision floats (they are served as
/// `type="float"` items in the Available lists).
pub const IDS: std::ops::Range<u32> = 1_000_000..16_777_216;

/// Why a question was left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Skip {
    FewerThanTwoWrongAnswers,
    QuestionTooLong,
    AnswerTooLong,
    NotAscii,
    Empty,
    NoTitleDifficulty,
    /// The same question again (in this category or another one, which it then also gets).
    Duplicate,
    /// Another question's text hashes to the same id.
    IdCollision,
}

impl Skip {
    pub fn describe(self) -> &'static str {
        match self {
            Skip::FewerThanTwoWrongAnswers => "fewer than two wrong answers",
            Skip::QuestionTooLong => "question too long",
            Skip::AnswerTooLong => "an answer too long",
            Skip::NotAscii => "characters outside ASCII",
            Skip::Empty => "empty text",
            Skip::NoTitleDifficulty => "difficulty outside 1-5",
            Skip::Duplicate => "duplicate",
            Skip::IdCollision => "id taken by another question",
        }
    }
}

/// The questions fitted so far, in the order they arrived (the bank order).
pub struct Fitter {
    source: String,
    limits: Limits,
    questions: Vec<Question>,
    by_id: HashMap<QuestionId, usize>,
}

impl Fitter {
    pub fn new(source: &str, limits: Limits) -> Self {
        Self {
            source: source.to_owned(),
            limits,
            questions: Vec::new(),
            by_id: HashMap::new(),
        }
    }

    /// Fit `question` of our `category` (`vehicles`), which the source calls `source_category`
    /// (`28`), and keep it, or say why not. A question seen again keeps its first origin.
    pub fn add(
        &mut self,
        category: &str,
        source_category: &str,
        question: &SourceQuestion,
    ) -> Result<QuestionId, Skip> {
        let mut fitted = fit(&self.source, category, question, self.limits)?;
        fitted.origin.category = Some(source_category.to_owned());
        if let Some(&index) = self.by_id.get(&fitted.id) {
            let kept = &mut self.questions[index];
            if kept.text != fitted.text {
                return Err(Skip::IdCollision);
            }
            if !kept.categories.iter().any(|c| c == category) {
                kept.categories.push(category.to_owned());
            }
            return Err(Skip::Duplicate);
        }
        self.by_id.insert(fitted.id, self.questions.len());
        let id = fitted.id;
        self.questions.push(fitted);
        Ok(id)
    }

    pub fn into_questions(self) -> Vec<Question> {
        self.questions
    }
}

/// One source question as the title asks it.
pub fn fit(
    source: &str,
    category: &str,
    question: &SourceQuestion,
    limits: Limits,
) -> Result<Question, Skip> {
    let text = question.text.trim().to_owned();
    let correct = question.correct.trim().to_owned();
    let mut wrong: Vec<String> = Vec::new();
    for answer in &question.incorrect {
        let answer = answer.trim();
        if !answer.is_empty() && answer != correct && !wrong.iter().any(|w| w == answer) {
            wrong.push(answer.to_owned());
        }
    }
    if text.is_empty() || correct.is_empty() {
        return Err(Skip::Empty);
    }
    if wrong.len() < 2 {
        return Err(Skip::FewerThanTwoWrongAnswers);
    }
    let hash = fnv1a(&[source.as_bytes(), b"\0", text.as_bytes()]);
    // Two of the wrong answers (dropping one by the hash when there are three or more)...
    while wrong.len() > 2 {
        let drop = (hash >> 8) as usize % wrong.len();
        wrong.remove(drop);
    }
    // ...and the correct one at a position chosen by the hash.
    let position = (hash >> 16) as usize % 3;
    let mut choices = wrong;
    choices.insert(position, correct);
    let difficulty = match question.difficulty {
        SourceDifficulty::Easy => {
            [Difficulty::VeryEasy, Difficulty::Easy][(hash >> 24) as usize % 2]
        }
        SourceDifficulty::Medium => Difficulty::Medium,
        SourceDifficulty::Hard => {
            [Difficulty::Hard, Difficulty::VeryHard][(hash >> 24) as usize % 2]
        }
        SourceDifficulty::Title(difficulty) => match Difficulty::from_number(difficulty) {
            Some(difficulty) => difficulty,
            None => return Err(Skip::NoTitleDifficulty),
        },
    };
    if limits.ascii_only
        && std::iter::once(&text)
            .chain(&choices)
            .any(|t| t.chars().any(|c| !c.is_ascii() || c.is_ascii_control()))
    {
        return Err(Skip::NotAscii);
    }
    if text.chars().count() > limits.question_chars {
        return Err(Skip::QuestionTooLong);
    }
    if choices
        .iter()
        .any(|choice| choice.chars().count() > limits.answer_chars)
    {
        return Err(Skip::AnswerTooLong);
    }
    let [a, b, c]: [String; 3] = choices.try_into().expect("three choices");
    Ok(Question {
        id: QuestionId(IDS.start + (hash % u64::from(IDS.end - IDS.start)) as u32),
        locale: "en-US".to_owned(),
        difficulty,
        text,
        choices: [a, b, c],
        correct: Choice::ALL[position],
        categories: vec![category.to_owned()],
        source: source.to_owned(),
        origin: Origin {
            category: None,
            id: question.id.clone(),
        },
    })
}

/// FNV-1a, 64 bit: stable across runs and platforms, unlike `std`'s hasher.
fn fnv1a(parts: &[&[u8]]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in parts.iter().flat_map(|part| part.iter()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(text: &str, difficulty: SourceDifficulty) -> SourceQuestion {
        SourceQuestion {
            id: None,
            text: text.into(),
            correct: "Right".into(),
            incorrect: vec!["Wrong 1".into(), "Wrong 2".into(), "Wrong 3".into()],
            difficulty,
        }
    }

    /// Each question the fitter takes, and each it leaves out with its reason, one per line.
    #[test]
    fn questions_fit_or_are_skipped_with_their_reason() {
        let limits = Limits::default();
        let mut one_wrong = question("Q?", SourceDifficulty::Easy);
        one_wrong.incorrect = vec!["Right".into(), " Other ".into(), "Other".into()];
        let mut long_answer = question("Q?", SourceDifficulty::Easy);
        long_answer.correct = "y".repeat(21);
        let cases = [
            (
                "a question of the source's easy tier",
                "science-nature",
                question("Which planet is red?", SourceDifficulty::Easy),
            ),
            (
                "a question of the source's hard tier",
                "general-knowledge",
                question("Which planet is blue?", SourceDifficulty::Hard),
            ),
            (
                "a question the source grades itself",
                "general-knowledge",
                question("Q?", SourceDifficulty::Title(4)),
            ),
            (
                "a question graded outside the title's five",
                "general-knowledge",
                question("Q?", SourceDifficulty::Title(6)),
            ),
            ("a question with one wrong answer", "s-1", one_wrong),
            (
                "a question too long",
                "s-1",
                question(&"x".repeat(81), SourceDifficulty::Easy),
            ),
            ("an answer too long", "s-1", long_answer),
            (
                "characters outside ASCII",
                "s-1",
                question("Who painted the Café Terrace?", SourceDifficulty::Easy),
            ),
            ("no text", "s-1", question("  ", SourceDifficulty::Easy)),
        ];
        let mut shown: Vec<String> = cases
            .into_iter()
            .map(|(what, category, source)| {
                format!("{what}: {:?}", fit("otdb", category, &source, limits))
            })
            .collect();
        let relaxed = Limits {
            ascii_only: false,
            ..limits
        };
        shown.push(format!(
            "characters outside ASCII, allowed: {:?}",
            fit(
                "otdb",
                "s-1",
                &question("Who painted the Café Terrace?", SourceDifficulty::Easy),
                relaxed,
            )
        ));
        insta::assert_snapshot!(shown.join("\n"));
    }

    #[test]
    fn a_fitted_question_is_the_same_every_time_with_an_id_of_its_own() {
        let source = question("Which planet is red?", SourceDifficulty::Easy);
        let fitted = fit("otdb", "science-nature", &source, Limits::default()).unwrap();
        assert_eq!(
            fit("otdb", "science-nature", &source, Limits::default()).unwrap(),
            fitted,
            "the same every time"
        );
        assert!(IDS.contains(&fitted.id.0));
        let other = fit("other", "other-1", &source, Limits::default()).unwrap();
        assert_ne!(other.id, fitted.id, "the source is part of the id");
    }

    #[test]
    fn difficulties_cover_the_titles_five_and_place_the_right_answer_anywhere() {
        let mut seen = std::collections::BTreeSet::new();
        for n in 0..200 {
            for (difficulty, allowed) in [
                (
                    SourceDifficulty::Easy,
                    Difficulty::VeryEasy..=Difficulty::Easy,
                ),
                (
                    SourceDifficulty::Medium,
                    Difficulty::Medium..=Difficulty::Medium,
                ),
                (
                    SourceDifficulty::Hard,
                    Difficulty::Hard..=Difficulty::VeryHard,
                ),
            ] {
                let fitted = fit(
                    "otdb",
                    "general-knowledge",
                    &question(&format!("Q{n}?"), difficulty),
                    Limits::default(),
                )
                .unwrap();
                assert!(allowed.contains(&fitted.difficulty));
                seen.insert(fitted.difficulty);
            }
        }
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            Difficulty::ALL,
            "both halves used"
        );
        let positions: std::collections::BTreeSet<u8> = (0..60)
            .map(|n| {
                fit(
                    "otdb",
                    "general-knowledge",
                    &question(&format!("P{n}?"), SourceDifficulty::Medium),
                    Limits::default(),
                )
                .unwrap()
                .correct
                .get()
            })
            .collect();
        assert_eq!(
            positions.into_iter().collect::<Vec<_>>(),
            [1, 2, 3],
            "the right answer moves"
        );
    }

    #[test]
    fn a_question_seen_again_is_kept_once_with_every_category() {
        let mut fitter = Fitter::new("otdb", Limits::default());
        let q = question("Which planet is red?", SourceDifficulty::Easy);
        let id = fitter.add("science-nature", "17", &q).unwrap();
        assert_eq!(fitter.add("science-nature", "17", &q), Err(Skip::Duplicate));
        assert_eq!(
            fitter.add("general-knowledge", "9", &q),
            Err(Skip::Duplicate)
        );
        let mut other = question("Which planet is blue?", SourceDifficulty::Hard);
        other.id = Some("Q-42".into());
        fitter.add("general-knowledge", "9", &other).unwrap();
        let questions = fitter.into_questions();
        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].id, id);
        insta::assert_debug_snapshot!(questions);
    }
}
