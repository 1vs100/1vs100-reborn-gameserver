//! GameState GET hold: answer at execute time unless the stored state would look stale.

use std::time::Duration;

use super::GameStateDoc;
use crate::time::Utc;

/// Reads happen this long before the current state ends; answers ending by then look stale.
const GET_EXECUTION_DELTA: Duration = Duration::from_millis(3_175);
/// Requests arrive at most this long before execute time; later is a clock problem.
const MAX_EXECUTE_LEAD: Duration = Duration::from_secs(105);
/// Wait after execute time for the next state; the One posts on its own clock.
const NEXT_STATE_GRACE: Duration = Duration::from_millis(1_500);

/// Execute time of a GET; `None` when the title could not have meant it.
pub fn execute_time(execute_at: Option<Utc>, now: Utc) -> Option<Utc> {
    execute_at.filter(|at| at.saturating_duration_since(now) <= MAX_EXECUTE_LEAD)
}

/// Whether `stored` looks stale at `at`: nothing stored, or ending within the read-ahead.
pub fn stale(stored: Option<&[u8]>, at: Utc) -> bool {
    stored.is_none_or(|value| {
        GameStateDoc::parse(value)
            .and_then(|doc| doc.end())
            .is_some_and(|end| at + GET_EXECUTION_DELTA >= end)
    })
}

/// Deadline a GET executed at `at` waits for the next state.
pub fn wait_until(at: Utc) -> Utc {
    at + NEXT_STATE_GRACE
}

#[cfg(test)]
mod tests {
    use super::*;

    const QA: &[u8] = include_bytes!("../../fixtures/native-r12-qa-251317.xml");

    #[test]
    fn an_execute_time_too_far_ahead_is_not_held_for() {
        let now = Utc::from_unix_secs(1_000);
        let at = now + MAX_EXECUTE_LEAD;
        assert_eq!(execute_time(Some(at), now), Some(at));
        assert_eq!(execute_time(Some(at + Duration::from_secs(1)), now), None);
        assert_eq!(execute_time(None, now), None);
    }

    #[test]
    fn a_state_is_stale_from_the_execution_delta_before_its_end() {
        let end = GameStateDoc::parse(QA).unwrap().end().unwrap();
        assert!(stale(None, end), "nothing stored");
        assert!(stale(Some(QA), end - GET_EXECUTION_DELTA));
        assert!(!stale(
            Some(QA),
            end - GET_EXECUTION_DELTA - Duration::from_millis(1)
        ));
        assert_eq!(
            wait_until(end),
            end + NEXT_STATE_GRACE,
            "the next state is waited for past the execute time"
        );
    }
}
