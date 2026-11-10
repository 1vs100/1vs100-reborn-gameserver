//! Each magic literal has one home in production code. Tests may hold them verbatim (recorded
//! keys), so only code outside `#[cfg(test)]` items is checked, and comments are not.

mod support;

use support::source::{production_lines, production_sources};

/// (literal, the one file it may appear in, what it is)
const HOMES: &[(&str, &str, &str)] = &[
    (
        "00000000-0000-0000-0000-000000000001",
        "src/schedule.rs",
        "the canonical session: use schedule::CANONICAL_SESSION or SessionId::is_canonical",
    ),
    (
        "0xF000_0000_0000_0001",
        "src/primetime.rs",
        "the simulated Mob's identities: use primetime::MOB_SELECTION_XUID_BASE",
    ),
    (
        "0xE000_FEED",
        "src/primetime.rs",
        "the simulator's member ids: primetime::VIRTUAL_XUID_BASE",
    ),
    (
        "251317",
        "src/questions.rs",
        "the captured question: the built-in bank's row",
    ),
    (
        "251_317",
        "src/questions.rs",
        "the captured question: the built-in bank's row",
    ),
    (
        "jiff::",
        "src/time.rs",
        "time zones: use time::{Zone, LocalDate, LocalTime}; the rest of the code speaks `time`",
    ),
];

#[test]
fn magic_literals_stay_in_their_one_home() {
    let mut misplaced = Vec::new();
    for (name, text) in production_sources() {
        for (number, line) in production_lines(&text) {
            for (literal, home, what) in HOMES {
                if line.contains(literal) && name != *home {
                    misplaced.push(format!("{name}:{number}: {literal} ({what})"));
                }
            }
        }
    }
    assert!(misplaced.is_empty(), "{misplaced:#?}");
}

#[test]
fn test_items_and_comments_are_not_production_lines() {
    let text = "fn a() {}\n// 0xE000_FEED\n#[cfg(test)]\nconst X: u8 = 1;\n#[cfg(test)]\nmod tests {\n    fn b() {\n    }\n}\nfn c() {}\n";
    let kept: Vec<_> = production_lines(text)
        .into_iter()
        .map(|(_, line)| line)
        .collect();
    assert_eq!(kept, ["fn a() {}", "fn c() {}"]);
}
