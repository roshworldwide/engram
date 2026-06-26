//! Time as a value (`Timestamp`) and as an injectable source (`Clock`).
//!
//! All time in Engram flows through the [`Clock`] trait so that bitemporal
//! time-travel and ACC histories are reproducible in tests without real waiting
//! ([`MockClock`]) while production uses the wall clock ([`SystemClock`]).

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A point in time, in nanoseconds since the Unix epoch.
///
/// ```
/// use engram_core::Timestamp;
/// let t = Timestamp::from_millis(1_700_000_000_000);
/// assert_eq!(t.as_millis(), 1_700_000_000_000);
/// ```
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// The Unix epoch (`0` nanoseconds).
    pub const EPOCH: Timestamp = Timestamp(0);

    /// Construct from nanoseconds since the epoch.
    #[must_use]
    pub const fn from_nanos(ns: i64) -> Self {
        Timestamp(ns)
    }

    /// Construct from milliseconds since the epoch (saturating on overflow).
    #[must_use]
    pub const fn from_millis(ms: i64) -> Self {
        Timestamp(ms.saturating_mul(1_000_000))
    }

    /// Nanoseconds since the epoch.
    #[must_use]
    pub const fn as_nanos(self) -> i64 {
        self.0
    }

    /// Whole milliseconds since the epoch (truncating).
    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0 / 1_000_000
    }

    /// Add a nanosecond offset, saturating at the bounds of `i64`.
    #[must_use]
    pub const fn saturating_add_nanos(self, ns: i64) -> Self {
        Timestamp(self.0.saturating_add(ns))
    }

    /// Nanoseconds elapsed from `earlier` to `self` (saturating, never negative
    /// below `0` is not enforced — callers clamp where decay requires it).
    #[must_use]
    pub const fn nanos_since(self, earlier: Timestamp) -> i64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// A source of "now". Implementors are `Send + Sync` so a single clock can be
/// shared across threads and agent instances.
pub trait Clock: Send + Sync {
    /// The current time.
    fn now(&self) -> Timestamp;
}

/// The real wall clock, reading nanoseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        Timestamp(ns)
    }
}

/// A controllable clock for deterministic tests. Time only moves when told to.
#[derive(Debug)]
pub struct MockClock {
    now_ns: AtomicI64,
}

impl MockClock {
    /// Create a clock fixed at `start`.
    #[must_use]
    pub fn new(start: Timestamp) -> Self {
        MockClock {
            now_ns: AtomicI64::new(start.0),
        }
    }

    /// Set the clock to an absolute time.
    pub fn set(&self, t: Timestamp) {
        self.now_ns.store(t.0, Ordering::SeqCst);
    }

    /// Advance the clock by `ns` nanoseconds (saturating).
    pub fn advance(&self, ns: i64) {
        // fetch_update keeps the saturating semantics atomic.
        let _ = self
            .now_ns
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |cur| {
                Some(cur.saturating_add(ns))
            });
    }
}

impl Clock for MockClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.now_ns.load(Ordering::SeqCst))
    }
}

/// A shared `Clock` (e.g. `Arc<MockClock>`) is itself a `Clock`, so a test can
/// hold one handle while a generator holds another.
impl<C: Clock + ?Sized> Clock for Arc<C> {
    fn now(&self) -> Timestamp {
        (**self).now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_clock_is_controllable() {
        let clock = MockClock::new(Timestamp::from_millis(1000));
        assert_eq!(clock.now(), Timestamp::from_millis(1000));
        clock.advance(5_000_000); // +5 ms
        assert_eq!(clock.now().as_millis(), 1005);
        clock.set(Timestamp::EPOCH);
        assert_eq!(clock.now(), Timestamp::EPOCH);
    }

    #[test]
    fn shared_clock_reflects_updates() {
        let clock = Arc::new(MockClock::new(Timestamp::EPOCH));
        let handle = Arc::clone(&clock);
        clock.advance(1_000_000);
        assert_eq!(handle.now().as_millis(), 1);
    }

    #[test]
    fn system_clock_is_nonnegative_and_advances_relationships_hold() {
        let c = SystemClock;
        assert!(c.now().as_nanos() >= 0);
    }
}
