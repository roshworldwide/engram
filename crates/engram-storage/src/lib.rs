#![deny(unsafe_op_in_unsafe_fn)]
//! # engram-storage
//!
//! The from-scratch storage engine (R1): on-disk record format, write-ahead log
//! (R6), copy-on-write B-tree providing MVCC (R6), the four memory stores (R2),
//! and the causal-provenance DAG store (R5).
//!
//! ## `unsafe` policy
//!
//! This is the **only** crate in the workspace permitted to use `unsafe`, and
//! only for the memory-mapped / zero-copy I/O paths added in Phase 1+. Every
//! `unsafe` block must carry a `// SAFETY:` proof comment and a focused test.
//! There is no `unsafe` code yet — this remains a Phase 0 scaffold until the WAL
//! and B-tree land in Phase 1b/1c.

/// The semantic version of the Engram storage crate.
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

    #[test]
    fn links_against_core() {
        // Proves the inter-crate dependency edge is wired before Phase 1.
        assert!(!engram_core::version().is_empty());
    }
}
