//! Show state: what one show's consoles wrote, its elections, and server notes.

mod elections;
mod top_n;

use std::collections::HashMap;

use crate::ids::{QuestionId, QuestionNumber, Round};

pub use elections::Elections;
pub use top_n::TopN;

#[derive(Default)]
pub struct Show {
    values: HashMap<String, Vec<u8>>,
    sums: HashMap<String, f64>,
    top_ns: HashMap<String, TopN>,
    pub elections: Elections,
    /// Asked question id by (round, question): helps read keys carrying no `QuestionInfo`.
    asked: HashMap<(Round, QuestionNumber), QuestionId>,
    /// Last GameState posted per key: shadow model previous state; cleared per show.
    pub last_posted: HashMap<String, crate::gamestate::flow::StateDoc>,
}

impl Show {
    pub fn value(&self, key: &str) -> Option<&[u8]> {
        self.values.get(key).map(Vec::as_slice)
    }

    pub fn set_value(&mut self, key: String, value: Vec<u8>) {
        self.values.insert(key, value);
    }

    pub fn remove_value(&mut self, key: &str) -> bool {
        self.values.remove(key).is_some()
    }

    /// Every stored value's key with its byte length, sorted.
    pub fn keys(&self) -> Vec<(&str, usize)> {
        let mut keys: Vec<_> = self
            .values
            .iter()
            .map(|(k, v)| (k.as_str(), v.len()))
            .collect();
        keys.sort_unstable();
        keys
    }

    pub fn sum(&self, key: &str) -> Option<f64> {
        self.sums.get(key).copied()
    }

    /// Add `delta` to the sum `key`; refused when the total would not be finite.
    pub fn sum_add(&mut self, key: &str, delta: f64) -> bool {
        let total = self.sum(key).unwrap_or_default() + delta;
        if !total.is_finite() {
            return false;
        }
        self.sums.insert(key.into(), total);
        true
    }

    pub fn top_n(&self, key: &str) -> Option<&TopN> {
        self.top_ns.get(key)
    }

    /// Every board with its key, as posted (no simulated entries).
    pub fn top_ns(&self) -> impl Iterator<Item = (&str, &TopN)> {
        self.top_ns.iter().map(|(key, board)| (key.as_str(), board))
    }

    /// Post to board `key`, created on first post; frozen TheOne posts are ignored.
    pub fn top_n_post(
        &mut self,
        key: &str,
        capacity: usize,
        score: f64,
        metadata: Vec<u8>,
    ) -> bool {
        if !(1..=TopN::MAX_CAPACITY).contains(&capacity)
            || !score.is_finite()
            || !top_n::valid_metadata(&metadata)
        {
            return false;
        }
        if self.elections.is_frozen(key) {
            return true;
        }
        let top_n = self
            .top_ns
            .entry(key.into())
            .or_insert_with(|| TopN::new(capacity));
        if top_n.capacity() != capacity {
            return false;
        }
        top_n.post(score, metadata);
        true
    }

    pub fn record_asked(&mut self, round: Round, question: QuestionNumber, id: QuestionId) {
        self.asked.insert((round, question), id);
    }

    /// The id of the question the QA of `round`/`question` asked, if one was stored.
    pub fn asked(&self, round: Round, question: QuestionNumber) -> Option<QuestionId> {
        self.asked.get(&(round, question)).copied()
    }
}
