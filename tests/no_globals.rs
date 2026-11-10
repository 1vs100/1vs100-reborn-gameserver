//! No process-wide state: every `static`
//! and `thread_local!` in production code is listed here as allowed, with why; everything else
//! lives in `App`. A new static fails, and so does a row whose static is gone.

mod support;

use std::collections::BTreeSet;

use support::source::{production_lines, production_sources};

/// (file, static, why it may stay)
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "src/logs.rs",
        "SESSION_DIR",
        "the tracing writer's directory",
    ),
    (
        "src/matchmaking/handlers.rs",
        "NEXT_CLIENT_ID",
        "an id counter",
    ),
    ("src/sbg/mod.rs", "NEXT_CONN", "an id counter"),
    (
        "src/sbg/handlers/game_state.rs",
        "SEQ",
        "an id counter (GameState dump file numbers)",
    ),
    (
        "src/timeline.rs",
        "EVENTS",
        "the operator's event log: written by game code, read only by the admin page",
    ),
];

/// The name a production line declares as `static`, if any (`static X`, `pub static X`,
/// `pub(crate) static X`, and the statics inside `thread_local!`).
fn declared_static(line: &str) -> Option<&str> {
    let rest = line.trim_start();
    let rest = rest
        .strip_prefix("pub(crate) ")
        .or_else(|| rest.strip_prefix("pub "))
        .unwrap_or(rest);
    let rest = rest.strip_prefix("static ")?;
    let rest = rest.strip_prefix("mut ").unwrap_or(rest);
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    Some(&rest[..end])
}

#[test]
fn every_static_is_allowed() {
    let found: BTreeSet<(String, String)> = production_sources()
        .into_iter()
        .flat_map(|(file, text)| {
            production_lines(&text)
                .into_iter()
                .filter_map(|(_, line)| declared_static(line).map(str::to_owned))
                .map(|name| (file.clone(), name))
                .collect::<Vec<_>>()
        })
        .collect();
    let listed: BTreeSet<(String, String)> = ALLOWED
        .iter()
        .map(|(file, name, _)| ((*file).to_owned(), (*name).to_owned()))
        .collect();
    let new: Vec<_> = found.difference(&listed).collect();
    let gone: Vec<_> = listed.difference(&found).collect();
    assert!(
        new.is_empty(),
        "new process-wide state, put it in App instead: {new:#?}"
    );
    assert!(
        gone.is_empty(),
        "moved or removed, delete its row here: {gone:#?}"
    );
}

#[test]
fn declarations_are_recognised() {
    assert_eq!(declared_static("static A: u8 = 1;"), Some("A"));
    assert_eq!(
        declared_static("    pub(crate) static B_C: X = x;"),
        Some("B_C")
    );
    assert_eq!(
        declared_static("    static CTX: Cell<u8> = const { .. };"),
        Some("CTX")
    );
    assert_eq!(declared_static("let s: &'static str = \"\";"), None);
    assert_eq!(declared_static("fn f() -> &'static str {"), None);
}
