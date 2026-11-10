//! GameState document: name, window, and `data` table; preserves field order.

use std::{borrow::Cow, fmt::Display};

use crate::{
    ids::{QuestionId, QuestionNumber, Round},
    time::Utc,
    xsor::table::Node,
};

pub mod flow;
pub mod hold;

/// Server state name of a GameState document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameStateName {
    ElectHost,
    LiveHostIntro,
    Introductions,
    Qa,
    OnesAnswer,
    UseHelp,
    WhichHelp,
    QaMobElimination,
    MoneyOrMob,
    RoundStart,
}

impl GameStateName {
    pub const ALL: [GameStateName; 10] = [
        GameStateName::ElectHost,
        GameStateName::LiveHostIntro,
        GameStateName::Introductions,
        GameStateName::Qa,
        GameStateName::OnesAnswer,
        GameStateName::UseHelp,
        GameStateName::WhichHelp,
        GameStateName::QaMobElimination,
        GameStateName::MoneyOrMob,
        GameStateName::RoundStart,
    ];

    /// Round state order; kept in agreement with [`flow::successors`] by test.
    pub const ROUND_ORDER: [GameStateName; 10] = [
        GameStateName::ElectHost,
        GameStateName::RoundStart,
        GameStateName::LiveHostIntro,
        GameStateName::Introductions,
        GameStateName::Qa,
        GameStateName::UseHelp,
        GameStateName::WhichHelp,
        GameStateName::OnesAnswer,
        GameStateName::QaMobElimination,
        GameStateName::MoneyOrMob,
    ];

    /// The name as the title writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            GameStateName::ElectHost => "ElectHost",
            GameStateName::LiveHostIntro => "LiveHostIntro",
            GameStateName::Introductions => "Introductions",
            GameStateName::Qa => "QA",
            GameStateName::OnesAnswer => "OnesAnswer",
            GameStateName::UseHelp => "UseHelp",
            GameStateName::WhichHelp => "WhichHelp",
            GameStateName::QaMobElimination => "QAMobElimination",
            GameStateName::MoneyOrMob => "MoneyOrMob",
            GameStateName::RoundStart => "RoundStart",
        }
    }

    /// A name exactly as the title writes it; `None` for any other.
    pub fn parse(name: &str) -> Option<GameStateName> {
        GameStateName::ALL
            .into_iter()
            .find(|state| state.as_str() == name)
    }
}

impl std::fmt::Display for GameStateName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GameStateDoc {
    root: Node,
}

impl GameStateDoc {
    /// `None` unless `xml` is an XSOR object with a `name`.
    pub fn parse(xml: &[u8]) -> Option<GameStateDoc> {
        let root = crate::xsor::table::parse(xml)?;
        let name = root.name();
        let local = name.rsplit_once(':').map_or(name, |(_, local)| local);
        if local != "XsorObject" && local != "object" {
            return None;
        }
        let doc = GameStateDoc { root };
        doc.root.child("name")?.text_value()?;
        Some(doc)
    }

    pub fn to_xml(&self) -> Vec<u8> {
        self.root.to_bytes()
    }

    /// True for `<XsorObject>` (1317), false for `<xsor:object>` (1149).
    pub fn is_xsor_object_form(&self) -> bool {
        self.root.name() == "XsorObject"
    }

    /// Set top-level `name`; false when absent.
    pub fn set(&mut self, name: &str, value: impl Display) -> bool {
        self.root
            .child_mut(name)
            .is_some_and(|node| node.set_text(value))
    }

    /// Set scalar `data` field `name`; false when absent.
    pub fn set_field(&mut self, name: &str, value: impl Display) -> bool {
        self.root
            .child_mut("data")
            .and_then(|data| data.child_mut(name))
            .is_some_and(|node| node.set_text(value))
    }

    /// State for a title name; `None` otherwise.
    pub fn state(&self) -> Option<GameStateName> {
        GameStateName::parse(&self.name())
    }

    /// The document's `name`; [`Self::parse`] rejects a document without one.
    pub fn name(&self) -> Cow<'_, str> {
        self.root
            .child("name")
            .and_then(Node::text_value)
            .unwrap_or_default()
    }

    pub fn start(&self) -> Option<Utc> {
        self.time("startTimeUtc")
    }

    pub fn end(&self) -> Option<Utc> {
        self.time("endTimeUtc")
    }

    pub fn send_get(&self) -> Option<Utc> {
        self.time("sendGetUtc")
    }

    fn time(&self, field: &str) -> Option<Utc> {
        Utc::parse(self.root.child(field)?.text_value()?.trim())
    }

    pub fn round(&self) -> Option<Round> {
        self.count("RoundNumber").map(Round)
    }

    /// The question the state is on; `None` before the round's first question.
    pub fn question(&self) -> Option<QuestionNumber> {
        QuestionNumber::new(self.reported_question()?)
    }

    /// `data.QuestionNumber` as the console reports it, 0 before the round's first question.
    pub fn reported_question(&self) -> Option<u32> {
        self.count("QuestionNumber")
    }

    pub fn ones_xuid(&self) -> Option<Cow<'_, str>> {
        self.field("OnesXuid").filter(|xuid| !xuid.is_empty())
    }

    /// `data.QuestionInfo.questionID` of a QA state.
    pub fn question_id(&self) -> Option<QuestionId> {
        let info = self.data()?.child("QuestionInfo")?;
        whole(&info.child("questionID")?.text_value()?).map(QuestionId)
    }

    /// The trimmed text of the scalar `data` field `name`.
    pub fn field(&self, name: &str) -> Option<Cow<'_, str>> {
        self.fields()
            .find(|(field, _)| *field == name)
            .map(|(_, value)| value)
    }

    /// Every scalar `data` field in document order, trimmed; nested tables are skipped.
    pub fn fields(&self) -> impl Iterator<Item = (&str, Cow<'_, str>)> {
        self.data()
            .map(Node::children)
            .unwrap_or_default()
            .iter()
            .filter(|field| field.attr("type").as_deref() != Some("table"))
            .filter_map(|field| {
                let value = field.text_value()?;
                let value = match value {
                    Cow::Borrowed(text) => Cow::Borrowed(text.trim()),
                    Cow::Owned(text) => Cow::Owned(text.trim().to_owned()),
                };
                Some((field.name(), value))
            })
    }

    fn data(&self) -> Option<&Node> {
        self.root.child("data")
    }

    /// A non-negative whole number the title writes as a float (`2`, `2.0`).
    fn count(&self, name: &str) -> Option<u32> {
        whole(&self.field(name)?)
    }
}

/// Whether `incoming` moves the round/question cursor backwards; pre-QA states never regress.
pub(crate) fn regresses(current: &[u8], incoming: &[u8]) -> bool {
    let cursor = |xml: &[u8]| {
        let doc = GameStateDoc::parse(xml)?;
        Some((doc.round()?, doc.question()))
    };
    matches!((cursor(current), cursor(incoming)), (Some(current), Some(incoming)) if incoming < current && incoming.1.is_some())
}

/// First state called `name` in the recorded corpus.
#[cfg(test)]
pub(crate) fn recorded(name: &str, xsor_object_form: bool) -> GameStateDoc {
    include_str!("../../fixtures/gamestates.txt")
        .lines()
        .filter_map(|line| GameStateDoc::parse(line.as_bytes()))
        .find(|doc| doc.name() == name && doc.is_xsor_object_form() == xsor_object_form)
        .unwrap_or_else(|| panic!("no recorded {name}"))
}

fn whole(text: &str) -> Option<u32> {
    let value = text.trim().parse::<f64>().ok()?;
    (value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= u32::MAX as f64)
        .then_some(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every distinct GameState in the recordings where the first round opens twice
    /// (`fixtures/sessions/first-round-opens-twice`), as posted and as served, plus the two
    /// captured native fixtures; one document per line.
    fn corpus() -> Vec<&'static [u8]> {
        include_bytes!("../../fixtures/gamestates.txt")
            .split(|&byte| byte == b'\n')
            .filter(|line| !line.is_empty())
            .collect()
    }

    #[test]
    fn every_recorded_game_state_round_trips_byte_for_byte() {
        let corpus = corpus();
        assert_eq!(corpus.len(), 72);
        for xml in corpus {
            let doc = GameStateDoc::parse(xml).expect("parses");
            assert_eq!(doc.to_xml(), xml, "{}", String::from_utf8_lossy(xml));
        }
    }

    /// Each recorded state and the two captured native documents as the fields the server reads
    /// from it, one per line.
    #[test]
    fn recorded_game_states_read_their_fields() {
        let shown: Vec<String> = [
            include_bytes!("../../fixtures/native-r12-qa-251317.xml").as_slice(),
            include_bytes!("../../fixtures/native-r15-round2-introductions.xml"),
        ]
        .into_iter()
        .chain(corpus())
        .map(|xml| {
            let doc = GameStateDoc::parse(xml).expect("parses");
            let time = |at: Option<Utc>| at.map_or_else(|| "-".to_owned(), |at| at.to_xsor());
            format!(
                "{} ({}): state {:?}, round {:?}, question {:?} of {:?} reported, question id {:?}, the One {:?}, {} to {}",
                doc.name(),
                if doc.is_xsor_object_form() { "1317" } else { "1149" },
                doc.state(),
                doc.round(),
                doc.question(),
                doc.reported_question(),
                doc.question_id(),
                doc.ones_xuid(),
                time(doc.start()),
                time(doc.end()),
            )
        })
        .collect();
        insta::assert_snapshot!(shown.join("\n"));
    }

    #[test]
    fn state_names_are_the_titles_exactly() {
        for state in GameStateName::ALL {
            assert_eq!(GameStateName::parse(state.as_str()), Some(state));
        }
        assert_eq!(GameStateName::Qa.as_str(), "QA");
        assert_eq!(GameStateName::parse("qa"), None, "the title's case only");
        assert_eq!(GameStateName::parse("STATE_PREGAME"), None);
        // Every state the consoles posted in the recorded corpus is one of them.
        for xml in corpus() {
            let doc = GameStateDoc::parse(xml).unwrap();
            assert!(doc.state().is_some(), "{}", doc.name());
        }
    }

    #[test]
    fn the_accessors_read_the_typed_fields() {
        let doc =
            GameStateDoc::parse(include_bytes!("../../fixtures/native-r12-qa-251317.xml")).unwrap();
        assert_eq!(doc.name(), "QA");
        assert_eq!(doc.question_id(), Some(QuestionId(251317)));
        assert!(doc.round().is_some() && doc.question().is_some());
        assert!(doc.end() > doc.start());
        assert_eq!(doc.ones_xuid().as_deref(), doc.field("OnesXuid").as_deref());
        assert_eq!(
            doc.field("QuestionInfo"),
            None,
            "tables are not scalar fields"
        );
    }

    #[test]
    fn the_question_number_zero_is_no_question_yet() {
        let doc = recorded("Introductions", false);
        assert_eq!(doc.reported_question(), Some(0), "the console's own number");
        assert_eq!(doc.question(), None);

        let qa = recorded("QA", false);
        assert_eq!(
            qa.question(),
            qa.reported_question().and_then(QuestionNumber::new)
        );
        assert!(qa.question().is_some());
    }

    #[test]
    fn the_window_reads_both_forms_and_fractional_seconds() {
        let native = "<XsorObject><name>QA</name><startTimeUtc>2026-10-11T01:33:28.000Z</startTimeUtc><endTimeUtc>2026-10-11T01:33:52.500Z</endTimeUtc><data><name>x</name></data></XsorObject>";
        let doc = GameStateDoc::parse(native.as_bytes()).unwrap();
        assert_eq!(doc.name(), "QA");
        assert_eq!(
            doc.end().unwrap().as_secs_f64() - doc.start().unwrap().as_secs_f64(),
            24.5
        );
        let doc = recorded("QA", false);
        assert!(!doc.is_xsor_object_form());
        assert_eq!(
            doc.end().unwrap().as_secs_f64() - doc.start().unwrap().as_secs_f64(),
            24.0
        );
    }

    #[test]
    fn only_named_xsor_objects_are_game_states() {
        assert!(GameStateDoc::parse(b"<XsorObject><name>QA</name></XsorObject>").is_some());
        for bad in [
            &b"<XsorObject><data type=\"table\" /></XsorObject>"[..],
            b"<other><name>QA</name></other>",
            b"<XsorObject><name>QA</name>",
        ] {
            assert_eq!(GameStateDoc::parse(bad), None);
        }
        assert_eq!(whole("2.0"), Some(2));
        assert_eq!(whole("2.5"), None);
        assert_eq!(whole("-1"), None);
    }
}
