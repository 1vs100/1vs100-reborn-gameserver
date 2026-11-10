//! [OpenTriviaDB](https://opentdb.com/api_config.php): its category list, then each category
//! pulled with a session token (so no question comes twice), multiple choice only, Base64-encoded
//! (so no HTML entities reach the title), one call per 5 s as the API asks.
//!
//! A call for more questions than the token has left answers code 4 with none (seen on the API,
//! 2026-10-23: 32 left in a category, `amount=50` -> code 4, `amount=10` -> 10). So questions are
//! pulled 50 at a time, and on code 4 (or 1, no results) with half as many, down to 1; code 4 at
//! 1 is the end. The count call cannot say where the end is: it ignores `type=multiple`.
//!
//! A whole import is one such pull without a category ([`QuestionSource::fetch_all`]): each
//! result names its category, so the shrinking pages at the end are paid once, not once per
//! category (about 113 calls instead of 338, 10 minutes instead of 31).

use std::time::{Duration, Instant};

use base64::Engine;
use serde::Deserialize;

use crate::source::{
    Attribution, FetchReport, QuestionSource, SkipReason, SourceCategory, SourceDifficulty,
    SourceError, SourceQuestion, StreamReport,
};

const BASE: &str = "https://opentdb.com";

/// Which of our categories (`categories.rs`) each OpenTriviaDB category is (its ids and names as
/// `api_category.php` listed them, 2026-10-23).
const OURS: &[(u32, &str)] = &[
    (9, "general-knowledge"),   // General Knowledge
    (10, "books"),              // Entertainment: Books
    (11, "film"),               // Entertainment: Film
    (12, "music"),              // Entertainment: Music
    (13, "musicals-theatre"),   // Entertainment: Musicals & Theatres
    (14, "television"),         // Entertainment: Television
    (15, "video-games"),        // Entertainment: Video Games
    (16, "board-games"),        // Entertainment: Board Games
    (17, "science-nature"),     // Science & Nature
    (18, "computers"),          // Science: Computers
    (19, "mathematics"),        // Science: Mathematics
    (20, "mythology"),          // Mythology
    (21, "sports"),             // Sports
    (22, "geography"),          // Geography
    (23, "history"),            // History
    (24, "politics"),           // Politics
    (25, "art"),                // Art
    (26, "celebrities"),        // Celebrities
    (27, "animals"),            // Animals
    (28, "vehicles"),           // Vehicles
    (29, "comics"),             // Entertainment: Comics
    (30, "gadgets"),            // Science: Gadgets
    (31, "anime-manga"),        // Entertainment: Japanese Anime & Manga
    (32, "cartoons-animation"), // Entertainment: Cartoon & Animations
];

fn ours(id: u32) -> Option<&'static str> {
    OURS.iter()
        .find(|(known, _)| *known == id)
        .map(|(_, ours)| *ours)
}
/// The API allows one call per 5 s per IP; a little more, so clock jitter never trips code 5.
pub const PACING: Duration = Duration::from_millis(5_500);
const PAGE: u32 = 50;
/// Code 5 (too many requests) is retried this often, each after the pacing again.
const RATE_LIMIT_RETRIES: u32 = 3;

/// One GET, answering the body. The real one is [`UreqHttp`]; tests answer recorded bodies.
pub trait Http {
    fn get(&mut self, url: &str) -> Result<String, SourceError>;
}

pub struct UreqHttp(ureq::Agent);

impl UreqHttp {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Self(config.into())
    }
}

impl Default for UreqHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl Http for UreqHttp {
    fn get(&mut self, url: &str) -> Result<String, SourceError> {
        let failed = |error: ureq::Error| SourceError::Http {
            url: url.to_owned(),
            detail: error.to_string(),
        };
        self.0
            .get(url)
            .call()
            .map_err(failed)?
            .body_mut()
            .read_to_string()
            .map_err(failed)
    }
}

pub struct OpenTriviaDb {
    http: Box<dyn Http>,
    pacing: Duration,
    last_call: Option<Instant>,
}

impl OpenTriviaDb {
    pub fn new(http: Box<dyn Http>, pacing: Duration) -> Self {
        Self {
            http,
            pacing,
            last_call: None,
        }
    }

    /// A paced GET of `path` decoded as JSON.
    fn call<T: for<'de> Deserialize<'de>>(&mut self, path: &str) -> Result<T, SourceError> {
        if let Some(last) = self.last_call {
            let wait = self.pacing.saturating_sub(last.elapsed());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
        let url = format!("{BASE}/{path}");
        let body = self.http.get(&url);
        self.last_call = Some(Instant::now());
        serde_json::from_str(&body?).map_err(|error| SourceError::Json { url, error })
    }

    /// Every multiple-choice result, of `category` or of all of them, handed to `each` as it
    /// arrives, under one session token.
    fn pull(
        &mut self,
        category: Option<&str>,
        each: &mut dyn FnMut(&Encoded) -> Result<(), SourceError>,
    ) -> Result<(), SourceError> {
        let filter = category.map_or(String::new(), |id| format!("&category={id}"));
        let mut token = self.new_token()?;
        let mut amount = PAGE;
        let mut rate_limited = 0;
        loop {
            let answer: QuestionsAnswer = self.call(&format!(
                "api.php?amount={amount}{filter}&type=multiple&encode=base64&token={token}"
            ))?;
            match answer.response_code {
                ResponseCode::Success => {
                    rate_limited = 0;
                    for result in &answer.results {
                        each(result)?;
                    }
                }
                // Fewer left than asked for, or none of these at all.
                ResponseCode::NoResults | ResponseCode::TokenEmpty if amount > 1 => {
                    amount = (amount / 2).max(1);
                }
                ResponseCode::NoResults | ResponseCode::TokenEmpty => return Ok(()),
                // Token not found (idle for 6 hours): a new one may repeat questions, which the
                // fitting drops as duplicates.
                ResponseCode::TokenNotFound => token = self.new_token()?,
                ResponseCode::RateLimited if rate_limited < RATE_LIMIT_RETRIES => rate_limited += 1,
                // Rate limited once more, an invalid parameter, or a code the API added later:
                // no other call is left to try.
                code => {
                    let scope =
                        category.map_or("all categories".to_owned(), |id| format!("category {id}"));
                    return Err(SourceError::Response {
                        scope,
                        code: code.number(),
                    });
                }
            }
        }
    }

    fn new_token(&mut self) -> Result<String, SourceError> {
        let answer: TokenAnswer = self.call("api_token.php?command=request")?;
        match (answer.response_code, answer.token) {
            (ResponseCode::Success, Some(token)) => Ok(token),
            (code, _) => Err(SourceError::Token {
                code: code.number(),
            }),
        }
    }
}

#[derive(Deserialize)]
struct CategoryList {
    trivia_categories: Vec<CategoryEntry>,
}

#[derive(Deserialize)]
struct CategoryEntry {
    id: u32,
    name: String,
}

/// The API's `response_code`: what became of a call (`api_config.php`, "Response Codes").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "u32")]
pub enum ResponseCode {
    /// 0: the results, if any, are in the answer.
    Success,
    /// 1: no results for these parameters; the token still has questions left.
    NoResults,
    /// 2: a parameter is missing or malformed.
    InvalidParameter,
    /// 3: the token is not known (no call for 6 hours).
    TokenNotFound,
    /// 4: the token has no questions left.
    TokenEmpty,
    /// 5: more than one call within 5 s.
    RateLimited,
    /// A code this tool does not know: the API's table can grow.
    Unknown(u32),
}

impl ResponseCode {
    /// The number the API wrote, for the error messages.
    fn number(self) -> u32 {
        match self {
            ResponseCode::Success => 0,
            ResponseCode::NoResults => 1,
            ResponseCode::InvalidParameter => 2,
            ResponseCode::TokenNotFound => 3,
            ResponseCode::TokenEmpty => 4,
            ResponseCode::RateLimited => 5,
            ResponseCode::Unknown(code) => code,
        }
    }
}

impl From<u32> for ResponseCode {
    fn from(code: u32) -> Self {
        match code {
            0 => ResponseCode::Success,
            1 => ResponseCode::NoResults,
            2 => ResponseCode::InvalidParameter,
            3 => ResponseCode::TokenNotFound,
            4 => ResponseCode::TokenEmpty,
            5 => ResponseCode::RateLimited,
            code => ResponseCode::Unknown(code),
        }
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    response_code: ResponseCode,
    token: Option<String>,
}

#[derive(Deserialize)]
struct QuestionsAnswer {
    response_code: ResponseCode,
    #[serde(default)]
    results: Vec<Encoded>,
}

/// A result with every field Base64-encoded (`encode=base64`).
#[derive(Deserialize)]
struct Encoded {
    #[serde(rename = "type")]
    kind: String,
    difficulty: String,
    category: String,
    question: String,
    correct_answer: String,
    incorrect_answers: Vec<String>,
}

fn decode(field: &str) -> Result<String, SourceError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(field)
        .map_err(|error| SourceError::Base64 { error })?;
    String::from_utf8(bytes).map_err(|error| SourceError::Utf8 { error })
}

impl Encoded {
    /// The question, or why it is left out.
    fn decode(&self) -> Result<Result<SourceQuestion, SkipReason>, SourceError> {
        if decode(&self.kind)? != "multiple" {
            return Ok(Err(SkipReason::NotMultipleChoice));
        }
        let difficulty = match decode(&self.difficulty)?.as_str() {
            "easy" => SourceDifficulty::Easy,
            "medium" => SourceDifficulty::Medium,
            "hard" => SourceDifficulty::Hard,
            _ => return Ok(Err(SkipReason::UnknownDifficulty)),
        };
        Ok(Ok(SourceQuestion {
            id: None,
            text: decode(&self.question)?,
            correct: decode(&self.correct_answer)?,
            incorrect: self
                .incorrect_answers
                .iter()
                .map(|answer| decode(answer))
                .collect::<Result<_, _>>()?,
            difficulty,
        }))
    }
}

impl QuestionSource for OpenTriviaDb {
    fn id(&self) -> &'static str {
        "otdb"
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            name: "Open Trivia Database (PIXELTAIL GAMES LLC)".to_owned(),
            licence: "CC BY-SA 4.0".to_owned(),
            url: "https://opentdb.com".to_owned(),
        }
    }

    fn categories(&mut self) -> Result<Vec<SourceCategory>, SourceError> {
        let list: CategoryList = self.call("api_category.php")?;
        Ok(list
            .trivia_categories
            .into_iter()
            .map(|entry| SourceCategory {
                ours: ours(entry.id),
                id: entry.id.to_string(),
                name: entry.name,
            })
            .collect())
    }

    fn fetch(
        &mut self,
        category: &SourceCategory,
        found: &mut dyn FnMut(SourceQuestion),
    ) -> Result<FetchReport, SourceError> {
        let mut report = FetchReport::default();
        self.pull(Some(&category.id), &mut |result| {
            match result.decode()? {
                Ok(question) => found(question),
                Err(reason) => report.skip(reason),
            }
            Ok(())
        })?;
        Ok(report)
    }

    fn fetch_all(
        &mut self,
        categories: &[SourceCategory],
        found: &mut dyn FnMut(usize, SourceQuestion),
    ) -> Result<StreamReport, SourceError> {
        let mut report = StreamReport {
            by_category: vec![FetchReport::default(); categories.len()],
            unsorted: FetchReport::default(),
        };
        self.pull(None, &mut |result| {
            let name = decode(&result.category)?;
            let Some(index) = categories.iter().position(|c| c.name == name) else {
                report.unsorted.skip(SkipReason::UnlistedCategory);
                return Ok(());
            };
            match result.decode()? {
                Ok(question) => found(index, question),
                Err(reason) => report.by_category[index].skip(reason),
            }
            Ok(())
        })?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_maps_onto_one_of_ours() {
        for (id, ours) in OURS {
            assert!(crate::categories::name(ours).is_some(), "{id} -> {ours}");
        }
        assert_eq!(ours(28), Some("vehicles"));
        assert_eq!(ours(33), None, "a category added later is not mapped yet");
    }

    #[test]
    fn response_codes_are_typed_and_an_unknown_one_keeps_its_number() {
        let code = |code: u32| {
            let body = format!(r#"{{"response_code":{code},"results":[]}}"#);
            serde_json::from_str::<QuestionsAnswer>(&body)
                .unwrap()
                .response_code
        };
        assert_eq!(code(0), ResponseCode::Success);
        assert_eq!(code(1), ResponseCode::NoResults);
        assert_eq!(code(2), ResponseCode::InvalidParameter);
        assert_eq!(code(3), ResponseCode::TokenNotFound);
        assert_eq!(code(4), ResponseCode::TokenEmpty);
        assert_eq!(code(5), ResponseCode::RateLimited);
        assert_eq!(code(99), ResponseCode::Unknown(99));
    }
}
