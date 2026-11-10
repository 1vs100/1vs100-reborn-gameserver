//! This binary's build, as `build.rs` embedded it.

use std::fmt;

use crate::time::Utc;

/// A git commit id, lowercase hex.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommitId(&'static str);

impl CommitId {
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    pub built: Utc,
    /// `None` when built outside a git checkout without `GAMESERVER_BUILD_COMMIT`.
    pub commit: Option<CommitId>,
}

pub fn current() -> BuildInfo {
    BuildInfo {
        built: Utc::from_unix_secs(
            env!("GAMESERVER_BUILT_UNIX")
                .parse()
                .expect("build.rs writes unix seconds"),
        ),
        commit: option_env!("GAMESERVER_COMMIT_ID").map(CommitId),
    }
}
