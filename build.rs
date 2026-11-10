//! Embeds the build time and commit read by `src/build_info.rs`.
//!
//! The time is `SOURCE_DATE_EPOCH` when set (reproducible builds), else the time this script runs.
//! The commit is `GAMESERVER_BUILD_COMMIT` when set (image builds have no `.git`), else git's `HEAD`.

use std::{
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[allow(
    clippy::disallowed_methods,
    reason = "a build script takes its inputs from the environment"
)]
fn env(name: &str) -> Option<String> {
    println!("cargo::rerun-if-env-changed={name}");
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (output.status.success() && !text.is_empty()).then(|| text.to_owned())
}

fn main() {
    // A new build time when the code changes, not on every build.
    for path in [
        "src",
        "assets",
        "keys",
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
    ] {
        println!("cargo::rerun-if-changed={path}");
    }
    // The reflog changes on every commit, amend, checkout and reset; the loose ref covers a
    // repository without one. A missing path would make cargo rerun this script on every build.
    let head_ref = git(&["symbolic-ref", "-q", "HEAD"]);
    for name in ["HEAD", "logs/HEAD"].into_iter().chain(head_ref.as_deref()) {
        if let Some(path) = git(&["rev-parse", "--git-path", name])
            && Path::new(&path).exists()
        {
            println!("cargo::rerun-if-changed={path}");
        }
    }

    let built: i64 = match env("SOURCE_DATE_EPOCH") {
        Some(value) => value
            .parse()
            .ok()
            .filter(|secs| *secs >= 0)
            .unwrap_or_else(|| panic!("SOURCE_DATE_EPOCH must be unix seconds, got {value:?}")),
        None => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
            .expect("the clock is after 1970"),
    };
    println!("cargo::rustc-env=GAMESERVER_BUILT_UNIX={built}");

    let commit = env("GAMESERVER_BUILD_COMMIT")
        .map(|commit| ("GAMESERVER_BUILD_COMMIT", commit))
        .or_else(|| git(&["rev-parse", "HEAD"]).map(|commit| ("git rev-parse HEAD", commit)));
    if let Some((source, commit)) = commit {
        // SHA-1 or SHA-256 object ids, abbreviated or in full.
        assert!(
            (7..=64).contains(&commit.len()) && commit.bytes().all(|b| b.is_ascii_hexdigit()),
            "{source} must give a git commit id, got {commit:?}"
        );
        println!(
            "cargo::rustc-env=GAMESERVER_COMMIT_ID={}",
            commit.to_ascii_lowercase()
        );
    }
}
