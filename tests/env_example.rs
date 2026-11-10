//! `.env.example` documents exactly the settings in `config::SETTINGS` (plus the web UI's own),
//! with their defaults. That only `config` reads the environment is enforced by clippy.

use std::{collections::BTreeMap, fs};

use onevs100_gameserver::config::SETTINGS;

/// Read by `deployment/nginx/serve.sh`, not by the server.
const UI_SETTINGS: &[&str] = &["GAMESERVER_UI_LISTEN", "GAMESERVER_SITE_LISTEN"];

/// Every `NAME=value` line, including commented-out ones (`# NAME=value` shows a default).
fn documented() -> BTreeMap<String, (String, bool)> {
    fs::read_to_string(".env.example")
        .expect(".env.example exists")
        .lines()
        .filter_map(|line| {
            let commented = line.starts_with('#');
            let (name, value) = line.trim_start_matches('#').trim_start().split_once('=')?;
            name.starts_with("GAMESERVER_")
                .then(|| (name.to_owned(), (value.to_owned(), commented)))
        })
        .collect()
}

#[test]
fn env_example_lists_exactly_the_settings() {
    let mut names: Vec<&str> = SETTINGS.iter().map(|setting| setting.name).collect();
    names.extend(UI_SETTINGS);
    names.sort_unstable();
    let documented = documented();
    assert_eq!(
        documented.keys().map(String::as_str).collect::<Vec<_>>(),
        names
    );
}

#[test]
fn commented_out_settings_show_their_defaults() {
    let documented = documented();
    for setting in SETTINGS {
        let (value, commented) = &documented[setting.name];
        if *commented {
            assert_eq!(
                setting.default.unwrap_or_default(),
                value,
                "{} in .env.example",
                setting.name
            );
        }
    }
}
