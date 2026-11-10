//! Process-wide ring buffer of notable state changes, served by the admin API.

use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

const CAPACITY: usize = 1000;

#[derive(Clone, Debug)]
pub struct Event {
    pub timestamp: String,
    pub category: &'static str,
    pub description: String,
    pub program_id: Option<String>,
}

static EVENTS: OnceLock<Mutex<VecDeque<Event>>> = OnceLock::new();

fn events() -> &'static Mutex<VecDeque<Event>> {
    EVENTS.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAPACITY)))
}

pub fn record(category: &'static str, description: impl Into<String>, program_id: Option<&str>) {
    let event = Event {
        timestamp: crate::time::now().to_grid(),
        category,
        description: description.into(),
        program_id: program_id.map(str::to_owned),
    };
    let mut events = crate::sync::lock(events());
    if events.len() == CAPACITY {
        events.pop_front();
    }
    events.push_back(event);
}

/// Newest first.
pub fn recent(limit: usize) -> Vec<Event> {
    let events = crate::sync::lock(events());
    events.iter().rev().take(limit).cloned().collect()
}
