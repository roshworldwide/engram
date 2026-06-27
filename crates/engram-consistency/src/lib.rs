#![forbid(unsafe_code)]
//! # engram-consistency
//!
//! Agent Causal Consistency (ACC, R7): the vector-clock engine, session
//! coordinator, and the read-your-writes / monotonic-reads / causal-memory
//! validators. ACC is achievable with `O(|agents|)` metadata per operation and
//! no global synchronization — the formal contract lives in the build spec (§9)
//! and is encoded here as runtime checks and proptest oracles.
//!
//! Phase 3a ships the [`VectorClock`] engine; the session coordinator and the
//! ACC validators land in Phase 3b.

pub mod vector_clock;

pub use vector_clock::{VectorClock, BYTES_PER_SLOT};

/// The semantic version of the Engram consistency crate.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_reported() {
        assert!(!super::version().is_empty());
    }
}
