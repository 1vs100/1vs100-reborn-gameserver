//! Title error reports under `Log.UserError`, one format string on both builds.

use std::borrow::Cow;

use quick_xml::escape::unescape;

/// The labels of the format string, in order; a value runs to the next `, <label>: `.
const LABELS: [&str; 19] = [
    "errorName",
    "errorReason",
    "Fatal",
    "Build",
    "GameVersion",
    "Players",
    "Invite",
    "QOS",
    "TS",
    "UTC",
    "@",
    "GE",
    "PID",
    "State",
    "Sub",
    "R",
    "Q",
    "key",
    "NB",
];

#[derive(Debug, PartialEq)]
pub(crate) struct UserError<'a> {
    pub name: &'a str,
    pub reason: &'a str,
    /// `No`/`None` for a report, `Fatal`, `Exit` or `Crash` (or `?`) when the title gives up.
    pub fatal: &'a str,
    /// The console's clock when it wrote the report, `YYYY-MM-DDTHH:MM:SS.fffZ`; absent from a
    /// report without one.
    pub utc: Option<&'a str>,
    pub state: &'a str,
    pub substate: &'a str,
    pub round: &'a str,
    pub question: &'a str,
    /// The SBG key the error is about (`NO_MSG` when none).
    pub key: &'a str,
}

impl UserError<'_> {
    pub fn is_fatal(&self) -> bool {
        !matches!(self.fatal, "No" | "None")
    }
}

/// The report's text: the document's content, unescaped.
pub(crate) fn text(value: &[u8]) -> Option<Cow<'_, str>> {
    let value = std::str::from_utf8(value).ok()?;
    let start = value.find('>')? + 1;
    let end = value.rfind("</")?;
    unescape(value.get(start..end)?).ok()
}

pub(crate) fn parse(text: &str) -> Option<UserError<'_>> {
    Some(UserError {
        name: field(text, "errorName")?,
        reason: field(text, "errorReason")?,
        fatal: field(text, "Fatal")?,
        utc: field(text, "UTC"),
        state: field(text, "State")?,
        substate: field(text, "Sub")?,
        round: field(text, "R")?,
        question: field(text, "Q")?,
        key: field(text, "key")?,
    })
}

fn field<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    let index = LABELS.iter().position(|l| *l == label)?;
    let marker = format!("{label}: ");
    let start = if index == 0 {
        text.strip_prefix(&marker).map(|_| marker.len())?
    } else {
        text.find(&format!(", {marker}"))? + 2 + marker.len()
    };
    // Values can hold ", " themselves (`Players`, `errorReason`); only the next label ends one.
    let rest = &text[start..];
    let end = LABELS[index + 1..]
        .iter()
        .find_map(|next| rest.find(&format!(", {next}: ")))
        .or_else(|| rest.find(", stack: "))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorded 1149 lobby report.
    const AD_BREAK_STATS: &[u8] = b"<xsor:object xmlns:xsor=\"http://schemas.xbox.com/xsor\" xmlns=\"http://schemas.xbox.com/SBG\" name=\"SBG.Actions\">errorName: LobbyFPP, errorReason: Connected(true) Zone(Lobby_PT) State(0)(STATE_ERROR) Error(210)(Could not contact Massive servers), Fatal: No, Build: 2.0.1149.0.02, GameVersion: 2.02, Players:  PlayerPhat(Crowd)[16140907667073616695](1), Invite: NO, QOS: 9; 10; 83886080; 83886080, TS: 102.822, UTC: 2026-10-23T00:28:54.397Z, @: 00:01:13.397, GE: PT, PID: 00000000-0000-0000-0000-000000000001, State: Lobby, Sub: NO_SUBSTATE, R: 0, Q: 0, key: NO_MSG, NB: 0, stack: Ads.lua.1850; Ads.lua.967; Ads.lua.755; StateLobby.lua.497; task.lua.135,</xsor:object>";

    #[test]
    fn a_recorded_report_reads_its_fields() {
        let text = text(AD_BREAK_STATS).unwrap();
        let report = parse(&text).unwrap();
        assert_eq!(
            report,
            UserError {
                name: "LobbyFPP",
                reason: "Connected(true) Zone(Lobby_PT) State(0)(STATE_ERROR) Error(210)(Could not contact Massive servers)",
                fatal: "No",
                utc: Some("2026-10-23T00:28:54.397Z"),
                state: "Lobby",
                substate: "NO_SUBSTATE",
                round: "0",
                question: "0",
                key: "NO_MSG",
            }
        );
        assert!(!report.is_fatal());
    }

    #[test]
    fn a_value_with_commas_runs_to_the_next_label_and_escapes_are_undone() {
        let value = b"<xsor:object>errorName: SBG_E_UNEXPECTED_STATE, errorReason: a, b &amp; c, Fatal: Exit, Build: 2.0.1317.0.02, GameVersion: 2.02, Players: P1(TheOne)[1](1), P2(Mob)[2](1), Invite: NO, QOS: 1, TS: 1, UTC: x, @: y, GE: PT, PID: p, State: QA, Sub: NO_SUBSTATE, R: 1, Q: 4, key: v1.Session.MGS.1vs100.p.GameState, NB: Get; 1, stack: a.lua.1,</xsor:object>";
        let text = text(value).unwrap();
        let report = parse(&text).unwrap();
        assert_eq!(report.reason, "a, b & c");
        assert_eq!(
            (report.state, report.round, report.question),
            ("QA", "1", "4")
        );
        assert_eq!(report.key, "v1.Session.MGS.1vs100.p.GameState");
        assert!(report.is_fatal());
    }

    #[test]
    fn other_text_is_not_a_report() {
        assert_eq!(parse("hello"), None);
        assert_eq!(text(b"not xml"), None);
    }
}
