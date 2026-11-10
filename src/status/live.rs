//! What is on now: players online, the current and next show, and the current scores.

use std::time::SystemTime;

use super::{
    GameType, LiveInfo, Progress, ShowSummary, ShowType, StandingInfo, Standings, StandingsScope,
    ranking::{Gamertag, Points, rank},
};
use crate::{
    app::App,
    gamestate::GameStateDoc,
    ids::QuestionNumber,
    programme::{PlannedShow, ShowKind},
    sbg::key::{Board, Key, ParsedKey},
    schedule::{Mode, Showing},
    store::Store,
    sync::lock,
    time::Utc,
};

/// Entries in [`Standings`].
const STANDINGS: usize = 10;

pub fn snapshot(app: &App, now: SystemTime) -> LiveInfo {
    let population = lock(&app.members).total_population();
    let (shows, active) = {
        let mut schedule = lock(&app.schedule);
        // Without a programme the title is told of a show that always starts soon; that is not
        // a show to announce.
        let shows = schedule
            .programme()
            .is_some()
            .then(|| schedule.showing_pair(now, 0));
        (shows, schedule.active_session().map(str::to_owned))
    };
    let programmed = shows.is_some();
    let (current, next) = match shows {
        Some([first, second]) if first.start() <= now => (Some(first), Some(second)),
        Some([first, _]) => (None, Some(first)),
        None => (None, None),
    };
    // The store holds the state of the show a console last started. With a programme that is the
    // current show's only while its session is the active one.
    let mode = match (programmed, &current, &active) {
        // The title plays the default game type when it is told of no programme.
        (false, _, _) => Some(Mode::default()),
        (true, Some(Showing::Scheduled(show)), Some(active))
            if show.session_id.eq_ignore_ascii_case(active) =>
        {
            Some(show.plan.mode())
        }
        _ => None,
    }
    .filter(|_| population.consoles > 0);
    let store = lock(&app.store);
    LiveInfo {
        consoles: population.consoles,
        players: population.players,
        current: current.as_ref().map(|show| summary(show, &store)),
        next: next.as_ref().map(|show| summary(show, &store)),
        progress: mode.and_then(|_| progress(&store)),
        standings: mode.and_then(|mode| standings(&store, mode)),
    }
}

fn summary(show: &Showing, store: &Store) -> ShowSummary {
    match show {
        Showing::Scheduled(show) => planned_summary(&show.plan, store),
        Showing::Unscheduled { start, end, .. } => build_summary(None, show.mode(), *start, *end),
    }
}

pub(super) fn planned_summary(show: &PlannedShow, store: &Store) -> ShowSummary {
    let special = match &show.kind {
        ShowKind::Special {
            title, category, ..
        } => Some((title.clone(), category_name(store, category))),
        ShowKind::Primetime | ShowKind::ExtendedPlay => None,
    };
    build_summary(special, show.mode(), show.start.into(), show.end.into())
}

fn build_summary(
    special: Option<(String, String)>,
    mode: Mode,
    start: SystemTime,
    end: SystemTime,
) -> ShowSummary {
    let game_type = match mode {
        Mode::Primetime => GameType::Primetime,
        Mode::ExtendedPlay => GameType::ExtendedPlay,
    };
    let show_type = match (&special, game_type) {
        (Some(_), _) => ShowType::Special,
        (None, GameType::Primetime) => ShowType::Primetime,
        (None, GameType::ExtendedPlay) => ShowType::ExtendedPlay,
    };
    let (title, topic) = special.unzip();
    ShowSummary {
        show_type,
        game_type,
        title,
        topic,
        language: language(crate::questions::LOCALE),
        start_utc: Utc::from(start).to_grid(),
        end_utc: Utc::from(end).to_grid(),
    }
}

/// The bank's name for a category id; the id itself for a category the bank does not have.
fn category_name(store: &Store, id: &str) -> String {
    store
        .questions()
        .categories()
        .iter()
        .find(|category| category.id == id)
        .map_or_else(|| id.to_owned(), |category| category.name.clone())
}

/// `en-us` as BCP 47 writes it, `en-US`.
fn language(locale: &str) -> String {
    match locale.split_once('-') {
        Some((language, region)) => format!("{language}-{}", region.to_ascii_uppercase()),
        None => locale.to_owned(),
    }
}

/// From the latest GameState the show's consoles posted.
fn progress(store: &Store) -> Option<Progress> {
    store
        .keys()
        .into_iter()
        .filter(|(key, _)| matches!(ParsedKey::parse(key).kind, Key::GameState { .. }))
        .filter_map(|(key, _)| GameStateDoc::parse(store.get(key).ok()?))
        .max_by_key(GameStateDoc::start)
        .map(|doc| Progress {
            round: doc.round(),
            question: doc.reported_question(),
        })
}

/// The latest round score board of the show's game type. The title posts to it only players
/// whose privacy setting lets them be named.
fn standings(store: &Store, mode: Mode) -> Option<Standings> {
    let (scope, round, question, board) = store
        .posted_top_ns()
        .filter_map(|(key, board)| {
            let (scope, round, question) = match (ParsedKey::parse(key).kind, mode) {
                (
                    Key::QuestionBoard {
                        round,
                        question,
                        board: Board::RoundScores,
                        ..
                    },
                    Mode::Primetime,
                ) => (StandingsScope::Round, Some(round), question),
                (Key::ExtendedPlayRoundScores { question, .. }, Mode::ExtendedPlay) => {
                    (StandingsScope::Show, None, question)
                }
                _ => return None,
            };
            Some((scope, round, question, board))
        })
        .max_by_key(|(_, round, question, _)| (*round, *question))?;
    let players = board
        .pairs()
        .into_iter()
        .filter_map(|(score, metadata)| Some((gamertag(&metadata)?, Points::from_score(score)?)))
        .collect();
    Some(Standings {
        scope,
        round,
        // The console writes 0 for a board posted before the round's first question.
        question: question.map_or(0, QuestionNumber::get),
        entries: rank(players)
            .iter()
            .take(STANDINGS)
            .map(StandingInfo::from)
            .collect(),
    })
}

/// `<Metadata>gamertag</Metadata>`, as the title posts a round score.
fn gamertag(metadata: &[u8]) -> Option<Gamertag> {
    let node = crate::xsor::table::parse(metadata)?;
    (node.name() == "Metadata").then_some(())?;
    Gamertag::new(&node.text_value()?)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        ids::{Round, Xuid},
        programme::Programme,
        storage::Storage,
    };

    const SESSION: &str = "00000000-0000-0000-0000-000000000001";

    fn app(programme: Option<Programme>) -> (App, Utc) {
        let storage: Arc<dyn Storage> = Arc::new(crate::storage::MemoryStorage::default());
        if let Some(programme) = programme {
            storage.set_programme(&programme).unwrap();
        }
        let schedule =
            crate::schedule::Schedule::load(Default::default(), Arc::clone(&storage)).unwrap();
        let app = App::new(
            crate::sbg::Settings::new(crate::wire::Protocol::V1317),
            Arc::new(std::sync::Mutex::new(schedule)),
            Store::bootstrap().with_storage(storage),
            crate::matchmaking::Members::default(),
        );
        (app, Utc::parse("2026-10-27T12:00:00Z").unwrap())
    }

    fn question(number: u32) -> QuestionNumber {
        QuestionNumber::new(number).expect("a question number is not 0")
    }

    fn post(app: &App, key: &str, score: f64, gamertag: &str) {
        let metadata = format!("<Metadata>{gamertag}</Metadata>").into_bytes();
        assert!(lock(&app.store).top_n_post(key, 100, score, metadata));
    }

    fn round_scores(round: Round, question: QuestionNumber) -> String {
        format!(
            "v3.Session.MGS.1vs100.TopN.{SESSION}.PRIMETIME.Round.{round}.Question.{question}.RoundScores"
        )
    }

    #[test]
    fn an_empty_server_lists_the_shows_only() {
        let (app, now) = app(Some(Programme::default()));
        insta::assert_json_snapshot!(snapshot(&app, now.into()));
    }

    #[test]
    fn the_latest_board_of_the_running_game_is_ranked() {
        let (app, _) = app(Some(Programme::default()));
        // During the default programme's Primetime show, 20:00 in New York.
        let now = Utc::parse("2026-10-28T00:10:00Z").unwrap();
        let running = lock(&app.schedule).current_session_id(now.into());
        let mut schedule = lock(&app.schedule);
        schedule.activate_session(now.into(), &running);
        assert_eq!(schedule.active_session(), Some(running.as_str()));
        drop(schedule);
        lock(&app.members).insert_test_console(
            SESSION,
            Xuid(0x0009_0000_0000_0001),
            &["PlayerOne", "PlayerTwo"],
        );
        post(
            &app,
            &round_scores(Round(1), question(3)),
            9_000.0,
            "Earlier",
        );
        post(
            &app,
            &format!("v3.Season.MGS.1vs100.TopN.{SESSION}.EXTENDEDPLAY.Question.20.RoundScores"),
            9_000.0,
            "OtherGameType",
        );
        for (score, gamertag) in [
            (676.625, "PlayerSlim"),
            (1_558.665, "PlayerPhat"),
            (1_558.2, "Tied"),
            (-1.0, "Negative"),
            (2_000.0, "Not A Gamertag!"),
        ] {
            post(&app, &round_scores(Round(2), question(1)), score, gamertag);
        }
        lock(&app.store).top_n_post(
            &round_scores(Round(2), question(1)),
            100,
            5.0,
            b"<Metadata type=\"table\"><xuid>7</xuid></Metadata>".to_vec(),
        );
        game_state(&app, Round(2), question(1));
        insta::assert_json_snapshot!(snapshot(&app, now.into()));
    }

    fn game_state(app: &App, round: Round, question: QuestionNumber) {
        lock(&app.store).set(
            crate::sbg::key::game_state_key(SESSION),
            format!(
                "<XsorObject><name>QA</name><data type=\"table\"><RoundNumber type=\"float\">\
                 {round}</RoundNumber><QuestionNumber type=\"float\">{question}</QuestionNumber>\
                 </data></XsorObject>"
            )
            .into_bytes(),
        );
    }

    #[test]
    fn a_finished_shows_state_is_not_reported() {
        let (app, _) = app(Some(Programme::default()));
        let started = Utc::parse("2026-10-28T00:10:00Z").unwrap();
        let session = lock(&app.schedule).current_session_id(started.into());
        lock(&app.schedule).activate_session(started.into(), &session);
        lock(&app.members).insert_test_console(SESSION, Xuid(1), &["PlayerOne"]);
        post(
            &app,
            &round_scores(Round(1), question(1)),
            10.0,
            "PlayerOne",
        );
        game_state(&app, Round(1), question(1));
        assert!(snapshot(&app, started.into()).progress.is_some());
        // The Primetime show is over; no console has started the Extended Play show after it.
        let later = Utc::parse("2026-10-28T01:10:00Z").unwrap();
        let live = snapshot(&app, later.into());
        assert_eq!((live.progress, live.standings), (None, None));
    }

    #[test]
    fn without_a_programme_a_running_game_is_reported_without_shows() {
        let (app, now) = app(None);
        post(
            &app,
            &round_scores(Round(1), question(1)),
            10.0,
            "PlayerOne",
        );
        game_state(&app, Round(1), question(1));
        assert!(snapshot(&app, now.into()).progress.is_none(), "no console");
        lock(&app.members).insert_test_console(SESSION, Xuid(1), &["PlayerOne"]);
        let live = snapshot(&app, now.into());
        assert_eq!((live.current, live.next), (None, None));
        assert_eq!(
            live.progress,
            Some(Progress {
                round: Some(Round(1)),
                question: Some(1)
            })
        );
        assert_eq!(
            live.standings.map(|standings| standings.entries.len()),
            Some(1)
        );
    }

    #[test]
    fn progress_keeps_the_consoles_own_question_number() {
        let (app, now) = app(None);
        lock(&app.members).insert_test_console(SESSION, Xuid(1), &["PlayerOne"]);
        game_state(&app, Round(1), question(1));
        // A GameState from before the round's first question reports 0, as the console writes it.
        let key = crate::sbg::key::game_state_key(SESSION);
        lock(&app.store).set(
            key,
            "<XsorObject><name>Introductions</name><data type=\"table\">\
             <RoundNumber type=\"float\">1</RoundNumber>\
             <QuestionNumber type=\"float\">0</QuestionNumber>\
             </data></XsorObject>"
                .as_bytes()
                .to_vec(),
        );
        let progress = snapshot(&app, now.into()).progress.unwrap();
        assert_eq!(
            (progress.round, progress.question),
            (Some(Round(1)), Some(0))
        );
    }

    #[test]
    fn locales_are_written_as_bcp_47() {
        assert_eq!(language("en-us"), "en-US");
        assert_eq!(language("de"), "de");
    }
}
