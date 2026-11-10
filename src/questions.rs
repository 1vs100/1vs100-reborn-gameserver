//! Question bank served to the title.
//! Order is served order, so the bank is a list; without an import the built-in seed bank is served.

use crate::{ids::QuestionId, schedule::Mode};
use serde::{Deserialize, Serialize};

/// The only locale the bank serves (the title asks `en-US` or `en-us`; the short form none).
pub const LOCALE: &str = "en-us";

/// How hard a question is, as the title's question progressions ask it: level 1 to 5.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Difficulty {
    VeryEasy,
    Easy,
    Medium,
    Hard,
    VeryHard,
}

impl Difficulty {
    /// Every difficulty, easiest first.
    pub const ALL: [Self; 5] = [
        Self::VeryEasy,
        Self::Easy,
        Self::Medium,
        Self::Hard,
        Self::VeryHard,
    ];

    /// The difficulty the title numbers `number` (1 to 5); `None` for a number that names none.
    pub fn from_number(number: u32) -> Option<Self> {
        Self::ALL
            .get(usize::try_from(number).ok()?.checked_sub(1)?)
            .copied()
    }

    /// The number the title gives this difficulty: 1 (very easy) to 5 (very hard).
    pub fn number(self) -> u32 {
        self.index() as u32 + 1
    }

    /// The difficulty's place in a list or a count of the five, counted from 0 for very easy.
    pub fn index(self) -> usize {
        match self {
            Self::VeryEasy => 0,
            Self::Easy => 1,
            Self::Medium => 2,
            Self::Hard => 3,
            Self::VeryHard => 4,
        }
    }

    /// The difficulty of a question at title level `level` (1 to 5); 0 is the easiest, above 5 the
    /// hardest.
    pub fn from_level(level: u32) -> Self {
        match level {
            0 | 1 => Self::VeryEasy,
            2 => Self::Easy,
            3 => Self::Medium,
            4 => Self::Hard,
            _ => Self::VeryHard,
        }
    }

    /// The title `mobai` level the Mob's skill is measured against.
    pub(crate) fn level(self) -> f64 {
        match self {
            Self::VeryEasy => 0.0,
            Self::Easy => 1.0,
            Self::Medium => 3.0,
            Self::Hard => 6.0,
            Self::VeryHard => 8.0,
        }
    }
}

impl std::fmt::Display for Difficulty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.number().fmt(f)
    }
}

impl Serialize for Difficulty {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.number())
    }
}

impl<'de> Deserialize<'de> for Difficulty {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let number = u32::deserialize(deserializer)?;
        Self::from_number(number).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "{number} names no difficulty: the difficulties are 1 to 5"
            ))
        })
    }
}

/// An answer choice as the title numbers it: A is 1, B is 2 and C is 3.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Choice {
    A,
    B,
    C,
}

impl Choice {
    /// Every choice, in the title's order.
    pub const ALL: [Self; 3] = [Self::A, Self::B, Self::C];

    /// The choice the title numbers `number`; `None` for a number that names no choice.
    pub fn new(number: u8) -> Option<Self> {
        Self::ALL.get(usize::from(number).checked_sub(1)?).copied()
    }

    /// The number the title gives this choice.
    pub fn get(self) -> u8 {
        match self {
            Self::A => 1,
            Self::B => 2,
            Self::C => 3,
        }
    }

    /// The choice's place in a list or a count of the three choices, counted from 0 for A.
    pub fn index(self) -> usize {
        usize::from(self.get()) - 1
    }
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

impl Serialize for Choice {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.get())
    }
}

impl<'de> Deserialize<'de> for Choice {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let number = u8::deserialize(deserializer)?;
        Self::new(number).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "{number} names no answer choice: the choices are 1, 2 and 3"
            ))
        })
    }
}

/// One question as the title is served it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: QuestionId,
    #[serde(default = "default_locale")]
    pub locale: String,
    /// How hard the title's question progressions find it.
    pub difficulty: Difficulty,
    pub text: String,
    pub choices: [String; 3],
    /// The right choice.
    pub correct: Choice,
    /// Category ids (`music`); none for the built-in questions.
    #[serde(default)]
    pub categories: Vec<String>,
    /// The id of the source it came from; the bank need not list it.
    pub source: String,
    /// Where it was in that source, as far as the source says.
    #[serde(default, skip_serializing_if = "Origin::is_empty")]
    pub origin: Origin,
}

/// Question position in its source: source category and source id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl Origin {
    pub fn is_empty(&self) -> bool {
        self.category.is_none() && self.id.is_none()
    }
}

fn default_locale() -> String {
    LOCALE.to_owned()
}

/// Question category, in our own namespace; sources map onto these. Older `source` fields are ignored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
}

/// Where questions come from, and whom to credit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub licence: String,
    #[serde(default)]
    pub url: String,
}

/// The bank file: what the import tool writes and the admin imports and exports.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BankFile {
    pub version: u32,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub categories: Vec<Category>,
    pub questions: Vec<Question>,
}

pub const BANK_FILE_VERSION: u32 = 1;

/// Most ids one question list holds. EP takes at most 19 per difficulty from the front of a
/// list and PT picks from it at random. The title reads a list into a 0x8000-byte buffer
/// (`wire::v1149::INFLATE_CAPACITY`); a list that exceeds it fails with `QMS_NO_QUESTIONS`.
pub const MAX_LISTED_IDS: usize = 49;

/// Selects the question lists served for one show; derived from its session or episode id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShowSeed(u64);

impl ShowSeed {
    /// FNV-1a of the id, lowercased so either casing a console sends gives the same seed.
    pub fn of(show_id: &str) -> Self {
        let hash = show_id
            .bytes()
            .map(|b| b.to_ascii_lowercase())
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, b| {
                (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            });
        Self(hash)
    }
}

/// SplitMix64 state increment (2^64 divided by the golden ratio).
const SPLITMIX_INCREMENT: u64 = 0x9e37_79b9_7f4a_7c15;

/// SplitMix64 generator: small, and the same sequence on every platform.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(SPLITMIX_INCREMENT);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// Built-in bank: captured 251317 plus filler so rounds run past question 1.
const SEED: &str = include_str!("../assets/seed-questions.json");

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestionBank {
    questions: Vec<Question>,
    categories: Vec<Category>,
    sources: Vec<Source>,
}

/// Why a bank file is not served ([`QuestionBank::from_file`]).
#[derive(Debug, PartialEq, Eq)]
pub enum BankError {
    /// The file is a version this server does not read.
    Version { found: u32, expected: u32 },
    /// Two questions of the file share an id.
    DuplicateId { id: QuestionId },
    /// A question has no text.
    NoText { id: QuestionId },
    /// A question's choices are blank or repeat each other.
    ChoicesNotDistinct { id: QuestionId },
    /// A question names a category the file does not list.
    UnknownCategory { id: QuestionId, category: String },
}

impl std::fmt::Display for BankError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BankError::Version { found, expected } => {
                write!(
                    f,
                    "bank file version {found} (this server reads {expected})"
                )
            }
            BankError::DuplicateId { id } => write!(f, "question {id}: the id is there twice"),
            BankError::NoText { id } => write!(f, "question {id}: no text"),
            BankError::ChoicesNotDistinct { id } => {
                write!(f, "question {id}: needs three different choices")
            }
            BankError::UnknownCategory { id, category } => {
                write!(f, "question {id}: unknown category {category:?}")
            }
        }
    }
}

impl std::error::Error for BankError {}

impl QuestionBank {
    /// Built-in bank, uncategorised.
    pub fn seed() -> Self {
        let file = serde_json::from_str(SEED).expect("assets/seed-questions.json is a bank file");
        Self::from_file(file).expect("assets/seed-questions.json is a valid bank")
    }

    /// Stored bank, or the built-in one while nothing is imported.
    pub fn in_effect(stored: QuestionBank) -> Self {
        if stored.is_empty() {
            Self::seed()
        } else {
            stored
        }
    }

    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }

    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    pub fn categories(&self) -> &[Category] {
        &self.categories
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn get(&self, id: QuestionId) -> Option<&Question> {
        self.questions.iter().find(|question| question.id == id)
    }

    /// Fallback question: captured 251317 if banked, else the first.
    pub fn fallback(&self) -> Option<&Question> {
        self.get(QuestionId(251_317))
            .or_else(|| self.questions.first())
    }

    /// The ids of `difficulty`, in bank order; of `category` only, when given.
    pub fn ids(&self, difficulty: Difficulty, category: Option<&str>) -> Vec<QuestionId> {
        self.questions
            .iter()
            .filter(|question| question.difficulty == difficulty)
            .filter(|question| {
                category.is_none_or(|category| question.categories.iter().any(|c| c == category))
            })
            .map(|question| question.id)
            .collect()
    }

    /// Ids served at `difficulty`; whole difficulty when the category has none there.
    pub fn served_ids(&self, difficulty: Difficulty, category: Option<&str>) -> Vec<QuestionId> {
        if let Some(category) = category {
            let ids = self.ids(difficulty, Some(category));
            if !ids.is_empty() {
                return ids;
            }
            tracing::warn!(
                category,
                difficulty = difficulty.number(),
                "no question of the show's category at this difficulty: serving all of them"
            );
        }
        self.ids(difficulty, None)
    }

    /// The ids one question list holds at `difficulty`: a sample of [`MAX_LISTED_IDS`] in an
    /// order fixed by `show`, so every console of a show gets the same list and shows differ.
    pub fn listed_ids(
        &self,
        difficulty: Difficulty,
        category: Option<&str>,
        show: ShowSeed,
    ) -> Vec<QuestionId> {
        let mut ids = self.served_ids(difficulty, category);
        let mut rng =
            SplitMix64(show.0 ^ u64::from(difficulty.number()).wrapping_mul(SPLITMIX_INCREMENT));
        let picked = ids.len().min(MAX_LISTED_IDS);
        for i in 0..picked {
            let span = (ids.len() - i) as u64;
            let j = i + (rng.next() % span) as usize;
            ids.swap(i, j);
        }
        ids.truncate(MAX_LISTED_IDS);
        ids
    }

    /// The sources of `category`'s questions, each once, in bank order.
    pub fn category_sources(&self, category: &str) -> Vec<String> {
        let mut sources: Vec<String> = Vec::new();
        for question in &self.questions {
            if question.categories.iter().any(|c| c == category)
                && !sources.contains(&question.source)
            {
                sources.push(question.source.clone());
            }
        }
        sources
    }

    /// How many questions `category` has at difficulties 1 to 5.
    pub fn counts(&self, category: &str) -> [usize; 5] {
        let mut counts = [0; 5];
        for difficulty in Difficulty::ALL {
            counts[difficulty.index()] = self.ids(difficulty, Some(category)).len();
        }
        counts
    }

    /// First difficulty where `category` holds fewer than a Primetime round asks.
    pub fn primetime_shortfall(&self, category: &str) -> Option<Shortfall> {
        self.shortfall(category, Mode::Primetime)
    }

    /// First difficulty where `category` holds fewer than a show of `mode` asks.
    pub fn shortfall(&self, category: &str, mode: Mode) -> Option<Shortfall> {
        let needs = show_needs(mode);
        let has = self.counts(category);
        Difficulty::ALL.into_iter().find_map(|difficulty| {
            let index = difficulty.index();
            (has[index] < needs[index]).then_some(Shortfall {
                mode,
                difficulty,
                has: has[index],
                needs: needs[index],
            })
        })
    }

    /// Bank from a bank file, if valid.
    pub fn from_file(file: BankFile) -> Result<Self, BankError> {
        if file.version != BANK_FILE_VERSION {
            return Err(BankError::Version {
                found: file.version,
                expected: BANK_FILE_VERSION,
            });
        }
        let bank = Self {
            questions: file.questions,
            categories: file.categories,
            sources: file.sources,
        };
        bank.validate()?;
        Ok(bank)
    }

    pub fn to_file(&self) -> BankFile {
        BankFile {
            version: BANK_FILE_VERSION,
            sources: self.sources.clone(),
            categories: self.categories.clone(),
            questions: self.questions.clone(),
        }
    }

    fn validate(&self) -> Result<(), BankError> {
        let mut seen = std::collections::HashSet::new();
        for question in &self.questions {
            let id = question.id;
            if !seen.insert(question.id) {
                return Err(BankError::DuplicateId { id });
            }
            if question.text.trim().is_empty() {
                return Err(BankError::NoText { id });
            }
            let [a, b, c] = &question.choices;
            if [a, b, c].iter().any(|choice| choice.trim().is_empty()) || a == b || b == c || a == c
            {
                return Err(BankError::ChoicesNotDistinct { id });
            }
            if let Some(category) = question
                .categories
                .iter()
                .find(|id| !self.categories.iter().any(|category| &category.id == *id))
            {
                return Err(BankError::UnknownCategory {
                    id,
                    category: category.clone(),
                });
            }
        }
        Ok(())
    }

    /// Questions, categories and sources of `other` replace those with the same id here; new
    /// ones are added at the end.
    pub fn merge(&mut self, other: QuestionBank) {
        fn upsert<T>(into: &mut Vec<T>, from: Vec<T>, same: impl Fn(&T, &T) -> bool) {
            for item in from {
                match into.iter_mut().find(|kept| same(kept, &item)) {
                    Some(kept) => *kept = item,
                    None => into.push(item),
                }
            }
        }
        upsert(&mut self.sources, other.sources, |a, b| a.id == b.id);
        upsert(&mut self.categories, other.categories, |a, b| a.id == b.id);
        upsert(&mut self.questions, other.questions, |a, b| a.id == b.id);
    }
}

/// A category's gap to one show ([`QuestionBank::shortfall`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortfall {
    pub mode: Mode,
    pub difficulty: Difficulty,
    pub has: usize,
    pub needs: usize,
}

impl std::fmt::Display for Shortfall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} question(s) at difficulty {}, {} asks {}",
            self.has,
            self.difficulty,
            match self.mode {
                Mode::Primetime => "a Primetime round",
                Mode::ExtendedPlay => "an Extended Play episode",
            },
            self.needs
        )
    }
}

/// How many questions one Primetime round asks at difficulties 1 to 5 (2, 3, 5, 4, 7).
pub fn primetime_round() -> [usize; 5] {
    show_needs(Mode::Primetime)
}

/// Questions at difficulties 1 to 5 that a Primetime round or an Extended Play episode asks.
fn show_needs(mode: Mode) -> [usize; 5] {
    let progression: &[u32] = match mode {
        Mode::Primetime => &crate::xsor::PT_QUESTION_PROGRESSION,
        Mode::ExtendedPlay => &crate::xsor::EP_QUESTION_PROGRESSION,
    };
    let mut needs = [0; 5];
    for &difficulty in progression {
        needs[difficulty as usize - 1] += 1;
    }
    needs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(id: u32, difficulty: Difficulty, categories: &[&str]) -> Question {
        Question {
            origin: Origin::default(),
            id: QuestionId(id),
            locale: LOCALE.into(),
            difficulty,
            text: format!("Question {id}?"),
            choices: ["A".into(), "B".into(), "C".into()],
            correct: Choice::B,
            categories: categories.iter().map(|&c| c.to_owned()).collect(),
            source: "otdb".into(),
        }
    }

    fn file(questions: Vec<Question>) -> BankFile {
        BankFile {
            version: BANK_FILE_VERSION,
            sources: vec![Source {
                id: "otdb".into(),
                name: "Open Trivia Database".into(),
                licence: "CC BY-SA 4.0".into(),
                url: "https://opentdb.com".into(),
            }],
            categories: vec![
                Category {
                    id: "otdb:9".into(),
                    name: "General Knowledge".into(),
                },
                Category {
                    id: "otdb:11".into(),
                    name: "Film".into(),
                },
            ],
            questions,
        }
    }

    #[test]
    fn each_show_lists_its_own_sample_of_the_difficulty() {
        // Easy holds VeryEasy's ids plus 1000, in the same bank order.
        let bank = QuestionBank::from_file(file(
            (1..=300)
                .map(|id| question(id, Difficulty::VeryEasy, &[]))
                .chain((1..=300).map(|id| question(1000 + id, Difficulty::Easy, &[])))
                .collect(),
        ))
        .unwrap();
        let episode = ShowSeed::of("f605b5d4-7f6b-42b0-95ab-16aee9b51db4");
        let listed = bank.listed_ids(Difficulty::VeryEasy, None, episode);
        assert_eq!(listed.len(), MAX_LISTED_IDS);
        let mut unique = listed.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), MAX_LISTED_IDS);
        assert!(listed.iter().all(|id| id.0 <= 300), "very easy only");

        let upper = ShowSeed::of("F605B5D4-7F6B-42B0-95AB-16AEE9B51DB4");
        assert_eq!(bank.listed_ids(Difficulty::VeryEasy, None, upper), listed);
        let other = ShowSeed::of("faa99237-7f6c-42bb-9e80-86514695c284");
        assert_ne!(bank.listed_ids(Difficulty::VeryEasy, None, other), listed);
        let easy: Vec<u32> = bank
            .listed_ids(Difficulty::Easy, None, episode)
            .iter()
            .map(|id| id.0 - 1000)
            .collect();
        assert_ne!(
            easy,
            listed.iter().map(|id| id.0).collect::<Vec<_>>(),
            "each difficulty in its own order"
        );
    }

    #[test]
    fn a_short_difficulty_is_listed_whole() {
        let bank = QuestionBank::seed();
        let show = ShowSeed::of("faa99237-7f6c-42bb-9e80-86514695c284");
        for difficulty in Difficulty::ALL {
            let mut listed = bank.listed_ids(difficulty, None, show);
            listed.sort_unstable();
            let mut all = bank.ids(difficulty, None);
            all.sort_unstable();
            assert_eq!(listed, all);
        }
    }

    #[test]
    fn the_seed_keeps_its_order_and_its_first_question() {
        let seed = QuestionBank::seed();
        assert_eq!(seed.questions().len(), 41);
        assert_eq!(seed.fallback().unwrap().id, QuestionId(251_317));
        assert_eq!(seed.get(QuestionId(251_317)).unwrap().choices[2], "Apple");
        assert!(
            seed.ids(Difficulty::VeryEasy, Some("otdb:9")).is_empty(),
            "the seed has no categories"
        );
        assert_eq!(QuestionBank::in_effect(QuestionBank::default()), seed);
    }

    #[test]
    fn ids_and_counts_follow_difficulty_and_category() {
        let bank = QuestionBank::from_file(file(vec![
            question(1, Difficulty::VeryEasy, &["otdb:9"]),
            question(2, Difficulty::VeryEasy, &["otdb:11"]),
            question(3, Difficulty::Easy, &["otdb:9", "otdb:11"]),
        ]))
        .unwrap();
        assert_eq!(bank.ids(Difficulty::VeryEasy, None), [1, 2].map(QuestionId));
        assert_eq!(
            bank.ids(Difficulty::VeryEasy, Some("otdb:9")),
            [1].map(QuestionId)
        );
        assert_eq!(
            bank.ids(Difficulty::Easy, Some("otdb:11")),
            [3].map(QuestionId)
        );
        assert_eq!(bank.counts("otdb:9"), [1, 1, 0, 0, 0]);
        let unsorted = QuestionBank::from_file(file(vec![
            question(9, Difficulty::VeryEasy, &[]),
            question(4, Difficulty::VeryEasy, &[]),
        ]));
        assert_eq!(
            unsorted.unwrap().ids(Difficulty::VeryEasy, None),
            [9, 4].map(QuestionId),
            "bank order, not id order"
        );
        assert_eq!(QuestionBank::from_file(bank.to_file()).unwrap(), bank);
    }

    #[test]
    fn a_category_is_checked_against_one_primetime_round() {
        assert_eq!(primetime_round(), [2, 3, 5, 4, 7]);
        let mut questions = Vec::new();
        for (index, needs) in primetime_round().into_iter().enumerate() {
            for _ in 0..needs {
                let id = questions.len() as u32 + 1;
                questions.push(question(id, Difficulty::ALL[index], &["otdb:9"]));
            }
        }
        let full = QuestionBank::from_file(file(questions.clone())).unwrap();
        assert_eq!(full.primetime_shortfall("otdb:9"), None);
        questions.retain(|question| question.id != QuestionId(5));
        let short = QuestionBank::from_file(file(questions)).unwrap();
        let shortfall = short.primetime_shortfall("otdb:9").unwrap();
        assert_eq!(
            shortfall,
            Shortfall {
                mode: Mode::Primetime,
                difficulty: Difficulty::Easy,
                has: 2,
                needs: 3
            }
        );
        assert_eq!(
            shortfall.to_string(),
            "2 question(s) at difficulty 2, a Primetime round asks 3"
        );
        assert_eq!(short.primetime_shortfall("otdb:11").unwrap().has, 0);
    }

    #[test]
    fn a_show_is_served_its_category_or_else_the_whole_difficulty() {
        let bank = QuestionBank::from_file(file(vec![
            question(1, Difficulty::VeryEasy, &["otdb:9"]),
            question(2, Difficulty::VeryEasy, &["otdb:11"]),
            question(3, Difficulty::Easy, &["otdb:9"]),
        ]))
        .unwrap();
        assert_eq!(
            bank.served_ids(Difficulty::VeryEasy, Some("otdb:11")),
            [2].map(QuestionId)
        );
        assert_eq!(
            bank.served_ids(Difficulty::Easy, Some("otdb:11")),
            [3].map(QuestionId),
            "the fallback"
        );
        assert_eq!(
            bank.served_ids(Difficulty::VeryEasy, None),
            [1, 2].map(QuestionId)
        );
    }

    #[test]
    fn sources_fill_one_category_and_keep_their_credit_and_origin() {
        let mut first = file(vec![question(1, Difficulty::VeryEasy, &["otdb:9"])]);
        first.questions[0].origin = Origin {
            category: Some("9".into()),
            id: None,
        };
        let mut bank = QuestionBank::from_file(first).unwrap();
        let mut second = file(vec![Question {
            source: "other".into(),
            origin: Origin {
                category: Some("trivia".into()),
                id: Some("A-17".into()),
            },
            ..question(2, Difficulty::VeryEasy, &["otdb:9"])
        }]);
        second.sources.push(Source {
            id: "other".into(),
            name: "Another source".into(),
            licence: String::new(),
            url: String::new(),
        });
        bank.merge(QuestionBank::from_file(second).unwrap());
        assert_eq!(
            bank.counts("otdb:9"),
            [2, 0, 0, 0, 0],
            "one category, two sources"
        );
        assert_eq!(bank.category_sources("otdb:9"), ["otdb", "other"]);
        assert_eq!(
            bank.get(QuestionId(2)).unwrap().origin.id.as_deref(),
            Some("A-17")
        );
        let json = serde_json::to_string(&bank.to_file()).unwrap();
        assert!(json.contains(r#""origin":{"category":"9"}"#), "{json}");
        assert_eq!(
            json.matches("\"origin\"").count(),
            2,
            "only where there is one"
        );

        // Older bank files name a source on each category; that is ignored.
        let old: BankFile = serde_json::from_str(
            r#"{"version":1,"sources":[{"id":"otdb","name":"OpenTriviaDB"}],
                "categories":[{"id":"otdb:9","name":"General Knowledge","source":"otdb"}],
                "questions":[]}"#,
        )
        .unwrap();
        assert_eq!(old.categories[0].name, "General Knowledge");
    }

    #[test]
    fn a_choice_is_one_of_three_numbers() {
        assert_eq!(Choice::new(0), None);
        assert_eq!(Choice::new(1), Some(Choice::A));
        assert_eq!(Choice::new(3), Some(Choice::C));
        assert_eq!(Choice::new(4), None);
        assert_eq!(serde_json::to_string(&Choice::B).unwrap(), "2");
        assert_eq!(serde_json::from_str::<Choice>("3").unwrap(), Choice::C);
        assert!(serde_json::from_str::<Choice>("0").is_err());
    }

    #[test]
    fn a_bank_file_is_validated() {
        let bad = |edit: &dyn Fn(&mut Question)| {
            let mut question = question(1, Difficulty::VeryEasy, &["otdb:9"]);
            edit(&mut question);
            QuestionBank::from_file(file(vec![question])).unwrap_err()
        };
        let mut no_difficulty =
            serde_json::to_value(file(vec![question(1, Difficulty::VeryEasy, &[])])).unwrap();
        no_difficulty["questions"][0]["difficulty"] = 6.into();
        assert!(
            serde_json::from_value::<BankFile>(no_difficulty).is_err(),
            "a number that names no difficulty is no bank file"
        );
        let mut no_choice =
            serde_json::to_value(file(vec![question(1, Difficulty::VeryEasy, &[])])).unwrap();
        no_choice["questions"][0]["correct"] = 0.into();
        assert!(
            serde_json::from_value::<BankFile>(no_choice).is_err(),
            "a number that names no choice is no bank file"
        );
        assert_eq!(
            bad(&|q| q.choices[1] = "A".into()),
            BankError::ChoicesNotDistinct { id: QuestionId(1) }
        );
        assert_eq!(
            bad(&|q| q.text = " ".into()),
            BankError::NoText { id: QuestionId(1) }
        );
        assert_eq!(
            bad(&|q| q.categories = vec!["otdb:99".into()]),
            BankError::UnknownCategory {
                id: QuestionId(1),
                category: "otdb:99".into()
            }
        );
        let twice = QuestionBank::from_file(file(vec![
            question(1, Difficulty::VeryEasy, &[]),
            question(1, Difficulty::Easy, &[]),
        ]));
        assert_eq!(
            twice.unwrap_err(),
            BankError::DuplicateId { id: QuestionId(1) }
        );
        let old = BankFile {
            version: 9,
            ..file(vec![])
        };
        assert_eq!(
            QuestionBank::from_file(old).unwrap_err(),
            BankError::Version {
                found: 9,
                expected: BANK_FILE_VERSION
            }
        );
    }

    #[test]
    fn merging_replaces_by_id_and_appends_the_rest() {
        let mut bank = QuestionBank::from_file(file(vec![
            question(1, Difficulty::VeryEasy, &[]),
            question(2, Difficulty::VeryEasy, &[]),
        ]))
        .unwrap();
        let changed = Question {
            text: "Changed?".into(),
            ..question(2, Difficulty::VeryEasy, &[])
        };
        bank.merge(
            QuestionBank::from_file(file(vec![
                changed.clone(),
                question(3, Difficulty::VeryEasy, &[]),
            ]))
            .unwrap(),
        );
        assert_eq!(
            bank.ids(Difficulty::VeryEasy, None),
            [1, 2, 3].map(QuestionId)
        );
        assert_eq!(bank.get(QuestionId(2)), Some(&changed));
        assert_eq!(bank.categories().len(), 2, "categories not doubled");
    }
}
