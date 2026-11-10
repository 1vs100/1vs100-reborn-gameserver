//! Locking shared by every service; recovers after a holder panics.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Lock `mutex`, recovering after a holder panics so one failure wedges nothing.
#[allow(clippy::disallowed_methods)] // the one place that locks directly
pub fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };

    #[test]
    fn an_unsized_value_can_be_locked() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer: Arc<Mutex<dyn Write + Send>> = Arc::clone(&buffer) as _;
        super::lock(&writer).write_all(b"frame").unwrap();
        assert_eq!(*super::lock(&buffer), b"frame");
    }
}
