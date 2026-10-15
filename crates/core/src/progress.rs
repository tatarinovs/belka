//! Transfer progress shared between a running transfer and whoever displays it.
//!
//! Providers only update two atomic counters; the reader polls them at its own
//! pace (a GUI timer, a CLI progress bar), so no callbacks run on the hot path.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct Progress {
    inner: Arc<Counters>,
}

#[derive(Debug, Default)]
struct Counters {
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes transferred so far. Goes back when a transfer restarts.
    pub fn done(&self) -> u64 {
        self.inner.done.load(Ordering::Relaxed)
    }

    /// Expected size in bytes; 0 while unknown.
    pub fn total(&self) -> u64 {
        self.inner.total.load(Ordering::Relaxed)
    }

    pub fn set_done(&self, bytes: u64) {
        self.inner.done.store(bytes, Ordering::Relaxed);
    }

    pub fn add(&self, bytes: u64) {
        self.inner.done.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn set_total(&self, bytes: u64) {
        self.inner.total.store(bytes, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_counters() {
        let a = Progress::new();
        let b = a.clone();
        a.set_total(100);
        a.add(30);
        b.add(12);
        assert_eq!((b.done(), b.total()), (42, 100));
        b.set_done(0);
        assert_eq!(a.done(), 0);
    }
}
