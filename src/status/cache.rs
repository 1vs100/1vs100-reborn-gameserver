//! A value rebuilt at most once per lifetime, so public polling cannot drive the game's locks or
//! storage harder than that.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

pub struct Cached<T> {
    slot: Mutex<Option<(Instant, T)>>,
}

impl<T> Default for Cached<T> {
    fn default() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }
}

impl<T: Clone> Cached<T> {
    /// The cached value while it is younger than its lifetime, else `build`'s; `build` gives the
    /// value with its lifetime. Callers wait for one build instead of each running their own.
    pub fn get(&self, build: impl FnOnce() -> (T, Duration)) -> T {
        let mut slot = crate::sync::lock(&self.slot);
        if let Some((until, value)) = slot.as_ref()
            && Instant::now() < *until
        {
            return value.clone();
        }
        let (value, lifetime) = build();
        *slot = Some((Instant::now() + lifetime, value.clone()));
        value
    }

    #[cfg(test)]
    pub fn expire(&self) {
        if let Some((until, _)) = crate::sync::lock(&self.slot).as_mut() {
            *until = Instant::now();
        }
    }
}
