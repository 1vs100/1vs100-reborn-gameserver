//! question-import --source otdb --out bank.json [--categories 9,11] [--max-question-chars 80]
//!                 [--max-answer-chars 20] [--allow-non-ascii]
//!
//! Fetches every category of the source (or those listed), fits the questions to the title and
//! writes a bank file for `POST /admin/questions/import`. The report goes to stderr. OpenTriviaDB
//! allows one call per 5 s and 50 questions per call, so a full import (all categories in one
//! pull) takes about 10 minutes; `--categories` (the source's own ids) pulls each listed category
//! on its own. The bank's categories are ours (`categories.rs`), whatever the source calls them.

use std::process::ExitCode;

use question_import::{bank, fit::Limits, sources};

const USAGE: &str = "usage: question-import --source <id> --out <file> [--categories <id,id,...>] \
[--max-question-chars <n>] [--max-answer-chars <n>] [--allow-non-ascii]";

#[derive(Debug)]
struct Args {
    source: String,
    out: String,
    categories: Option<Vec<String>>,
    limits: Limits,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let (mut source, mut out, mut categories) = (None, None, None);
    let mut limits = Limits::default();
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        let number = |value: String| {
            value
                .parse::<usize>()
                .map_err(|_| format!("{flag} takes a number, not {value:?}"))
        };
        match flag.as_str() {
            "--source" => source = Some(value()?),
            "--out" => out = Some(value()?),
            "--categories" => {
                categories = Some(value()?.split(',').map(|id| id.trim().to_owned()).collect())
            }
            "--max-question-chars" => limits.question_chars = number(value()?)?,
            "--max-answer-chars" => limits.answer_chars = number(value()?)?,
            "--allow-non-ascii" => limits.ascii_only = false,
            _ => return Err(format!("unknown argument {flag:?}")),
        }
    }
    Ok(Args {
        source: source.ok_or("--source is required")?,
        out: out.ok_or("--out is required")?,
        categories,
        limits,
    })
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Some(mut source) = sources::by_id(&args.source) else {
        eprintln!(
            "unknown source {:?}; known: {}",
            args.source,
            sources::IDS.join(", ")
        );
        return ExitCode::from(2);
    };
    let imported = bank::import(
        source.as_mut(),
        args.categories.as_deref(),
        args.limits,
        &mut |category| eprintln!("fetching {category}"),
    );
    let imported = match imported {
        Ok(imported) => imported,
        Err(error) => {
            eprintln!("import failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = std::fs::write(&args.out, bank::to_json(&imported.bank)) {
        eprintln!("{}: {error}", args.out);
        return ExitCode::FAILURE;
    }
    eprint!("{}", imported.report.render(&imported.bank));
    eprintln!("written to {}", args.out);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Result<Args, String> {
        parse(line.split_whitespace().map(str::to_owned))
    }

    #[test]
    fn arguments_are_read_and_checked() {
        let parsed = args("--source otdb --out b.json --categories 9,11 --max-question-chars 100 --allow-non-ascii").unwrap();
        assert_eq!(
            (parsed.source.as_str(), parsed.out.as_str()),
            ("otdb", "b.json")
        );
        assert_eq!(parsed.categories.unwrap(), ["9", "11"]);
        assert_eq!(
            parsed.limits,
            Limits {
                question_chars: 100,
                answer_chars: 20,
                ascii_only: false
            }
        );
        assert_eq!(
            args("--source otdb --out b.json").unwrap().limits,
            Limits::default()
        );
        assert!(args("--out b.json").unwrap_err().contains("--source"));
        assert!(
            args("--source otdb --out")
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(
            args("--source otdb --out b --max-answer-chars x")
                .unwrap_err()
                .contains("number")
        );
        assert!(
            args("--source otdb --out b --frobnicate")
                .unwrap_err()
                .contains("unknown")
        );
    }
}
