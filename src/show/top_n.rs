//! A TopN board: up to `capacity` entries ranked by score, each with its `<Metadata>` document.

use quick_xml::{
    Reader,
    escape::unescape,
    events::{BytesStart, Event},
};

pub struct TopN {
    capacity: usize,
    entries: Vec<TopNEntry>,
}

struct TopNEntry {
    score: f64,
    metadata: Vec<u8>,
}

impl TopN {
    /// The largest board the title posts (the Mob's 100 and The One).
    pub const MAX_CAPACITY: usize = 101;

    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Vec::with_capacity(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// A post replaces the entry with the same metadata, or adds one; past the capacity the
    /// lowest scores drop out.
    pub fn post(&mut self, score: f64, metadata: Vec<u8>) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.metadata == metadata)
        {
            entry.score = score;
        } else {
            self.entries.push(TopNEntry { score, metadata });
        }
        if self.entries.len() > self.capacity {
            // Stable sort keeps first-arrival order among equal scores.
            let mut ranked = std::mem::take(&mut self.entries);
            ranked.sort_by(|left, right| right.score.total_cmp(&left.score));
            ranked.truncate(self.capacity);
            self.entries = ranked;
        }
    }

    /// The entries as `(score, metadata)`, in posting order.
    pub fn pairs(&self) -> Vec<(f64, Vec<u8>)> {
        self.entries
            .iter()
            .map(|entry| (entry.score, entry.metadata.clone()))
            .collect()
    }
}

pub fn valid_metadata(metadata: &[u8]) -> bool {
    let mut reader = Reader::from_reader(metadata);
    let mut elements = Vec::<Vec<u8>>::new();
    let mut saw_root = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                if !valid_element(&start, elements.is_empty(), &mut saw_root) {
                    return false;
                }
                elements.push(start.name().as_ref().to_vec());
            }
            Ok(Event::Empty(start)) => {
                if !valid_element(&start, elements.is_empty(), &mut saw_root) {
                    return false;
                }
            }
            Ok(Event::End(end)) => {
                if elements.pop().as_deref() != Some(end.name().as_ref()) {
                    return false;
                }
            }
            Ok(Event::Text(text)) => {
                let Ok(text) = text.xml_content() else {
                    return false;
                };
                if elements.is_empty() && !text.trim().is_empty() {
                    return false;
                }
            }
            Ok(Event::CData(data)) => {
                if elements.is_empty() || data.xml_content().is_err() {
                    return false;
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                let Ok(reference) = reference.xml10_content() else {
                    return false;
                };
                if elements.is_empty() || unescape(&format!("&{reference};")).is_err() {
                    return false;
                }
            }
            Ok(Event::Comment(_)) if !elements.is_empty() => {}
            Ok(Event::Eof) => return saw_root && elements.is_empty(),
            // Reject document syntax, depth-0 comments, and malformed XML around the sole root.
            Ok(Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_)) | Err(_) => {
                return false;
            }
        }
    }
}

/// Validates an opening tag; at depth 0 it must be the sole `<Metadata>` root.
fn valid_element(start: &BytesStart<'_>, at_root: bool, saw_root: &mut bool) -> bool {
    if !valid_start(start) || (at_root && (*saw_root || start.name().as_ref() != b"Metadata")) {
        return false;
    }
    *saw_root = true;
    true
}

fn valid_start(start: &BytesStart<'_>) -> bool {
    start
        .attributes()
        .with_checks(true)
        .all(|attribute| match attribute {
            Ok(attribute) => attribute.unescape_value().is_ok(),
            Err(_) => false,
        })
}
