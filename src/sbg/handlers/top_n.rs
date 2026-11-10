//! TopN boards.

use std::sync::Mutex;

use crate::{store::Store, sync::lock, xsor};

use crate::sbg::{key::ParsedKey, message::Reply};

pub(in crate::sbg) fn top_n_get(key: &ParsedKey, store: &Mutex<Store>) -> Reply {
    match lock(store).top_n_read(key) {
        Some(entries) => Reply::document(xsor::top_n_values(&entries)),
        None => Reply::not_found(),
    }
}

pub(in crate::sbg) fn top_n_post(
    key: &str,
    capacity: usize,
    score: f64,
    metadata: Vec<u8>,
    store: &Mutex<Store>,
) -> Reply {
    if lock(store).top_n_post(key, capacity, score, metadata) {
        Reply::ok_empty()
    } else {
        Reply::not_found()
    }
}
