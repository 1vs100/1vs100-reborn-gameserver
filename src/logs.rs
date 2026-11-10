//! Per-session log directory teed to stderr and tailed by the admin API.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use tracing_subscriber::fmt::MakeWriter;

const MAIN_LOG: &str = "gameserver.log";

static SESSION_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn session_dir() -> Option<&'static Path> {
    SESSION_DIR.get().map(PathBuf::as_path)
}

/// Create this session's log directory and return a writer that tees to it.
/// Falls back to stderr only (with a warning on stderr) if the directory
/// cannot be created.
pub fn open_session(base: &Path) -> TeeWriter {
    let secs = crate::time::now().unix_secs();
    let dir = base.join(secs.to_string());
    let file = fs::create_dir_all(&dir).and_then(|()| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(MAIN_LOG))
    });
    match file {
        Ok(file) => {
            let _ = SESSION_DIR.set(dir);
            TeeWriter(Some(Arc::new(Mutex::new(file))))
        }
        Err(error) => {
            eprintln!("log directory {} unavailable: {error}", dir.display());
            TeeWriter(None)
        }
    }
}

#[derive(Clone)]
pub struct TeeWriter(Option<Arc<Mutex<File>>>);

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::stderr().write_all(buf)?;
        if let Some(file) = &self.0 {
            let mut file = crate::sync::lock(file);
            // The file is a copy of stderr: a failed write must not fail the logging call.
            let _ = file.write_all(&strip_ansi(buf));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stderr().flush()
    }
}

impl<'a> MakeWriter<'a> for TeeWriter {
    type Writer = TeeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Drop `ESC [ ... <letter>` colour sequences so the file stays readable.
pub(crate) fn strip_ansi(buf: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(buf.len());
    let mut bytes = buf.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == 0x1b {
            for next in bytes.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(byte);
        }
    }
    out
}

/// Plain files directly inside the session directory, sorted by name.
pub fn list() -> Vec<(String, u64)> {
    let Some(dir) = session_dir() else {
        return Vec::new();
    };
    let mut files: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            meta.is_file()
                .then(|| (entry.file_name().to_string_lossy().into_owned(), meta.len()))
        })
        .collect();
    files.sort();
    files
}

#[derive(Debug, PartialEq, Eq)]
pub enum TailError {
    InvalidName,
    NotFound,
}

/// A record's severity, most severe first: a filter keeps its level and every one before it.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "ERROR" => Self::Error,
            "WARN" => Self::Warn,
            "INFO" => Self::Info,
            "DEBUG" => Self::Debug,
            "TRACE" => Self::Trace,
            _ => return None,
        })
    }
}

/// Which records a tail keeps.
#[derive(Debug, Default)]
pub struct TailFilter<'a> {
    /// The least severe level kept; records with no level are dropped when set.
    pub level: Option<LogLevel>,
    /// Text a line of the record must contain, ignoring ASCII case.
    pub contains: Option<&'a str>,
}

/// Last `lines` lines of a file listed by [`list`], of the records `filter` keeps. Only bare
/// file names are accepted, so the request can never leave the session directory.
pub fn tail(name: &str, lines: usize, filter: &TailFilter) -> Result<String, TailError> {
    if name.is_empty() || name.contains(['/', '\\', '\0']) || name.starts_with('.') {
        return Err(TailError::InvalidName);
    }
    let dir = session_dir().ok_or(TailError::NotFound)?;
    let text = fs::read(dir.join(name)).map_err(|_| TailError::NotFound)?;
    Ok(filtered_tail(
        &String::from_utf8_lossy(&text),
        lines,
        filter,
    ))
}

/// The level of a line that starts a record: a timestamp, then the level word.
fn record_level(line: &str) -> Option<LogLevel> {
    if !line.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    LogLevel::parse(line.split_whitespace().nth(1)?)
}

fn contains_ignoring_ascii_case(line: &str, needle: &str) -> bool {
    needle.is_empty()
        || line
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// A record is a line with a level and the lines up to the next one, so a multi-line
/// message is kept or dropped whole.
fn filtered_tail(text: &str, lines: usize, filter: &TailFilter) -> String {
    let keeps = |level: Option<LogLevel>, record: &[&str]| {
        let level_kept = match (filter.level, level) {
            (None, _) => true,
            (Some(least), Some(level)) => level <= least,
            (Some(_), None) => false,
        };
        level_kept
            && filter.contains.is_none_or(|needle| {
                record
                    .iter()
                    .any(|line| contains_ignoring_ascii_case(line, needle))
            })
    };
    let mut kept = Vec::new();
    let mut record = Vec::new();
    let mut level = None;
    for line in text.lines() {
        if let Some(next) = record_level(line) {
            if keeps(level, &record) {
                kept.append(&mut record);
            }
            record.clear();
            level = Some(next);
        }
        record.push(line);
    }
    if keeps(level, &record) {
        kept.append(&mut record);
    }
    let start = kept.len().saturating_sub(lines);
    let mut out = kept[start..].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{LogLevel, TailFilter, filtered_tail, strip_ansi};

    const LOG: &str = "\
2026-11-09T10:00:00.000001Z  INFO onevs100_gameserver: listening
2026-11-09T10:00:01.000001Z DEBUG onevs100_gameserver::sbg: SBG register
2026-11-09T10:00:02.000001Z  WARN onevs100_gameserver::sbg: SBG request answered NOT_FOUND
  key: v1.Session.MGS.1vs100.p.GameState
2026-11-09T10:00:03.000001Z ERROR onevs100_gameserver::storage: console error report not saved
2026-11-09T10:00:04.000001Z TRACE onevs100_gameserver::http: HTTP request
";

    #[test]
    fn a_level_keeps_itself_and_every_more_severe_record_with_its_lines() {
        let filter = TailFilter {
            level: Some(LogLevel::Warn),
            contains: None,
        };
        insta::assert_snapshot!(filtered_tail(LOG, 100, &filter), @r"
        2026-11-09T10:00:02.000001Z  WARN onevs100_gameserver::sbg: SBG request answered NOT_FOUND
          key: v1.Session.MGS.1vs100.p.GameState
        2026-11-09T10:00:03.000001Z ERROR onevs100_gameserver::storage: console error report not saved
        ");
    }

    #[test]
    fn text_matches_any_line_of_a_record_ignoring_case() {
        let filter = TailFilter {
            level: None,
            contains: Some("GAMESTATE"),
        };
        insta::assert_snapshot!(filtered_tail(LOG, 100, &filter), @r"
        2026-11-09T10:00:02.000001Z  WARN onevs100_gameserver::sbg: SBG request answered NOT_FOUND
          key: v1.Session.MGS.1vs100.p.GameState
        ");
    }

    #[test]
    fn the_line_count_applies_after_filtering() {
        let filter = TailFilter {
            level: Some(LogLevel::Debug),
            contains: None,
        };
        insta::assert_snapshot!(filtered_tail(LOG, 2, &filter), @r"
          key: v1.Session.MGS.1vs100.p.GameState
        2026-11-09T10:00:03.000001Z ERROR onevs100_gameserver::storage: console error report not saved
        ");
        assert_eq!(filtered_tail(LOG, 100, &TailFilter::default()), LOG);
    }

    #[test]
    fn an_indented_line_continues_its_record() {
        let log = "2026-11-09T10:00:00.000001Z ERROR x: failed\n  42 WARN retries left\n";
        let filter = TailFilter {
            level: Some(LogLevel::Error),
            contains: None,
        };
        assert_eq!(filtered_tail(log, 100, &filter), log);
    }

    #[test]
    fn strips_colour_sequences_only() {
        assert_eq!(
            strip_ansi(b"\x1b[2m2026\x1b[0m \x1b[32m INFO\x1b[0m ok"),
            b"2026  INFO ok"
        );
    }
}
