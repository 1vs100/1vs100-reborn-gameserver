use std::{collections::HashMap, sync::Arc};

use crate::{
    gamestate::GameStateDoc,
    questions::{QuestionBank, ShowSeed},
    sbg::key::{BankEntry, ElectionItem, Key, ParsedKey, QuestionList},
    show::Show,
    simulation::{SimulatedReads, is_canonical_election},
    time::{Clock, SystemClock, Utc},
    xsor,
};

/// Canonical session's GameState key in posted casing.
fn captured_game_state_key() -> String {
    crate::sbg::key::game_state_key(crate::schedule::CANONICAL_SESSION)
}

/// Opt-in test behaviours, all off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimulationOptions {
    pub synthetic_mob: bool,
    /// Simulated crowd members in Crowd choice sums (0 = none).
    pub synthetic_crowd: u32,
}

/// Persistent data home; memory unless started with a database.
pub struct StorageHandle(std::sync::Arc<dyn crate::storage::Storage>);

impl Default for StorageHandle {
    fn default() -> Self {
        Self(std::sync::Arc::new(crate::storage::MemoryStorage::default()))
    }
}

/// Question bank served; empty serves as built-in.
#[derive(Clone)]
pub struct QuestionsHandle(Arc<QuestionBank>);

impl QuestionsHandle {
    fn new(bank: Arc<QuestionBank>) -> Self {
        if bank.is_empty() {
            Self::default()
        } else {
            Self(bank)
        }
    }
}

impl Default for QuestionsHandle {
    fn default() -> Self {
        Self(Arc::new(QuestionBank::seed()))
    }
}

/// Store clock; system unless started with another (a replay).
pub struct ClockHandle(Arc<dyn Clock>);

impl Default for ClockHandle {
    fn default() -> Self {
        Self(Arc::new(SystemClock))
    }
}

#[derive(Default)]
pub struct Store {
    storage: StorageHandle,
    clock: ClockHandle,
    questions: QuestionsHandle,
    options: SimulationOptions,
    served_config: xsor::ServedConfig,
    show: Show,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GetError {
    NotFound,
}

impl Store {
    /// Seed setup documents; Config and bank generate per request, so they are not seeded.
    pub fn bootstrap() -> Self {
        Self {
            show: setup_show(),
            ..Self::default()
        }
    }

    /// Persist player documents and global counters through `storage`.
    pub fn with_storage(mut self, storage: std::sync::Arc<dyn crate::storage::Storage>) -> Self {
        self.storage = StorageHandle(storage);
        self
    }

    /// Serve `config` in the Config document for a replay.
    pub fn with_served_config(mut self, config: xsor::ServedConfig) -> Self {
        self.served_config = config;
        self
    }

    pub fn served_config(&self) -> xsor::ServedConfig {
        self.served_config
    }

    /// Read time from `clock`.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = ClockHandle(clock);
        self
    }

    pub fn now(&self) -> Utc {
        self.clock.0.now()
    }

    /// Store clock, for waits outside the store lock.
    pub fn clock(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.clock.0)
    }

    /// Storage handle; clone out of the store lock before I/O.
    pub fn storage(&self) -> std::sync::Arc<dyn crate::storage::Storage> {
        std::sync::Arc::clone(&self.storage.0)
    }

    /// New show: drop session state and re-seed fixtures; storage, options, served config stay.
    pub fn reset_session(&mut self) {
        self.show = setup_show();
    }

    /// Serve the bank in storage, else the built-in one.
    pub fn with_questions(mut self, bank: Arc<QuestionBank>) -> Self {
        self.questions = QuestionsHandle::new(bank);
        self
    }

    /// Serve `bank` from now on.
    pub fn set_questions(&mut self, bank: Arc<QuestionBank>) {
        self.questions = QuestionsHandle::new(bank);
    }

    pub fn questions(&self) -> Arc<QuestionBank> {
        Arc::clone(&self.questions.0)
    }

    /// Last GameState posted per key.
    pub(crate) fn last_posted_mut(
        &mut self,
    ) -> &mut HashMap<String, crate::gamestate::flow::StateDoc> {
        &mut self.show.last_posted
    }

    pub fn options(&self) -> SimulationOptions {
        self.options
    }

    pub fn bootstrap_with(options: SimulationOptions) -> Self {
        Self {
            options,
            ..Self::bootstrap()
        }
    }

    pub fn get(&self, key: &str) -> Result<&[u8], GetError> {
        self.show.value(key).ok_or(GetError::NotFound)
    }

    /// Answer for a question id list key: what the title stored, else the bank's list for `show`,
    /// filtered to `category` for a special.
    pub fn question_list_get(
        &self,
        key: &ParsedKey,
        category: Option<&str>,
        show: ShowSeed,
    ) -> Option<Vec<u8>> {
        // The title writes both lists back as it picks; the stored form answers until it has.
        if matches!(key.kind, Key::SessionQuestionList { .. })
            && let Some(stored) = self.show.value(&key.raw)
        {
            if let Some(served) = xsor::stored_question_list(stored) {
                return Some(served);
            }
            tracing::warn!(key = %key.raw, "stored question list unreadable: the bank's is served");
        }
        match key.kind.clone() {
            Key::SessionQuestionList {
                session,
                difficulty,
                list,
            } if session.is_canonical() && (1..=5).contains(&difficulty) => match list {
                QuestionList::Available => xsor::question_bank_available(
                    &self.questions.0,
                    difficulty.into(),
                    category,
                    show,
                ),
                QuestionList::Used => xsor::question_bank_used(difficulty.into()),
            },
            Key::QuestionBank {
                locale,
                difficulty,
                entry: BankEntry::ResetId,
            } if serves_locale(locale.as_deref()) => {
                xsor::question_reset_ids(&self.questions.0, difficulty.into(), category, show)
            }
            _ => None,
        }
    }

    /// Answer for a single question key.
    pub fn question_get(&self, key: &ParsedKey) -> Option<Vec<u8>> {
        match key.kind {
            Key::QuestionBank {
                ref locale,
                difficulty,
                entry: BankEntry::Id(id),
            } if serves_locale(locale.as_deref()) && (1..=5).contains(&difficulty) => {
                xsor::question_payload(&self.questions.0, id)
            }
            _ => None,
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: Vec<u8>) -> bool {
        let key = key.into();
        let parsed = ParsedKey::parse(&key);
        let captured = matches!(&parsed.kind, Key::GameState { session } if session.is_canonical());
        // For the captured session, reject stale writes; either casing may hold the current value.
        if captured
            && let Some(current) = self
                .show
                .value(&key)
                .or_else(|| self.show.value(&captured_game_state_key()))
            && crate::gamestate::regresses(current, &value)
        {
            return false;
        }
        if let Key::GameState { session } = &parsed.kind {
            crate::timeline::record(
                "gamestate",
                describe_game_state(&value),
                Some(session.as_str()),
            );
        }
        if captured
            && let Some(doc) = GameStateDoc::parse(&value)
            && doc.state() == Some(crate::gamestate::GameStateName::Qa)
            && let (Some(round), Some(question), Some(id)) =
                (doc.round(), doc.question(), doc.question_id())
        {
            self.show.record_asked(round, question, id);
        }
        self.show.set_value(key, value);
        true
    }

    /// Every plain stored key with its byte length, sorted.
    pub fn keys(&self) -> Vec<(&str, usize)> {
        self.show.keys()
    }

    pub fn remove(&mut self, key: &str) -> bool {
        self.show.remove_value(key)
    }

    pub fn sum_add(&mut self, key: &str, delta: f64) -> bool {
        self.show.sum_add(key, delta)
    }

    /// Consoles' view: show plus simulated Mob and Crowd per options.
    fn reads(&self) -> SimulatedReads<'_> {
        SimulatedReads::new(&self.show, self.options, &self.questions.0)
    }

    pub fn sum_get(&self, parsed: &ParsedKey) -> Option<f64> {
        self.reads().sum(parsed)
    }

    pub fn top_n_post(
        &mut self,
        key: &str,
        capacity: usize,
        score: f64,
        metadata: Vec<u8>,
    ) -> bool {
        self.show.top_n_post(key, capacity, score, metadata)
    }

    /// SBG read path: first non-empty TheOne read fixes its winner.
    pub fn top_n_read(&mut self, key: &ParsedKey) -> Option<Vec<(f64, Vec<u8>)>> {
        let entries = self.top_n_get(key)?;
        if !entries.is_empty() && is_canonical_election(key, ElectionItem::TheOne) {
            self.show.elections.freeze(&key.raw);
        }
        Some(entries)
    }

    /// Fold an election key's id into its epoch; `candidacy` Top1 posts may open or alias one.
    pub fn fold_election(&mut self, key: ParsedKey, candidacy: bool, now: Utc) -> ParsedKey {
        self.show.elections.fold(key, candidacy, now)
    }

    pub fn top_n_get(&self, parsed: &ParsedKey) -> Option<Vec<(f64, Vec<u8>)>> {
        self.reads().top_n(parsed)
    }

    /// The show's boards as the consoles posted them, without simulated entries.
    pub fn posted_top_ns(&self) -> impl Iterator<Item = (&str, &crate::show::TopN)> {
        self.show.top_ns()
    }
}

/// Setup-only show [`Store::bootstrap`] seeds.
fn setup_show() -> Show {
    let mut show = Show::default();
    let prizing = xsor::prizing_matrix_fixture();
    for week in 1..=52 {
        show.set_value(
            format!("v3.Setup.MGS.1vs100.Store.Config.Week{week}"),
            prizing.clone(),
        );
    }
    show.set_value(
        format!(
            "v3.Setup.MGS.1vs100.Store.{}.AvailableLanguages",
            crate::schedule::CANONICAL_EPISODE
        ),
        xsor::available_languages_fixture(),
    );
    show
}

fn describe_game_state(value: &[u8]) -> String {
    let doc = GameStateDoc::parse(value);
    let doc = doc.as_ref();
    let show = |n: Option<String>| n.unwrap_or_else(|| "?".to_owned());
    format!(
        "GameState {} (round {}, question {})",
        doc.map_or("?".into(), GameStateDoc::name),
        show(
            doc.and_then(GameStateDoc::round)
                .map(|round| round.to_string())
        ),
        show(
            doc.and_then(GameStateDoc::question)
                .map(|question| question.to_string())
        ),
    )
}

/// The bank serves en-US only; the short key form names no locale.
fn serves_locale(locale: Option<&str>) -> bool {
    locale.is_none_or(|locale| locale == "en-us")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{
        ids::{QuestionNumber, Round},
        questions::{Choice, Difficulty},
        sbg::key::Board,
    };

    #[test]
    fn opt_in_store_options_default_off() {
        let default = Store::bootstrap();
        assert!(!default.options.synthetic_mob);

        let configured = Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        });
        assert!(configured.options.synthetic_mob);
    }

    #[test]
    fn synthetic_mob_leaves_themob_to_the_console() {
        // The simulated mob only adds TopN and Sum entries: TheMob is whatever the title SETs, and
        // reads as missing until then.
        const KEY: &str =
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.387.TheMob";
        let store = Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        });

        assert_eq!(store.get(KEY), Err(GetError::NotFound));
    }

    #[test]
    fn missing_is_not_empty_and_set_overwrites() {
        let mut store = Store::default();
        assert_eq!(store.get("GameState"), Err(GetError::NotFound));
        store.set("GameState", b"old".to_vec());
        store.set("GameState", b"new".to_vec());
        assert_eq!(store.get("GameState"), Ok(&b"new"[..]));
    }

    #[test]
    fn captured_game_state_ignores_stale_round_and_accepts_current_qa() {
        const KEY: &str =
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.GameState";
        const OTHER_SESSION: &str =
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000002.GameState";
        let intro2_xml = include_bytes!("../fixtures/native-r15-round2-introductions.xml").to_vec();
        let mut qa2 = crate::gamestate::recorded("QA", true);
        qa2.set_field("RoundNumber", 2);
        qa2.set_field("QuestionNumber", 1);
        let qa2 = qa2.to_xml();
        let stale_qa1 = include_bytes!("../fixtures/native-r12-qa-251317.xml").to_vec();

        let mut store = Store::bootstrap();
        assert!(store.set(KEY, intro2_xml.clone()));
        assert!(!store.set(KEY, stale_qa1.clone()));
        assert_eq!(store.get(KEY), Ok(&intro2_xml[..]));
        assert!(store.set(OTHER_SESSION, stale_qa1.clone()));
        assert_eq!(store.get(OTHER_SESSION), Ok(&stale_qa1[..]));
        assert!(store.set(KEY, qa2.clone()));
        assert_eq!(store.get(KEY), Ok(&qa2[..]));
        let malformed = b"not a GameState".to_vec();
        assert!(store.set(KEY, malformed.clone()));
        assert_eq!(store.get(KEY), Ok(&malformed[..]));
    }

    #[test]
    fn synthetic_mob_store_augments_only_existing_mob_and_player_count_data() {
        const MOB_KEY: &str = "V3.sEssion.MgS.1vs100.tOPn.00000000-0000-0000-0000-000000000001.pRImEtImE.693.mOBSELECTION";
        const PLAYER_COUNT_KEY: &str = "v3.sESSION.mgs.1vs100.sUM.00000000-0000-0000-0000-000000000001.pRIMETIME.693.rstotalplayercount";
        const ONE_KEY: &str =
            "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.693.TheOne";
        const QOS_KEY: &str =
            "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.693.QOS";
        const OTHER_TITLE_MOB_KEY: &str = "v3.Session.MGS.1vs200.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.693.MobSelection";
        const OTHER_TITLE_COUNT_KEY: &str = "v3.Session.MGS.1vs200.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.693.RSTotalPlayerCount";
        const OTHER_SESSION_MOB_KEY: &str = "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000002.PRIMETIME.693.MobSelection";
        const OTHER_SESSION_COUNT_KEY: &str = "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000002.PRIMETIME.693.RSTotalPlayerCount";
        const MISSING_MOB_KEY: &str = "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.694.MobSelection";
        const MISSING_COUNT_KEY: &str = "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.694.RSTotalPlayerCount";

        let mut store = Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        });
        let real = b"<Metadata>e00007ff484d4b37</Metadata>".to_vec();
        assert!(store.top_n_post(MOB_KEY, 101, 0.6743096, real.clone()));
        assert!(store.sum_add(PLAYER_COUNT_KEY, 1.0));
        assert!(store.top_n_post(
            ONE_KEY,
            101,
            0.6743096,
            b"<Metadata>one</Metadata>".to_vec()
        ));
        assert!(store.sum_add(QOS_KEY, 1.0));
        assert!(store.top_n_post(
            OTHER_TITLE_MOB_KEY,
            101,
            0.6743096,
            b"<Metadata>other-title</Metadata>".to_vec(),
        ));
        assert!(store.sum_add(OTHER_TITLE_COUNT_KEY, 1.0));
        assert!(store.top_n_post(
            OTHER_SESSION_MOB_KEY,
            101,
            0.6743096,
            b"<Metadata>other-session</Metadata>".to_vec(),
        ));
        assert!(store.sum_add(OTHER_SESSION_COUNT_KEY, 1.0));

        let entries = store.top_n_get(&ParsedKey::parse(MOB_KEY)).unwrap();
        assert_eq!(entries.len(), 101);
        assert_eq!(entries[0], (0.6743096, real));
        let metadata = entries
            .iter()
            .map(|(_, metadata)| metadata.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(metadata.len(), 101);
        for xuid in 0xF000_0000_0000_0001u64..=0xF000_0000_0000_0064u64 {
            let expected = format!("<Metadata>{xuid:016x}</Metadata>").into_bytes();
            assert!(metadata.contains(&expected));
        }

        let stored_before = store.show.top_n(MOB_KEY).unwrap().pairs();
        assert_eq!(
            store.top_n_get(&ParsedKey::parse(MOB_KEY)).unwrap(),
            entries
        );
        let stored_after = store.show.top_n(MOB_KEY).unwrap().pairs();
        assert_eq!(stored_after, stored_before);
        assert_eq!(stored_after.len(), 1);
        assert_eq!(
            store.sum_get(&ParsedKey::parse(PLAYER_COUNT_KEY)),
            Some(102.0)
        );
        assert_eq!(store.show.sum(PLAYER_COUNT_KEY), Some(1.0));

        assert_eq!(
            store.top_n_get(&ParsedKey::parse(ONE_KEY)).unwrap().len(),
            1
        );
        assert_eq!(store.sum_get(&ParsedKey::parse(QOS_KEY)), Some(1.0));
        assert_eq!(
            store
                .top_n_get(&ParsedKey::parse(OTHER_TITLE_MOB_KEY))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store.sum_get(&ParsedKey::parse(OTHER_TITLE_COUNT_KEY)),
            Some(1.0)
        );
        assert_eq!(
            store
                .top_n_get(&ParsedKey::parse(OTHER_SESSION_MOB_KEY))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store.sum_get(&ParsedKey::parse(OTHER_SESSION_COUNT_KEY)),
            Some(1.0)
        );
        assert_eq!(store.top_n_get(&ParsedKey::parse(MISSING_MOB_KEY)), None);
        assert_eq!(store.sum_get(&ParsedKey::parse(MISSING_COUNT_KEY)), None);
        assert!(store.show.top_n(MISSING_MOB_KEY).is_none());
        assert!(store.show.sum(MISSING_COUNT_KEY).is_none());

        let mut limited = Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        });
        let limited_key = "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.695.MobSelection";
        let limited_real = b"<Metadata>limited-real</Metadata>".to_vec();
        assert!(limited.top_n_post(limited_key, 3, 0.6743096, limited_real.clone()));
        let limited_entries = limited.top_n_get(&ParsedKey::parse(limited_key)).unwrap();
        assert_eq!(limited_entries.len(), 3);
        assert_eq!(limited_entries[0].1, limited_real);

        let mut ordinary = Store::bootstrap();
        assert!(ordinary.top_n_post(
            MOB_KEY,
            101,
            0.6743096,
            b"<Metadata>ordinary</Metadata>".to_vec()
        ));
        assert!(ordinary.sum_add(PLAYER_COUNT_KEY, 1.0));
        assert_eq!(
            ordinary
                .top_n_get(&ParsedKey::parse(MOB_KEY))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            ordinary.sum_get(&ParsedKey::parse(PLAYER_COUNT_KEY)),
            Some(1.0)
        );
    }

    #[test]
    fn the_simulated_mob_fills_the_stat_break_buckets_on_top_of_real_posts() {
        let key = |bucket: u32| {
            format!(
                "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.StatBreaks.NumCorrectAnswers.Bucket{bucket}"
            )
        };
        let off = Store::bootstrap();
        assert_eq!(
            off.sum_get(&ParsedKey::parse(&key(5))),
            None,
            "only with the simulated Mob"
        );

        let mut store = Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        });
        let simulated = crate::primetime::synthetic_stat_break(
            Round(1),
            QuestionNumber::FIRST,
            QuestionBank::seed().fallback().unwrap().difficulty,
            crate::primetime::StatBreak::NumCorrectAnswers,
        );
        let read = |store: &Store| {
            (1..=5)
                .map(|b| store.sum_get(&ParsedKey::parse(&key(b))))
                .collect::<Vec<_>>()
        };
        // After question 1 everybody is in bucket 1 (wrong) or 5 (right): 2-4 stay NOT_FOUND.
        assert_eq!(
            read(&store),
            [
                Some(f64::from(simulated[0])),
                None,
                None,
                None,
                Some(f64::from(simulated[4]))
            ]
        );
        // The One posts its own entry; it is added on top.
        assert!(store.sum_add(&key(5), 1.0));
        assert_eq!(
            store.sum_get(&ParsedKey::parse(&key(5))),
            Some(f64::from(simulated[4]) + 1.0)
        );
        assert!(store.sum_add(&key(3), 1.0));
        assert_eq!(store.sum_get(&ParsedKey::parse(&key(3))), Some(1.0));
        // Not a stat-break bucket: unchanged.
        assert_eq!(store.sum_get(&ParsedKey::parse(&key(6))), None);
    }

    #[test]
    fn the_simulated_mob_posts_a_brain_for_trust_the_brain() {
        const KEY: &str = "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.2.Brain";
        assert_eq!(
            Store::bootstrap().top_n_get(&ParsedKey::parse(KEY)),
            None,
            "only with the simulated Mob"
        );

        let mut store = Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        });
        // Question 2 asks 251401 (correct choice 2).
        store.set(
            captured_game_state_key(),
            b"<XsorObject><name>QA</name><data type=\"table\"><RoundNumber type=\"float\">1</RoundNumber>\
              <QuestionNumber type=\"float\">2</QuestionNumber><QuestionInfo type=\"table\">\
              <difficulty type=\"float\">1</difficulty><questionID type=\"float\">251401</questionID>\
              </QuestionInfo></data></XsorObject>"
                .to_vec(),
        );
        let board = store.top_n_get(&ParsedKey::parse(KEY)).unwrap();
        assert_eq!(
            board,
            crate::primetime::synthetic_brain(
                Round(1),
                QuestionNumber::new(2).unwrap(),
                Difficulty::from_level(1),
                Choice::B
            )
        );
        assert_eq!(board.len(), 10);
        // A real player with a higher round score takes a place on the board.
        let real =
            b"<Metadata type=\"table\"><xuid>7</xuid><choice type=\"float\">1</choice></Metadata>"
                .to_vec();
        assert!(store.top_n_post(KEY, 10, 99_999.0, real.clone()));
        let board = store.top_n_get(&ParsedKey::parse(KEY)).unwrap();
        assert_eq!(board.len(), 10);
        assert_eq!(board[0], (99_999.0, real));
    }

    #[test]
    fn synthetic_mob_serves_mob_choices_only_when_enabled() {
        const KEY: &str = "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.1.MobChoices";
        let mut store = Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        });
        let entries = store.top_n_get(&ParsedKey::parse(KEY)).unwrap();
        assert!(!entries.is_empty() && entries.len() <= 100);
        let (score, metadata) = &entries[0];
        assert!((1.0..=3.0).contains(score));
        let xml = String::from_utf8(metadata.clone()).unwrap();
        assert!(xml.starts_with("<Metadata><xuid>"), "{xml}");
        assert!(xml.contains("<correct type=\"boolean\">"), "{xml}");
        assert_eq!(store.top_n_get(&ParsedKey::parse(KEY)).unwrap(), entries);
        // Another session's board is not simulated.
        assert_eq!(
            crate::simulation::canonical_board(
                &ParsedKey::parse(
                    "v3.Session.MGS.1vs100.TopN.x.PRIMETIME.Round.1.Question.1.MobChoices"
                ),
                Board::MobChoices
            ),
            None
        );
        store.options.synthetic_mob = false;
        assert!(store.top_n_get(&ParsedKey::parse(KEY)).is_none());
    }

    #[test]
    fn mob_choices_answer_the_question_the_qa_of_that_number_asked() {
        let key = |question: u32| {
            format!(
                "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.{question}.MobChoices"
            )
        };
        let mut store = Store::bootstrap_with(SimulationOptions {
            synthetic_mob: true,
            ..Default::default()
        });
        let correct_choices = |store: &Store, question: u32| {
            store
                .top_n_get(&ParsedKey::parse(&key(question)))
                .unwrap()
                .into_iter()
                .filter(|(_, metadata)| metadata.ends_with(b">1</correct></Metadata>"))
                .map(|(choice, _)| choice as u8)
                .collect::<std::collections::HashSet<_>>()
        };
        let state = |name: &str, question: u32, info: &str| {
            format!(
                "<XsorObject><name>{name}</name><data type=\"table\"><RoundNumber type=\"float\">1</RoundNumber>\
                 <QuestionNumber type=\"float\">{question}</QuestionNumber>{info}</data></XsorObject>"
            )
            .into_bytes()
        };
        // No QA stored for it: the bank's first question (251317, choice 3).
        assert_eq!(correct_choices(&store, 2), [3].into());
        // Question 2 asks 251401 ("What color is a ripe banana?", choice 2).
        store.set(
            captured_game_state_key(),
            state("QA", 2, "<QuestionInfo type=\"table\"><difficulty type=\"float\">1</difficulty><questionID type=\"float\">251401</questionID></QuestionInfo>"),
        );
        assert_eq!(correct_choices(&store, 2), [2].into());
        // A help reads the Mob's answers while UseHelp is stored, which names no question
        // (2026-10-23 LAN recording: Trust the Mob handed the One the wrong answer).
        store.set(captured_game_state_key(), state("UseHelp", 2, ""));
        assert_eq!(correct_choices(&store, 2), [2].into());
    }

    #[test]
    fn top_n_get_sorts_descending_and_preserves_first_arrival_for_ties() {
        let mut store = Store::default();
        for (score, metadata) in [
            (0.5, b"<Metadata>first</Metadata>".to_vec()),
            (1.0, b"<Metadata>second</Metadata>".to_vec()),
            (1.0, b"<Metadata>third</Metadata>".to_vec()),
        ] {
            assert!(store.top_n_post("key", 3, score, metadata));
        }

        assert_eq!(
            store.top_n_get(&ParsedKey::parse("key")).unwrap(),
            vec![
                (1.0, b"<Metadata>second</Metadata>".to_vec()),
                (1.0, b"<Metadata>third</Metadata>".to_vec()),
                (0.5, b"<Metadata>first</Metadata>".to_vec()),
            ]
        );
    }

    #[test]
    fn top_n_repost_updates_metadata_match_and_capacity_keeps_best_entries() {
        let mut store = Store::default();
        assert!(store.top_n_post("key", 2, 1.0, b"<Metadata>first</Metadata>".to_vec()));
        assert!(store.top_n_post("key", 2, 2.0, b"<Metadata>second</Metadata>".to_vec()));
        assert!(store.top_n_post("key", 2, 3.0, b"<Metadata>third</Metadata>".to_vec()));
        assert!(store.top_n_post("key", 2, 4.0, b"<Metadata>second</Metadata>".to_vec()));

        assert_eq!(
            store.top_n_get(&ParsedKey::parse("key")).unwrap(),
            vec![
                (4.0, b"<Metadata>second</Metadata>".to_vec()),
                (3.0, b"<Metadata>third</Metadata>".to_vec()),
            ]
        );
        assert!(!store.top_n_post("key", 1, 5.0, b"<Metadata>mismatch</Metadata>".to_vec()));
        assert_eq!(store.top_n_get(&ParsedKey::parse("key")).unwrap().len(), 2);
    }

    #[test]
    fn top_n_rejects_mismatched_nested_metadata_without_mutating_state() {
        let mut store = Store::default();
        let original = b"<Metadata>valid</Metadata>".to_vec();
        assert!(store.top_n_post("key", 2, 1.0, original.clone()));

        assert!(!store.top_n_post("key", 2, 2.0, b"<Metadata><A></B></Metadata>".to_vec(),));
        assert_eq!(
            store.top_n_get(&ParsedKey::parse("key")),
            Some(vec![(1.0, original)])
        );
    }

    #[test]
    fn top_n_rejects_invalid_capacity_score_and_metadata_without_creating_state() {
        let mut store = Store::default();
        for (capacity, score, metadata) in [
            (0, 1.0, b"<Metadata>zero</Metadata>".to_vec()),
            (102, 1.0, b"<Metadata>large</Metadata>".to_vec()),
            (1, f64::NAN, b"<Metadata>nan</Metadata>".to_vec()),
            (1, f64::INFINITY, b"<Metadata>inf</Metadata>".to_vec()),
            (1, 1.0, b"<Metadata>unclosed".to_vec()),
            (1, 1.0, b"<Other>wrong root</Other>".to_vec()),
            (
                1,
                1.0,
                b"<Metadata>one</Metadata><Metadata>two</Metadata>".to_vec(),
            ),
        ] {
            assert!(!store.top_n_post("invalid", capacity, score, metadata));
        }
        assert_eq!(store.top_n_get(&ParsedKey::parse("invalid")), None);
    }

    const SESSION: &str = "00000000-0000-0000-0000-000000000001";

    const ONE_KEY: &str =
        "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.30.TheOne";

    #[test]
    fn first_read_of_theone_freezes_the_election_for_every_client() {
        let mut store = Store::default();
        let first = b"<Metadata>S</Metadata>".to_vec();
        let late = b"<Metadata>PHAT</Metadata>".to_vec();
        assert!(store.top_n_post(ONE_KEY, 1, 0.680581, first.clone()));
        // S reads before PHAT has posted, so S is The One for everyone.
        assert_eq!(
            store.top_n_read(&ParsedKey::parse(ONE_KEY)),
            Some(vec![(0.680581, first.clone())])
        );
        // PHAT's better-scoring post is acknowledged but must not displace the frozen winner.
        assert!(store.top_n_post(ONE_KEY, 1, 0.681184, late));
        assert_eq!(
            store.top_n_read(&ParsedKey::parse(ONE_KEY)),
            Some(vec![(0.680581, first)])
        );
    }

    #[test]
    fn theone_posts_before_the_first_read_still_rank_by_score() {
        let mut store = Store::default();
        assert!(store.top_n_post(ONE_KEY, 1, 0.5, b"<Metadata>low</Metadata>".to_vec()));
        assert!(store.top_n_post(ONE_KEY, 1, 0.9, b"<Metadata>high</Metadata>".to_vec()));
        assert_eq!(
            store.top_n_read(&ParsedKey::parse(ONE_KEY)),
            Some(vec![(0.9, b"<Metadata>high</Metadata>".to_vec())])
        );
    }

    #[test]
    fn empty_theone_read_does_not_freeze_and_other_keys_never_freeze() {
        let mut store = Store::default();
        assert_eq!(store.top_n_read(&ParsedKey::parse(ONE_KEY)), None);
        assert!(store.top_n_post(ONE_KEY, 1, 1.0, b"<Metadata>a</Metadata>".to_vec()));
        assert_eq!(
            store.top_n_read(&ParsedKey::parse(ONE_KEY)).unwrap().len(),
            1
        );

        let mob = "v3.Session.MGS.1vs100.TopN.00000000-0000-0000-0000-000000000001.PRIMETIME.30.MobSelection";
        assert!(store.top_n_post(mob, 101, 1.0, b"<Metadata>a</Metadata>".to_vec()));
        assert_eq!(store.top_n_read(&ParsedKey::parse(mob)).unwrap().len(), 1);
        assert!(store.top_n_post(mob, 101, 2.0, b"<Metadata>b</Metadata>".to_vec()));
        assert_eq!(store.top_n_read(&ParsedKey::parse(mob)).unwrap().len(), 2);
    }

    /// `key` after the election fold, as `dispatch` folds it.
    fn fold(store: &mut Store, key: &str, candidacy: bool, now: Utc) -> String {
        store
            .fold_election(ParsedKey::parse(key), candidacy, now)
            .raw
    }

    #[test]
    fn candidacies_within_the_window_share_one_election_id() {
        let mut store = Store::default();
        let t0 = Utc::from_unix_secs(1_792_618_363);
        let one = |id: &str| format!("v3.Session.MGS.1vs100.TopN.{SESSION}.PRIMETIME.{id}.TheOne");
        let mob = |id: &str| format!("v3.Session.MGS.1vs100.Store.{SESSION}.PRIMETIME.{id}.TheMob");
        // Round 2: PHAT keys its election 381, S keys it 387 (local clocks differ by 6 s).
        assert_eq!(fold(&mut store, &one("381"), true, t0), one("381"));
        assert_eq!(
            fold(&mut store, &one("387"), true, t0 + Duration::from_secs(1)),
            one("381")
        );
        // Every later key of either console resolves to the first id, reads and writes alike.
        assert_eq!(
            fold(&mut store, &mob("387"), false, t0 + Duration::from_secs(20)),
            mob("381")
        );
        assert_eq!(
            fold(&mut store, &mob("381"), false, t0 + Duration::from_secs(20)),
            mob("381")
        );
        // A re-election later than the window opens a fresh epoch.
        assert_eq!(
            fold(&mut store, &one("900"), true, t0 + Duration::from_secs(400)),
            one("900")
        );
        assert_eq!(
            fold(
                &mut store,
                &mob("900"),
                false,
                t0 + Duration::from_secs(401)
            ),
            mob("900")
        );
    }

    #[test]
    fn election_key_canonicalisation_ignores_other_keys_and_unknown_ids() {
        let mut store = Store::default();
        let now = Utc::from_unix_secs(1_792_618_363);
        for key in [
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.GameState",
            "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.QuestionBank.Difficulty.1.Available",
            "v3.Season.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.1.Question.0.GameStateLiveHostIntro",
        ] {
            assert_eq!(fold(&mut store, key, true, now), key);
        }
        // A read of an id that never posted a candidacy is left alone (and does not open an epoch).
        let read = format!("v3.Session.MGS.1vs100.Store.{SESSION}.PRIMETIME.77.TheMob");
        assert_eq!(fold(&mut store, &read, false, now), read);
        let post = format!("v3.Session.MGS.1vs100.TopN.{SESSION}.PRIMETIME.90.TheOne");
        assert_eq!(fold(&mut store, &post, true, now), post);
    }
}

#[cfg(test)]
mod crowd_key_tests {
    use super::*;
    use crate::{
        ids::{QuestionNumber, Round},
        questions::Choice,
        sbg::key::QuestionSum,
    };

    const KEY: &str = "v3.Session.MGS.1vs100.Sum.00000000-0000-0000-0000-000000000001.PRIMETIME.Round.2.Question.3.Crowd.Choice1";

    #[test]
    fn parses_crowd_keys_only() {
        let crowd = |key: &str| match ParsedKey::parse(key).kind {
            Key::QuestionSum {
                round,
                question,
                sum: QuestionSum::Crowd(choice),
                ..
            } => Some((round, question, choice)),
            _ => None,
        };
        assert_eq!(
            crowd(KEY),
            Some((Round(2), QuestionNumber::new(3), Choice::A))
        );
        assert_eq!(crowd(&KEY.replace("Choice1", "Choice4")), None);
        assert_eq!(crowd(&KEY.replace("Crowd", "Mob")), None);
    }

    #[test]
    fn simulated_crowd_adds_to_real_votes_even_without_posts() {
        let mut store = Store::bootstrap_with(SimulationOptions {
            synthetic_crowd: 60,
            ..Default::default()
        });
        let total = |store: &Store| {
            (1..=3)
                .map(|c| {
                    store
                        .sum_get(&ParsedKey::parse(
                            &KEY.replace("Choice1", &format!("Choice{c}")),
                        ))
                        .unwrap()
                })
                .sum::<f64>()
        };
        assert_eq!(total(&store), 60.0);
        assert!(store.sum_add(KEY, 1.0));
        assert_eq!(total(&store), 61.0);
        let off = Store::bootstrap();
        assert_eq!(off.sum_get(&ParsedKey::parse(KEY)), None);
    }
}
