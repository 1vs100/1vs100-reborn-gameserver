//! GameState flow model: which states may follow, how long each lasts, what carries over.
//! Shadow mode checks posted states against the model and logs deviations.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    time::Duration,
};

use crate::{
    gamestate::{GameStateDoc, GameStateName},
    time::Utc,
};

/// A GameState document with the window the model works on (epoch seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct StateDoc {
    pub doc: GameStateDoc,
    /// The name as posted (shown in deviations, also when it is none of the title's).
    pub name: String,
    pub state: Option<GameStateName>,
    pub start: f64,
    pub end: f64,
}

impl StateDoc {
    /// `None` without a window. Reads both forms (1317 `<XsorObject>`, 1149 `<xsor:object>`).
    pub fn new(doc: GameStateDoc) -> Option<StateDoc> {
        Some(StateDoc {
            name: doc.name().into_owned(),
            state: doc.state(),
            start: doc.start()?.as_secs_f64(),
            end: doc.end()?.as_secs_f64(),
            doc,
        })
    }

    pub fn num(&self, field: &str) -> Option<f64> {
        self.doc.field(field)?.parse().ok()
    }

    pub fn text(&self, field: &str) -> Option<Cow<'_, str>> {
        self.doc.field(field)
    }

    fn has(&self, field: &str) -> bool {
        self.doc.field(field).is_some()
    }
}

/// Parse a GameState document for the model.
pub fn parse(xml: impl AsRef<[u8]>) -> Option<StateDoc> {
    StateDoc::new(GameStateDoc::parse(xml.as_ref())?)
}

/// States that may directly follow `state` (derived from `statequestion.lua` / `stateroundstart.lua`).
pub fn successors(state: GameStateName) -> &'static [GameStateName] {
    use GameStateName::*;
    match state {
        ElectHost => &[LiveHostIntro],
        LiveHostIntro => &[Introductions],
        Introductions => &[Qa],
        Qa => &[OnesAnswer, UseHelp, RoundStart],
        UseHelp => &[WhichHelp, RoundStart],
        WhichHelp => &[OnesAnswer, QaMobElimination, MoneyOrMob, Qa, RoundStart],
        OnesAnswer => &[QaMobElimination, MoneyOrMob, RoundStart],
        QaMobElimination => &[Qa, RoundStart],
        MoneyOrMob => &[Qa, RoundStart],
        RoundStart => &[LiveHostIntro, ElectHost],
    }
}

/// Whether `next` may follow `prev`; a name that is none of the title's follows nothing.
fn may_follow(prev: &StateDoc, next: &StateDoc) -> bool {
    match (prev.state, next.state) {
        (Some(prev), Some(next)) => successors(prev).contains(&next),
        _ => false,
    }
}

/// Lengths (seconds) a state may have (`PTTimingsTable.ServerStateTimeLength`). The answer states
/// include `answerRevealTime`, which is 6 s for question tiers 0-6 and 7 s for tiers 7-10, so they come
/// in two lengths; the first entry is the default. Empty: the length is not fixed (it depends on
/// the One's inputs, see `mob_elimination_length`, or on the election and the round's awards).
pub fn allowed_lengths(state: GameStateName) -> &'static [f64] {
    use GameStateName::*;
    match state {
        LiveHostIntro => &[22.0],
        Introductions => &[38.0],
        OnesAnswer => &[16.0, 17.0],
        UseHelp => &[7.0],
        WhichHelp => &[23.0, 24.0],
        ElectHost | Qa | QaMobElimination | MoneyOrMob | RoundStart => &[],
    }
}

/// The length a state must have: the posted one when it is an allowed length, otherwise the default.
pub fn fixed_length(state: GameStateName, posted: f64) -> Option<f64> {
    let allowed = allowed_lengths(state);
    allowed
        .iter()
        .copied()
        .find(|len| (posted - len).abs() <= 0.5)
        .or_else(|| allowed.first().copied())
}

/// What the server predicts for the data of the state after `prev`.
#[derive(Debug, Default)]
pub struct Prediction {
    /// Fields that must have exactly this value.
    pub expect: BTreeMap<String, String>,
    /// Fields that must have one of these values.
    pub one_of: BTreeMap<String, Vec<String>>,
    /// Fields that must be absent.
    pub absent: Vec<&'static str>,
    /// Fields that are inputs from the One / the crowd (or still unmodelled): never compared.
    pub free: Vec<&'static str>,
}

fn n(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

/// `helpsLeft` after `h`: unchanged, or one fewer (spent in UseHelp or WhichHelp).
fn one_help_spent_or_not(p: &mut Prediction, h: f64) {
    p.one_of.insert("helpsLeft".into(), vec![n(h), n(h - 1.0)]);
}

/// Fields every state carries forward unchanged unless a rule below says otherwise.
const CARRIED: &[&str] = &[
    "OnesDance",
    "OnesXuid",
    "OnesGamertag",
    "roleDeterminationKey",
    "RoundNumber",
    "adTimingIndex",
];

/// Deterministic part of `next_name` after `prev`; `None` without a rule yet.
pub fn predict(prev: &StateDoc, next: GameStateName) -> Option<Prediction> {
    use GameStateName::*;
    let mut p = Prediction::default();
    for field in CARRIED {
        if let Some(value) = prev.text(field) {
            p.expect.insert((*field).to_owned(), value.into_owned());
        }
    }
    let q = prev.num("QuestionNumber")?;
    match (prev.state?, next) {
        (LiveHostIntro, Introductions) => {
            p.expect.insert("stateLength".into(), "38".into());
            p.expect.insert("QuestionNumber".into(), n(q));
            p.free = vec![
                "totalPlayers",
                "OnesLTS",
                "OnesRoundScore",
                "OnesRoundQuestions",
                "QuestionsAsked",
                "MobSize",
                "helpsLeft",
            ];
        }
        (Introductions, Qa) => {
            p.expect.insert("stateLength".into(), "24".into());
            p.expect.insert("QuestionNumber".into(), n(q + 1.0));
            p.expect.insert("bCurveBall".into(), "0".into());
            p.expect.insert("OnesRoundQuestions".into(), "0".into());
            p.expect.insert("OnesRoundScore".into(), "0".into());
            p.free = vec![
                "questionID",
                "difficulty",
                "totalPlayers",
                "OnesLTS",
                "QuestionsAsked",
                "MobSize",
                "helpsLeft",
            ];
        }
        (Qa, OnesAnswer | UseHelp) => {
            // OnesAnswer's length follows from the One's AnswerRevealTime (`answer_reveal_length`).
            if next == UseHelp {
                p.expect.insert("stateLength".into(), "7".into());
            }
            p.expect.insert("QuestionNumber".into(), n(q));
            p.expect.insert("QuestionsAsked".into(), n(q));
            p.expect.insert("bSuddenStop".into(), "0".into());
            p.expect.insert("bHardStop".into(), "0".into());
            p.absent = vec!["bCurveBall", "questionID", "difficulty"];
            p.free = vec![
                "OneAnswer",
                "AnswerRevealTime",
                "totalPlayers",
                "OnesLTS",
                "OnesRoundScore",
                "OnesRoundQuestions",
                "MobSize",
                "bDoMoneyOrMobResult",
            ];
            if let Some(h) = prev.num("helpsLeft") {
                match next {
                    // A help locked during the question is already spent here.
                    UseHelp => one_help_spent_or_not(&mut p, h),
                    _ => {
                        p.expect.insert("helpsLeft".into(), n(h));
                    }
                }
            }
        }
        (UseHelp, WhichHelp) => {
            p.expect.insert("stateLength".into(), "23".into());
            p.expect.insert("QuestionNumber".into(), n(q));
            p.expect.insert("QuestionsAsked".into(), n(q));
            // Spent here unless UseHelp already showed it (`one_help_spent_or_not`).
            if let Some(h) = prev.num("helpsLeft") {
                one_help_spent_or_not(&mut p, h);
            }
            p.absent = vec!["AnswerRevealTime"];
            p.free = vec![
                "OneLifeline",
                "totalPlayers",
                "OnesLTS",
                "OnesRoundScore",
                "OnesRoundQuestions",
                "MobSize",
                "bSuddenStop",
                "bHardStop",
            ];
        }
        (QaMobElimination | MoneyOrMob, Qa) => {
            p.expect.insert("stateLength".into(), "24".into());
            p.expect.insert("QuestionNumber".into(), n(q + 1.0));
            p.absent = vec![
                "EliminatedMobCount",
                "TotalEliminatedMobCount",
                "eliminateMobstersTime",
                "bShowStats",
            ];
            // the One throws a curveball from its own Mob-elimination count (`ThrowCurveBallQuestion`)
            p.free = vec![
                "bCurveBall",
                "questionID",
                "difficulty",
                "totalPlayers",
                "OnesLTS",
                "OnesRoundScore",
                "OnesRoundQuestions",
                "QuestionsAsked",
                "MobSize",
                "helpsLeft",
                "bFromHelpState",
                "bDoMoneyOrMobResult",
                "bShowAd",
                "bAskMoneyOrMob",
                "enoughQuestionsPassed",
                "adTimingIndex",
                "MobElimTimingTable",
            ];
        }
        (RoundStart, LiveHostIntro) => {
            p.expect.clear();
            p.expect.insert("stateLength".into(), "22".into());
            p.expect.insert("QuestionNumber".into(), "0".into());
            p.expect.insert("MobSize".into(), "100".into());
            p.expect.insert("helpsLeft".into(), "3".into());
            if let Some(r) = prev.num("RoundNumber") {
                p.expect.insert("RoundNumber".into(), n(r + 1.0));
            }
            p.absent = vec!["awardsLength", "prizingSubstate"];
            // the next round's One, role key, score and head-count come from the election
            p.free = vec![
                "OnesXuid",
                "OnesGamertag",
                "roleDeterminationKey",
                "totalPlayers",
                "OnesLTS",
                "OnesRoundScore",
                "OnesRoundQuestions",
                "QuestionsAsked",
                "OnesDance",
                "adTimingIndex",
            ];
        }
        _ => return None,
    }
    Some(p)
}

/// Field-level deviations of `next` from `prediction`.
pub fn check_fields(next: &StateDoc, prediction: &Prediction) -> Vec<String> {
    let mut diffs = Vec::new();
    for (field, want) in &prediction.expect {
        if prediction.free.contains(&field.as_str()) {
            continue;
        }
        // a QA that opens with the money-or-mob result runs 3 s longer
        let want = if field == "stateLength"
            && want == "24"
            && next.text("bDoMoneyOrMobResult").as_deref() == Some("1")
        {
            "27"
        } else {
            want.as_str()
        };
        match next.text(field) {
            Some(got) if got == *want => {}
            Some(got) => diffs.push(format!("{field}={got} (expected {want})")),
            None => diffs.push(format!("{field} missing (expected {want})")),
        }
    }
    for (field, allowed) in &prediction.one_of {
        match next.text(field) {
            Some(got) if allowed.iter().any(|want| *want == got) => {}
            Some(got) => diffs.push(format!("{field}={got} (expected one of {allowed:?})")),
            None => diffs.push(format!("{field} missing (expected one of {allowed:?})")),
        }
    }
    for field in &prediction.absent {
        if next.has(field) {
            diffs.push(format!("{field} present (expected absent)"));
        }
    }
    diffs
}

/// Whole seconds, title format.
fn format_seconds(seconds: f64) -> String {
    Utc::from_unix_secs(seconds.max(0.0) as i64).to_xsor()
}

/// Server-owned flow decision on a posted state.
#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Store this document (possibly re-timed onto the server's chain).
    Accept(GameStateDoc),
    /// Drop the post: the chain already holds this state.
    Ignore(&'static str),
}

/// The server owns the chain.  `prev` is the state the server currently has stored.  A post that
/// repeats a state the chain already has is dropped; anything else follows the chain: its start
/// becomes `prev.end` and (for fixed-length states) its end follows from the model.
pub fn decide(prev: &StateDoc, next: &StateDoc) -> Decision {
    if next.name == prev.name && next.num("QuestionNumber") == prev.num("QuestionNumber") {
        return Decision::Ignore("same state again");
    }
    if !may_follow(prev, next) {
        return Decision::Accept(next.doc.clone());
    }
    let posted = next.end - next.start;
    let length = next
        .state
        .and_then(|state| fixed_length(state, posted))
        .unwrap_or(posted);
    if (next.start - prev.end).abs() <= 1.5 && ((next.end - next.start) - length).abs() <= 0.5 {
        return Decision::Accept(next.doc.clone());
    }
    let mut out = next.doc.clone();
    out.set("startTimeUtc", format_seconds(prev.end));
    out.set("endTimeUtc", format_seconds(prev.end + length));
    out.set("sendGetUtc", format_seconds(prev.end));
    out.set_field("stateLength", n(length));
    Decision::Accept(out)
}

/// Deviations of `next` from what the model expects after `prev`; empty means "as predicted".
pub fn check(prev: &StateDoc, next: &StateDoc) -> Vec<String> {
    let mut diffs = Vec::new();
    if !may_follow(prev, next) {
        diffs.push(format!("{} may not follow {}", next.name, prev.name));
    }
    // `SetState` chains every state onto the end of the current one.
    if (next.start - prev.end).abs() > 1.5 {
        diffs.push(format!(
            "start {:+.1}s from the expected start",
            next.start - prev.end
        ));
    }
    if let Some(expected) = next
        .state
        .and_then(|state| fixed_length(state, next.end - next.start))
        && ((next.end - next.start) - expected).abs() > 0.5
    {
        diffs.push(format!(
            "length {:.0}s, expected {expected:.0}s",
            next.end - next.start
        ));
    }
    if let Some(declared) = next.num("stateLength")
        && ((next.end - next.start) - declared).abs() > 0.5
    {
        diffs.push(format!(
            "stateLength {declared:.0} vs window {:.0}",
            next.end - next.start
        ));
    }
    use GameStateName::*;
    let new_round = prev.state == Some(RoundStart);
    if !new_round {
        for field in ["OnesXuid", "roleDeterminationKey"] {
            if prev.text(field) != next.text(field) {
                diffs.push(format!("{field} changed mid-round"));
            }
        }
        if prev.num("RoundNumber") != next.num("RoundNumber") {
            diffs.push("RoundNumber changed mid-round".into());
        }
    } else if next.state == Some(LiveHostIntro)
        && next.num("RoundNumber") != prev.num("RoundNumber").map(|r| r + 1.0)
    {
        diffs.push("RoundNumber did not advance after RoundStart".into());
    }
    if let (Some(a), Some(b)) = (prev.num("QuestionNumber"), next.num("QuestionNumber")) {
        let advances = next.state == Some(Qa)
            && matches!(
                prev.state,
                Some(Introductions | QaMobElimination | MoneyOrMob)
            );
        let want = if advances { a + 1.0 } else { a };
        if !new_round && (b - want).abs() > f64::EPSILON && next.state != Some(Introductions) {
            diffs.push(format!("QuestionNumber {b} (expected {want})"));
        }
    }
    if let (Some(a), Some(b)) = (prev.num("MobSize"), next.num("MobSize"))
        && b > a
        && !new_round
    {
        diffs.push(format!("MobSize grew {a} -> {b}"));
    }
    if let Some(prediction) = next.state.and_then(|state| predict(prev, state)) {
        diffs.extend(check_fields(next, &prediction));
    }
    diffs.extend(mob_elimination_length(next));
    diffs.extend(answer_reveal_length(next));
    diffs
}

/// The One's OnesAnswer: its `AnswerRevealTime` is one the title offers, and the state lasts the
/// title's base time plus that reveal.
fn answer_reveal_length(next: &StateDoc) -> Option<String> {
    use crate::pt_timings as t;
    if next.state != Some(GameStateName::OnesAnswer) {
        return None;
    }
    let Some(reveal) = next.num("AnswerRevealTime") else {
        return Some("AnswerRevealTime missing".into());
    };
    let reveal = reveal.round();
    let reveal_time = Duration::from_secs_f64(reveal.max(0.0));
    if !t::ANSWER_REVEAL.contains(&reveal_time) {
        return Some(format!(
            "AnswerRevealTime={reveal} (expected one of {:?})",
            t::ANSWER_REVEAL
        ));
    }
    let expected = (t::ONES_ANSWER_BASE + reveal_time).as_secs() as i64;
    let length = (next.end - next.start).round() as i64;
    (length != expected).then(|| {
        format!("OnesAnswer length {length}s, title formula {expected}s (reveal {reveal}s)")
    })
}

/// The One's QAMobElimination / MoneyOrMob length against the title's formula, using the One's
/// own elimination time (the one input the server cannot derive) and its break / stop flags.
fn mob_elimination_length(next: &StateDoc) -> Option<String> {
    use crate::pt_timings as t;
    if !matches!(
        next.state,
        Some(GameStateName::QaMobElimination | GameStateName::MoneyOrMob)
    ) {
        return None;
    }
    let elim = next.num("eliminateMobstersTime")?.round();
    let elim_time = Duration::from_secs_f64(elim.max(0.0));
    let flag = |f: &str| matches!(next.text(f).as_deref(), Some("true" | "1" | "True"));
    let ad = if flag("bShowAd") {
        let index = next.num("adTimingIndex").map_or(1, |i| i as usize).max(1);
        t::AD_BREAK
            .get(index - 1)
            .copied()
            .unwrap_or(Duration::ZERO)
    } else {
        Duration::ZERO
    };
    let stats = if flag("bShowStats") {
        t::STAT_BREAK
    } else {
        Duration::ZERO
    };
    let hard_stop = if flag("bHardStop") {
        t::HardStop::Yes
    } else {
        t::HardStop::No
    };
    let sudden_stop = if flag("bSuddenStop") {
        t::SuddenStop::Yes
    } else {
        t::SuddenStop::No
    };
    let expected = if next.state == Some(GameStateName::MoneyOrMob) {
        t::money_or_mob(elim_time, hard_stop, ad, stats)
    } else {
        t::qa_mob_elimination(elim_time, sudden_stop, ad, stats)
    }
    .as_secs() as i64;
    let length = (next.end - next.start).round() as i64;
    (length != expected).then(|| {
        format!("{} length {length}s, title formula {expected}s (elimination {elim}s, ad {}s, stats {}s)", next.name, ad.as_secs(), stats.as_secs())
    })
}

/// Shadow mode: compare every accepted GameState post with the server's model
/// of the flow and log deviations; the post is stored unchanged either way. With `server_flow` the
/// server's stored state is the chain: repeats are dropped, other posts are re-timed onto it
/// (see [`decide`]).
pub fn check_post(
    server_flow: bool,
    last: &mut HashMap<String, StateDoc>,
    key: &str,
    value: Vec<u8>,
    stored: Option<&[u8]>,
) -> Option<Vec<u8>> {
    let Some(mut doc) = parse(&value) else {
        return Some(value);
    };
    let mut value = value;
    // Under the server-owned flow the chain is what the store holds right now.
    let prev = if server_flow {
        stored.and_then(parse).or_else(|| last.get(key).cloned())
    } else {
        last.get(key).cloned()
    };
    if let Some(prev) = prev {
        let diffs = check(&prev, &doc);
        if diffs.is_empty() {
            tracing::info!(state = %doc.name, "shadow: post matches the model");
        } else {
            tracing::warn!(state = %doc.name, after = %prev.name, deviations = %diffs.join("; "), "shadow: post deviates from the model");
        }
        if server_flow {
            match decide(&prev, &doc) {
                Decision::Ignore(why) => {
                    tracing::warn!(state = %doc.name, after = %prev.name, why, "server flow: post ignored");
                    return None;
                }
                Decision::Accept(fixed) if fixed.to_xml() != value => {
                    if let Some(fixed) = StateDoc::new(fixed) {
                        tracing::warn!(state = %doc.name, "server flow: post re-timed onto the server's chain");
                        value = fixed.doc.to_xml();
                        doc = fixed;
                    }
                }
                Decision::Accept(_) => {}
            }
        }
    }
    last.insert(key.to_owned(), doc);
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_round_order_lists_every_state_once_along_the_flow() {
        use GameStateName::*;
        let order = GameStateName::ROUND_ORDER;
        for state in GameStateName::ALL {
            assert_eq!(order.iter().filter(|s| **s == state).count(), 1, "{state}");
        }
        // The main path of a question, in order, each step one the flow allows.
        for pair in [
            (ElectHost, LiveHostIntro),
            (LiveHostIntro, Introductions),
            (Introductions, Qa),
            (Qa, UseHelp),
            (UseHelp, WhichHelp),
            (WhichHelp, OnesAnswer),
            (OnesAnswer, QaMobElimination),
            (OnesAnswer, MoneyOrMob),
            (RoundStart, LiveHostIntro),
        ] {
            assert!(successors(pair.0).contains(&pair.1), "{pair:?}");
            let at = |state| order.iter().position(|s| *s == state).unwrap();
            assert!(at(pair.0) < at(pair.1), "{pair:?} in order");
        }
    }

    fn doc(name: &str, start: &str, end: &str, data: &str) -> String {
        format!(
            "<XsorObject><startTimeUtc type=\"userdata\" subtype=\"UTCTime\">{start}</startTimeUtc>\
             <name>{name}</name><data>{data}</data>\
             <endTimeUtc type=\"userdata\" subtype=\"UTCTime\">{end}</endTimeUtc></XsorObject>"
        )
    }

    #[test]
    fn mob_elimination_length_is_checked_against_the_title_formula() {
        let good = doc(
            "QAMobElimination",
            "2026-10-11T19:19:45.000Z",
            "2026-10-11T19:19:56.000Z",
            "<eliminateMobstersTime type=\"float\">7</eliminateMobstersTime>",
        );
        assert!(mob_elimination_length(&parse(&good).unwrap()).is_none());
        let bad = doc(
            "QAMobElimination",
            "2026-10-11T19:19:45.000Z",
            "2026-10-11T19:19:55.000Z",
            "<eliminateMobstersTime type=\"float\">7</eliminateMobstersTime>",
        );
        assert!(mob_elimination_length(&parse(&bad).unwrap()).is_some());
    }

    #[test]
    fn ones_answer_length_follows_the_ones_answer_reveal() {
        let ones_answer = |end: &str, reveal: u32| {
            doc(
                "OnesAnswer",
                "2026-10-11T19:19:24.000Z",
                end,
                &format!("<AnswerRevealTime type=\"float\">{reveal}</AnswerRevealTime>"),
            )
        };
        for (end, reveal) in [
            ("2026-10-11T19:19:40.000Z", 6),
            ("2026-10-11T19:19:41.000Z", 7),
        ] {
            let state = parse(ones_answer(end, reveal)).unwrap();
            assert_eq!(answer_reveal_length(&state), None, "reveal {reveal}");
        }
        let short = parse(ones_answer("2026-10-11T19:19:40.000Z", 7)).unwrap();
        assert!(answer_reveal_length(&short).is_some());
        let unknown = parse(ones_answer("2026-10-11T19:19:42.000Z", 8)).unwrap();
        assert!(answer_reveal_length(&unknown).is_some());
    }

    #[test]
    fn parses_scalars_and_skips_tables() {
        let xml = doc(
            "QAMobElimination",
            "2026-10-11T19:19:45.000Z",
            "2026-10-11T19:19:55.000Z",
            "<stateLength type=\"float\">10</stateLength><OnesXuid>123</OnesXuid>\
             <MobElimTimingTable><XsorItem>D 0.35</XsorItem></MobElimTimingTable>\
             <MobSize type=\"float\">89</MobSize>",
        );
        let d = parse(&xml).unwrap();
        assert_eq!(d.name, "QAMobElimination");
        assert_eq!(d.end - d.start, 10.0);
        assert_eq!(d.num("MobSize"), Some(89.0));
        assert_eq!(d.text("OnesXuid").as_deref(), Some("123"));
        assert!(!d.has("MobElimTimingTable"));
    }

    #[test]
    fn the_observed_opening_matches_the_model() {
        // Round 1 of the 1149 two-console fixture: each state starts where the previous one ends.
        let one = "<OnesXuid>1</OnesXuid><roleDeterminationKey>30</roleDeterminationKey>\
                   <RoundNumber type=\"float\">1</RoundNumber>";
        let lhi = parse(doc(
            "LiveHostIntro",
            "2026-10-22T19:30:34.000Z",
            "2026-10-22T19:30:56.000Z",
            &format!("<stateLength type=\"float\">22</stateLength>{one}<QuestionNumber type=\"float\">0</QuestionNumber>"),
        ))
        .unwrap();
        let intro = parse(doc(
            "Introductions",
            "2026-10-22T19:30:56.000Z",
            "2026-10-22T19:31:34.000Z",
            &format!("<stateLength type=\"float\">38</stateLength>{one}<QuestionNumber type=\"float\">0</QuestionNumber>"),
        ))
        .unwrap();
        let qa = parse(doc(
            "QA",
            "2026-10-22T19:31:34.000Z",
            "2026-10-22T19:31:58.000Z",
            &format!("<stateLength type=\"float\">24</stateLength>{one}<QuestionNumber type=\"float\">1</QuestionNumber><MobSize type=\"float\">100</MobSize>\
                 <bCurveBall type=\"boolean\">0</bCurveBall><OnesRoundQuestions type=\"float\">0</OnesRoundQuestions>\
                 <OnesRoundScore type=\"float\">0</OnesRoundScore>"),
        ))
        .unwrap();
        assert_eq!(check(&lhi, &intro), Vec::<String>::new());
        assert_eq!(check(&intro, &qa), Vec::<String>::new());
    }

    #[test]
    fn deviations_are_reported() {
        let a = parse(doc(
            "QA",
            "2026-10-11T19:19:00.000Z",
            "2026-10-11T19:19:24.000Z",
            "<OnesXuid>1</OnesXuid><QuestionNumber type=\"float\">1</QuestionNumber>",
        ))
        .unwrap();
        let b = parse(doc(
            "UseHelp",
            "2026-10-11T19:19:30.000Z",
            "2026-10-11T19:19:40.000Z",
            "<OnesXuid>2</OnesXuid><QuestionNumber type=\"float\">1</QuestionNumber>",
        ))
        .unwrap();
        let diffs = check(&a, &b);
        assert!(diffs.iter().any(|d| d.contains("expected start")));
        assert!(diffs.iter().any(|d| d.contains("length 10s, expected 7s")));
        assert!(diffs.iter().any(|d| d.contains("OnesXuid changed")));
    }

    #[test]
    fn a_help_is_spent_in_use_help_or_in_which_help() {
        let state = |name: &str, start: &str, end: &str, helps: u32| {
            parse(doc(
                name,
                start,
                end,
                &format!(
                    "<OnesXuid>1</OnesXuid><QuestionNumber type=\"float\">2</QuestionNumber>\
                     <QuestionsAsked type=\"float\">2</QuestionsAsked><bSuddenStop type=\"boolean\">0</bSuddenStop>\
                     <bHardStop type=\"boolean\">0</bHardStop><helpsLeft type=\"float\">{helps}</helpsLeft>"
                ),
            ))
            .unwrap()
        };
        let qa = |helps| {
            state(
                "QA",
                "2026-10-23T05:55:00.000Z",
                "2026-10-23T05:55:24.000Z",
                helps,
            )
        };
        let use_help = |helps| {
            state(
                "UseHelp",
                "2026-10-23T05:55:24.000Z",
                "2026-10-23T05:55:31.000Z",
                helps,
            )
        };
        let which_help = |helps| {
            state(
                "WhichHelp",
                "2026-10-23T05:55:31.000Z",
                "2026-10-23T05:55:54.000Z",
                helps,
            )
        };
        let helps_diffs = |prev: &StateDoc, next: &StateDoc| {
            check(prev, next)
                .into_iter()
                .filter(|diff| diff.starts_with("helpsLeft"))
                .collect::<Vec<_>>()
        };
        // Chosen in the extra choose time (1149_mm_v2.sbg): spent in WhichHelp.
        assert!(helps_diffs(&qa(3), &use_help(3)).is_empty());
        assert!(helps_diffs(&use_help(3), &which_help(2)).is_empty());
        // Locked during the question (1149_mm_v2_signin_fix.sbg): spent in UseHelp already.
        assert!(helps_diffs(&qa(3), &use_help(2)).is_empty());
        assert!(helps_diffs(&use_help(2), &which_help(2)).is_empty());
        // Two at once, or one coming back, is a deviation.
        assert_eq!(helps_diffs(&use_help(3), &which_help(1)).len(), 1);
        assert_eq!(helps_diffs(&qa(2), &use_help(3)).len(), 1);
    }

    #[test]
    fn decide_drops_reposts_and_rechains_the_rest() {
        let intro = doc(
            "Introductions",
            "2026-10-11T19:18:27.000Z",
            "2026-10-11T19:19:05.000Z",
            "<stateLength type=\"float\">38</stateLength><QuestionNumber type=\"float\">0</QuestionNumber>",
        );
        let prev = parse(&intro).unwrap();
        // The same state posted again: dropped.
        let repost = doc(
            "Introductions",
            "2026-10-11T19:19:05.000Z",
            "2026-10-11T19:19:43.000Z",
            "<stateLength type=\"float\">38</stateLength><QuestionNumber type=\"float\">0</QuestionNumber>",
        );
        assert!(matches!(
            decide(&prev, &parse(&repost).unwrap()),
            Decision::Ignore(_)
        ));
        // A QA that starts late is re-chained onto the end of the stored Introductions.
        let qa = doc(
            "QA",
            "2026-10-11T19:19:20.000Z",
            "2026-10-11T19:19:44.000Z",
            "<stateLength type=\"float\">24</stateLength><QuestionNumber type=\"float\">1</QuestionNumber>",
        );
        let Decision::Accept(fixed) = decide(&prev, &parse(&qa).unwrap()) else {
            panic!("QA must be accepted");
        };
        let fixed = StateDoc::new(fixed).unwrap();
        assert_eq!(fixed.start, prev.end);
        assert_eq!(fixed.end - fixed.start, 24.0);
    }

    #[test]
    fn answer_states_keep_the_long_reveal_of_hard_questions() {
        // answerRevealTime is 6 s for question tiers 0-6 and 7 s for tiers 7-10, so OnesAnswer
        // is 16 or 17 s and WhichHelp 23 or 24 s; the substates add up to the longer value and the
        // client asserts ("will finish past end of the state") if the state is cut short.
        let qa = doc(
            "QA",
            "2026-10-11T19:19:00.000Z",
            "2026-10-11T19:19:24.000Z",
            "<stateLength type=\"float\">24</stateLength><QuestionNumber type=\"float\">8</QuestionNumber>",
        );
        let prev = parse(&qa).unwrap();
        for (name, len) in [
            ("OnesAnswer", 17),
            ("OnesAnswer", 16),
            ("WhichHelp", 24),
            ("WhichHelp", 23),
        ] {
            let prev = if name == "WhichHelp" {
                let use_help = doc(
                    "UseHelp",
                    "2026-10-11T19:19:24.000Z",
                    "2026-10-11T19:19:31.000Z",
                    "<stateLength type=\"float\">7</stateLength><QuestionNumber type=\"float\">8</QuestionNumber>",
                );
                parse(&use_help).unwrap()
            } else {
                prev.clone()
            };
            let next = doc(
                name,
                // posted 3 s late so the chain has to re-time it
                &format_seconds(prev.end + 3.0),
                &format_seconds(prev.end + 3.0 + len as f64),
                &format!(
                    "<stateLength type=\"float\">{len}</stateLength><QuestionNumber type=\"float\">8</QuestionNumber>"
                ),
            );
            let Decision::Accept(fixed) = decide(&prev, &parse(&next).unwrap()) else {
                panic!("{name} must be accepted");
            };
            let fixed = StateDoc::new(fixed).unwrap();
            assert_eq!(fixed.start, prev.end, "{name} {len}");
            assert_eq!(fixed.end - fixed.start, len as f64, "{name} {len}");
            assert!(
                check(&prev, &parse(&next).unwrap())
                    .iter()
                    .all(|d| !d.contains("length")),
                "{name} {len}"
            );
        }
    }

    #[test]
    fn rejects_non_xsor_documents() {
        assert!(parse("<GameState/>").is_none());
    }

    /// Replay a directory of dumped posts (`GAMESERVER_DUMP_GAMESTATE`): `GAMESERVER_SHADOW_REPLAY=<dir> cargo test replay -- --nocapture`.
    #[test]
    #[allow(clippy::disallowed_methods)] // a developer switch for this test, not a server setting
    fn replay_dump_dir() {
        let Some(dir) = std::env::var_os("GAMESERVER_SHADOW_REPLAY") else {
            return;
        };
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        let mut prev: Option<StateDoc> = None;
        for path in files {
            let doc = parse(std::fs::read(&path).unwrap()).unwrap();
            if let Some(p) = &prev {
                let diffs = check(p, &doc);
                println!(
                    "{:<28} {}",
                    path.file_name().unwrap().to_string_lossy(),
                    if diffs.is_empty() {
                        "ok".into()
                    } else {
                        diffs.join("; ")
                    }
                );
            }
            prev = Some(doc);
        }
    }
}
