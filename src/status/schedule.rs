//! The shows to come, as consoles will be served them.

use std::time::SystemTime;

use super::{ShowSummary, live::planned_summary};
use crate::{app::App, sync::lock, time::Utc};

/// Shows in [`snapshot`]: the running one, then those after it.
pub(super) const SHOWS: usize = 20;

/// Empty without a programme: the title is then told of no fixed show.
pub(super) fn snapshot(app: &App, now: SystemTime) -> Vec<ShowSummary> {
    let now = Utc::from(now);
    let shows: Vec<_> = {
        let mut schedule = lock(&app.schedule);
        if schedule.programme().is_none() {
            return Vec::new();
        }
        schedule
            .shows(now)
            .iter()
            .filter(|show| show.plan.end > now)
            .take(SHOWS)
            .map(|show| show.plan.clone())
            .collect()
    };
    let store = lock(&app.store);
    shows
        .iter()
        .map(|show| planned_summary(show, &store))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{programme::Programme, storage::Storage, store::Store};

    fn app(programme: Option<Programme>) -> App {
        let storage: Arc<dyn Storage> = Arc::new(crate::storage::MemoryStorage::default());
        if let Some(programme) = programme {
            storage.set_programme(&programme).unwrap();
        }
        let schedule =
            crate::schedule::Schedule::load(Default::default(), Arc::clone(&storage)).unwrap();
        App::new(
            crate::sbg::Settings::new(crate::wire::Protocol::V1317),
            Arc::new(std::sync::Mutex::new(schedule)),
            Store::bootstrap().with_storage(storage),
            crate::matchmaking::Members::default(),
        )
    }

    #[test]
    fn the_running_show_and_those_after_it_are_listed() {
        let app = app(Some(Programme::default()));
        let now = Utc::parse("2026-10-27T23:50:00Z").unwrap();
        insta::assert_json_snapshot!(snapshot(&app, now.into()));
    }

    #[test]
    fn without_a_programme_no_show_is_listed() {
        let app = app(None);
        let now = Utc::parse("2026-10-27T12:00:00Z").unwrap();
        assert!(snapshot(&app, now.into()).is_empty());
    }
}
