//! Replay SBG recordings and print replies that differ; exits 1 on mismatch.
//! With `--list`, print the events of one recording instead. Filters combine; `--conn`,
//! `--xuid` and `--peer` select whole connections, `--key` selects requests and their replies.
//!
//!   sbg_replay <recording>...
//!   sbg_replay --list <recording> [--conn N] [--xuid HEX] [--peer IP] [--key TEXT]

use std::io::Write;

use onevs100_gameserver::{
    ids::{ConnId, Xuid},
    sbg::{record, replay},
};

const USAGE: &str = "usage: sbg_replay <recording>...
       sbg_replay --list <recording> [--conn N] [--xuid HEX] [--peer IP] [--key TEXT]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag, path, options @ ..] if flag == "--list" => match parse_filter(options) {
            Ok(filter) => list(path, &filter),
            Err(error) => {
                eprintln!("{error}");
                usage()
            }
        },
        [] => usage(),
        paths if paths.iter().any(|arg| arg.starts_with("--")) => usage(),
        paths => compare(paths),
    }
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2);
}

/// Why the `--list` options were refused.
#[derive(Debug)]
enum OptionError {
    Unknown(String),
    MissingValue(&'static str),
    Repeated(&'static str),
    BadValue { option: &'static str, value: String },
}

impl std::fmt::Display for OptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(option) => write!(f, "unknown option {option}"),
            Self::MissingValue(option) => write!(f, "{option} needs a value"),
            Self::Repeated(option) => write!(f, "{option} given twice"),
            Self::BadValue { option, value } => write!(f, "{option}: bad value {value:?}"),
        }
    }
}

fn parse_filter(options: &[String]) -> Result<replay::ListFilter, OptionError> {
    let mut filter = replay::ListFilter::default();
    let mut options = options.iter();
    while let Some(option) = options.next() {
        let option: &'static str = match option.as_str() {
            "--conn" => "--conn",
            "--xuid" => "--xuid",
            "--peer" => "--peer",
            "--key" => "--key",
            other => return Err(OptionError::Unknown(other.to_owned())),
        };
        let value = options
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or(OptionError::MissingValue(option))?;
        let bad = || OptionError::BadValue {
            option,
            value: value.clone(),
        };
        let set = |slot: bool| {
            if slot {
                Err(OptionError::Repeated(option))
            } else {
                Ok(())
            }
        };
        match option {
            "--conn" => {
                set(filter.conn.is_some())?;
                filter.conn = Some(ConnId(value.parse().map_err(|_| bad())?));
            }
            // As the listing and the storage export print it: 16 hex digits.
            "--xuid" => {
                set(filter.xuid.is_some())?;
                let digits = value
                    .strip_prefix("0x")
                    .or_else(|| value.strip_prefix("0X"))
                    .unwrap_or(value);
                filter.xuid = Some(Xuid(u64::from_str_radix(digits, 16).map_err(|_| bad())?));
            }
            "--peer" => {
                set(filter.peer.is_some())?;
                filter.peer = Some(value.parse().map_err(|_| bad())?);
            }
            _ => {
                set(filter.key.is_some())?;
                filter.key = Some(value.clone());
            }
        }
    }
    Ok(filter)
}

fn read(path: &str) -> record::Recording {
    let text = std::fs::read_to_string(path).unwrap_or_else(|error| unreadable(path, error));
    let recording = record::parse(&text).unwrap_or_else(|error| unreadable(path, error));
    if recording.dropped > 0 {
        eprintln!(
            "{path}: the recorder dropped {} events; replies may differ",
            recording.dropped
        );
    }
    recording
}

/// Stop the tool on a recording it cannot read.
fn unreadable(path: &str, error: impl std::fmt::Display) -> ! {
    eprintln!("{path}: {error}");
    std::process::exit(2)
}

fn list(path: &str, filter: &replay::ListFilter) {
    let recording = read(path);
    let mut out = std::io::stdout().lock();
    for line in replay::list(&recording, filter) {
        if writeln!(out, "{line}").is_err() {
            // The reader went away (`| head`); nothing more to print.
            return;
        }
    }
}

fn compare(paths: &[String]) {
    let mut differing = 0;
    for path in paths {
        let mismatches = match replay::replay(&read(path)) {
            Ok(mismatches) => mismatches,
            Err(error) => {
                eprintln!("{path}: {error}");
                std::process::exit(2);
            }
        };
        println!("{path}: {} differing replies", mismatches.len());
        for mismatch in &mismatches {
            match mismatch.cookie {
                Some(cookie) => println!("  conn {} cookie {cookie:#06x}", mismatch.conn),
                None => println!("  conn {}: frame too short for a cookie", mismatch.conn),
            }
            println!(
                "    recorded: {}",
                mismatch.expected.as_deref().unwrap_or("(none)")
            );
            println!(
                "    replayed: {}",
                mismatch.actual.as_deref().unwrap_or("(none)")
            );
        }
        differing += mismatches.len();
    }
    std::process::exit(i32::from(differing > 0));
}
