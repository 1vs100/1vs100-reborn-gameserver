//! The import end to end: the OpenTriviaDB source on answers in the API's recorded shape (pacing
//! off), and the shared fitting and bank file with an in-memory source.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::Duration,
};

use onevs100_gameserver::questions::{BankFile, Difficulty, QuestionBank};
use question_import::{
    bank,
    fit::{Limits, Skip},
    source::{
        Attribution, FetchReport, QuestionSource, SkipReason, SourceCategory, SourceDifficulty,
        SourceError, SourceQuestion,
    },
    sources::otdb::{Http, OpenTriviaDb},
};

const BASE: &str = "https://opentdb.com/";
const TOKEN: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

/// Answers each URL from a queue of bodies, and logs the calls.
struct FixtureHttp {
    answers: HashMap<String, VecDeque<String>>,
    calls: Rc<RefCell<Vec<String>>>,
}

impl Http for FixtureHttp {
    fn get(&mut self, url: &str) -> Result<String, SourceError> {
        self.calls
            .borrow_mut()
            .push(url.trim_start_matches(BASE).to_owned());
        self.answers
            .get_mut(url)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| SourceError::Http {
                url: url.to_owned(),
                detail: "no answer".to_owned(),
            })
    }
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/otdb/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn page(amount: u32, token: &str) -> String {
    format!("api.php?amount={amount}&category=13&type=multiple&encode=base64&token={token}")
}

fn source(answers: Vec<(String, String)>) -> (OpenTriviaDb, Rc<RefCell<Vec<String>>>) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut queued: HashMap<String, VecDeque<String>> = HashMap::new();
    for (path, body) in answers {
        queued
            .entry(format!("{BASE}{path}"))
            .or_default()
            .push_back(body);
    }
    let http = FixtureHttp {
        answers: queued,
        calls: Rc::clone(&calls),
    };
    (OpenTriviaDb::new(Box::new(http), Duration::ZERO), calls)
}

#[test]
fn a_category_is_pulled_to_its_end_and_fitted() {
    let mut answers = vec![
        ("api_category.php".to_owned(), fixture("categories.json")),
        (
            "api_token.php?command=request".to_owned(),
            fixture("token.json"),
        ),
    ];
    for (amount, file) in [
        (50, "13-amount-50.json"),
        (25, "13-amount-25.json"),
        (12, "13-amount-12.json"),
        (6, "13-amount-6.json"),
        (3, "13-amount-3-first.json"),
        (3, "13-amount-3-second.json"),
        (1, "13-amount-1-first.json"),
        (1, "13-amount-1-second.json"),
    ] {
        answers.push((page(amount, TOKEN), fixture(file)));
    }
    let (mut otdb, calls) = source(answers);
    let mut started = Vec::new();
    let imported = bank::import(
        &mut otdb,
        Some(&["13".to_owned()]),
        Limits::default(),
        &mut |category| started.push(category.to_owned()),
    )
    .unwrap();

    // Halved on each code 4, kept at 3 while it answers, ended by code 4 at 1.
    let expected: Vec<String> = ["api_category.php", "api_token.php?command=request"]
        .into_iter()
        .map(str::to_owned)
        .chain([50, 25, 12, 6, 3, 3, 1, 1].map(|amount| page(amount, TOKEN)))
        .collect();
    assert_eq!(*calls.borrow(), expected);
    assert_eq!(started, ["fetching 1 categories one by one"]);

    let bank = imported.bank;
    let texts: Vec<&str> = bank.questions().iter().map(|q| q.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Which invented musical opens with the song \"Rise & Shine\"?",
            "In the invented operetta \"The Clockmaker\", who sings the duet?",
        ],
        "Base64 decoded: no HTML entities"
    );
    let first = &bank.questions()[0];
    assert_eq!(first.choices[first.correct.index()], "Harbour Lights");
    assert_eq!(
        first.categories,
        ["musicals-theatre"],
        "ours, not the source's"
    );
    assert_eq!(first.origin.category.as_deref(), Some("13"), "the source's");
    assert_eq!(first.origin.id, None, "OpenTriviaDB has no ids");
    assert_eq!(
        bank.questions()[1].difficulty,
        Difficulty::Medium,
        "medium is 3"
    );
    assert_eq!(bank.categories().len(), 1, "only the asked category");
    assert_eq!(bank.categories()[0].name, "Musicals & Theatre");
    let source = &bank.sources()[0];
    assert_eq!(
        (
            source.id.as_str(),
            source.licence.as_str(),
            source.url.as_str()
        ),
        ("otdb", "CC BY-SA 4.0", "https://opentdb.com")
    );

    let report = &imported.report.categories[0];
    assert_eq!((report.fetched, report.kept), (4, 2));
    assert_eq!(
        report.skipped,
        [
            (SkipReason::Fit(Skip::QuestionTooLong), 1),
            (SkipReason::Fit(Skip::NotAscii), 1)
        ]
    );
    let rendered = imported.report.render(&bank);
    assert!(
        rendered.contains("musicals-theatre Musicals & Theatre (source 13): kept 2 of 4"),
        "{rendered}"
    );
    assert!(rendered.contains("not enough for a special"), "{rendered}");
    assert!(
        rendered.contains("skipped 1: question too long"),
        "{rendered}"
    );

    // The file is one the server takes as it is.
    let file: BankFile = serde_json::from_str(&bank::to_json(&bank)).unwrap();
    assert_eq!(QuestionBank::from_file(file).unwrap(), bank);
}

#[test]
fn a_whole_import_is_one_stream_sorted_by_each_results_category() {
    let mut answers = vec![
        ("api_category.php".to_owned(), fixture("categories.json")),
        (
            "api_token.php?command=request".to_owned(),
            fixture("token.json"),
        ),
    ];
    let stream =
        |amount: u32| format!("api.php?amount={amount}&type=multiple&encode=base64&token={TOKEN}");
    for (amount, file) in [
        (50, "stream-amount-50-first.json"),
        (50, "stream-amount-50-second.json"),
        (25, "stream-amount-25.json"),
        (12, "stream-amount-12.json"),
        (6, "stream-amount-6.json"),
        (3, "stream-amount-3-first.json"),
        (3, "stream-amount-3-second.json"),
        (1, "stream-amount-1.json"),
    ] {
        answers.push((stream(amount), fixture(file)));
    }
    let (mut otdb, calls) = source(answers);
    let mut started = Vec::new();
    let imported = bank::import(&mut otdb, None, Limits::default(), &mut |what| {
        started.push(what.to_owned())
    })
    .unwrap();

    // One token, no category, and the shrinking pages at the end paid once.
    let expected: Vec<String> = ["api_category.php", "api_token.php?command=request"]
        .into_iter()
        .map(str::to_owned)
        .chain([50, 50, 25, 12, 6, 3, 3, 1].map(stream))
        .collect();
    assert_eq!(*calls.borrow(), expected);
    assert_eq!(started, ["fetching all 2 categories"]);

    let report = &imported.report;
    let general = &report.categories[0];
    assert_eq!(
        (general.id.as_str(), general.fetched, general.kept),
        ("general-knowledge", 27, 26)
    );
    assert_eq!(
        general.skipped,
        [(SkipReason::Fit(Skip::QuestionTooLong), 1)]
    );
    let musicals = &report.categories[1];
    assert_eq!(
        (musicals.id.as_str(), musicals.fetched, musicals.kept),
        ("musicals-theatre", 25, 25)
    );
    assert!(musicals.skipped.is_empty());
    assert_eq!(report.unsorted, [(SkipReason::UnlistedCategory, 1)]);

    let bank = imported.bank;
    assert_eq!(bank.questions().len(), 51);
    assert_eq!(bank.categories().len(), 2);
    let sum = |category: &str| bank.counts(category).iter().sum::<usize>();
    assert_eq!(
        (sum("general-knowledge"), sum("musicals-theatre")),
        (26, 25)
    );
    assert!(
        !bank.questions().iter().any(|q| q.text.contains("hologram")),
        "the unknown category's question is left out"
    );
    let rendered = report.render(&bank);
    assert!(
        rendered.contains("skipped 1: category not in the category list"),
        "{rendered}"
    );
}

#[test]
fn a_lost_token_is_replaced_a_rate_limit_waited_out_and_bad_parameters_reported() {
    let ok = r#"{"response_code":0,"results":[]}"#.to_owned();
    let code = |code: u32| format!(r#"{{"response_code":{code},"results":[]}}"#);
    let other = "a".repeat(64);
    let token = |token: &str| format!(r#"{{"response_code":0,"token":"{token}"}}"#);
    let (mut otdb, calls) = source(vec![
        ("api_token.php?command=request".to_owned(), token(TOKEN)),
        ("api_token.php?command=request".to_owned(), token(&other)),
        (page(50, TOKEN), code(3)),
        (page(50, &other), code(5)),
        (page(50, &other), ok),
        (page(50, &other), code(1)),
        (page(25, &other), code(2)),
    ]);
    let category = SourceCategory {
        id: "13".into(),
        name: "Musicals".into(),
        ours: Some("musicals-theatre"),
    };
    let error = otdb.fetch(&category, &mut |_| {}).unwrap_err();
    assert_eq!(error.to_string(), "category 13: response code 2");
    assert_eq!(calls.borrow().len(), 7);
    assert_eq!(calls.borrow()[3], page(50, &other), "the new token");
}

#[test]
fn a_response_code_the_api_adds_later_is_its_own_case() {
    let token = format!(r#"{{"response_code":0,"token":"{TOKEN}"}}"#);
    let unknown = r#"{"response_code":99,"results":[]}"#.to_owned();
    let (mut otdb, _calls) = source(vec![
        ("api_token.php?command=request".to_owned(), token),
        (page(50, TOKEN), unknown),
    ]);
    let category = SourceCategory {
        id: "13".into(),
        name: "Musicals".into(),
        ours: Some("musicals-theatre"),
    };
    let error = otdb.fetch(&category, &mut |_| {}).unwrap_err();
    assert_eq!(error.to_string(), "category 13: response code 99");
    assert!(
        matches!(error, SourceError::Response { code: 99, .. }),
        "{error:?}"
    );
}

/// Questions handed over as given, two categories.
struct FixtureSource;

impl QuestionSource for FixtureSource {
    fn id(&self) -> &'static str {
        "fixture"
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            name: "Fixture".into(),
            licence: "none".into(),
            url: String::new(),
        }
    }

    fn categories(&mut self) -> Result<Vec<SourceCategory>, SourceError> {
        Ok(["1", "2"]
            .map(|id| SourceCategory {
                ours: (id == "1").then_some("music"),
                id: id.into(),
                name: format!("Category {id}"),
            })
            .to_vec())
    }

    fn fetch(
        &mut self,
        category: &SourceCategory,
        found: &mut dyn FnMut(SourceQuestion),
    ) -> Result<FetchReport, SourceError> {
        let question = |text: &str, difficulty| SourceQuestion {
            id: Some(format!("{}-{text}", category.id)),
            text: text.into(),
            correct: "Yes".into(),
            incorrect: vec!["No".into(), "Maybe".into()],
            difficulty,
        };
        found(question("Shared question?", SourceDifficulty::Easy));
        found(question(
            &format!("Only in {}?", category.id),
            SourceDifficulty::Title(5),
        ));
        let mut report = FetchReport::default();
        report.skip(SkipReason::NotMultipleChoice);
        Ok(report)
    }
}

#[test]
fn every_category_is_imported_and_a_shared_question_kept_once() {
    let imported = bank::import(&mut FixtureSource, None, Limits::default(), &mut |_| {}).unwrap();
    let bank = imported.bank;
    let shared: Vec<_> = bank
        .questions()
        .iter()
        .filter(|q| q.text == "Shared question?")
        .collect();
    assert_eq!(shared.len(), 1);
    assert_eq!(
        shared[0].categories,
        ["music", "fixture-2"],
        "mapped, and one of its own"
    );
    assert_eq!(shared[0].origin.category.as_deref(), Some("1"));
    assert_eq!(shared[0].origin.id.as_deref(), Some("1-Shared question?"));
    assert_eq!(bank.categories()[0].name, "Music", "our name");
    assert_eq!(bank.categories()[1].name, "Category 2", "the source's name");
    assert!(!imported.report.categories[0].unmapped);
    assert!(imported.report.categories[1].unmapped);
    let rendered = imported.report.render(&bank);
    assert!(
        rendered.contains("fixture-2 Category 2 (source 2) (not mapped"),
        "{rendered}"
    );
    assert!(
        rendered.contains("    skipped 1: not multiple choice\n    skipped 1: duplicate\n"),
        "the report still prints the reasons: {rendered}"
    );
    assert_eq!(bank.questions().len(), 3);
    assert_eq!(bank.counts("fixture-2")[4], 1, "the source's own 5");
    let second = &imported.report.categories[1];
    assert_eq!((second.fetched, second.kept), (2, 1));
    assert_eq!(
        second.skipped,
        [
            (SkipReason::NotMultipleChoice, 1),
            (SkipReason::Fit(Skip::Duplicate), 1)
        ]
    );

    let unknown = bank::import(
        &mut FixtureSource,
        Some(&["3".to_owned()]),
        Limits::default(),
        &mut |_| {},
    )
    .err()
    .expect("no category 3");
    assert!(
        matches!(
            unknown,
            SourceError::UnknownCategory {
                source: "fixture",
                ref category,
            } if category == "3"
        ),
        "{unknown:?}"
    );
}
