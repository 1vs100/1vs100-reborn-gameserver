//! Role elections across consoles: one id per election window, and The One fixed at first read.

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use crate::{
    sbg::key::{Key, ParsedKey},
    time::Utc,
};

#[derive(Default)]
pub struct Elections {
    ids: ElectionIds,
    /// Lowercased `...TopN.<session>.PRIMETIME.<id>.TheOne` keys that a client has already read.
    /// Every client scores the same election window at its own time (two consoles start their
    /// local election seconds apart), so the first read fixes the winner for everybody; later
    /// posts are acknowledged but ignored, otherwise each console elects itself The One.
    frozen_ones: HashSet<String>,
}

/// A role election's window id (`PRIMETIME.<id>` in SBG keys) is the posting console's local
/// game-clock second, so two consoles run the same election under different ids (round 1 both
/// `30`, round 2 `381` vs `387`). Candidacy posts that start within [`ELECTION_WINDOW`] of the
/// first one are aliased to its id so every key of the round (TheOne, MobSelection, TheMob,
/// player counts, avatars) resolves to the same entry.
#[derive(Default)]
struct ElectionIds {
    epoch: Option<(u32, Utc)>,
    alias: HashMap<u32, u32>,
}

/// Shorter than ElectHost (51 s) so an immediate re-election opens a new epoch.
const ELECTION_WINDOW: Duration = Duration::from_secs(40);

impl Elections {
    /// An election key (`...PRIMETIME.<id>...`, [`Key::Election`]) with its id folded into the id of the
    /// election epoch it belongs to; any other key is returned unchanged. `candidacy` marks a
    /// Top1 (`...TopN.<session>.PRIMETIME.<id>.TheOne`) post, the only request that may open an
    /// epoch or alias a new id into the current one.
    pub fn fold(&mut self, key: ParsedKey, candidacy: bool, now: Utc) -> ParsedKey {
        let Key::Election { id, .. } = key.kind else {
            return key;
        };
        let ids = &mut self.ids;
        let canonical = match ids.alias.get(&id) {
            Some(canonical) => *canonical,
            None if candidacy => {
                let canonical = match ids.epoch {
                    Some((epoch_id, started))
                        if now.saturating_duration_since(started) < ELECTION_WINDOW =>
                    {
                        epoch_id
                    }
                    _ => {
                        ids.epoch = Some((id, now));
                        id
                    }
                };
                ids.alias.insert(id, canonical);
                canonical
            }
            None => id,
        };
        if canonical == id {
            key
        } else {
            key.with_election_id(canonical)
        }
    }

    /// Fix The One of the election `key` names: later posts to it are ignored.
    pub fn freeze(&mut self, key: &str) {
        self.frozen_ones.insert(key.to_ascii_lowercase());
    }

    pub fn is_frozen(&self, key: &str) -> bool {
        self.frozen_ones.contains(&key.to_ascii_lowercase())
    }
}
