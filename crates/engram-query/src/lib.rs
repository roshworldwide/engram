#![forbid(unsafe_code)]
//! # engram-query
//!
//! The query engine: memory-type dispatcher, bitemporal time-travel resolver
//! (R3), lazy confidence-decay evaluator (R4), causal-DAG traversal and
//! provenance tracer (R5), and the working-memory cap. Sits above the storage
//! and consistency layers; requests flow down, results flow up.
//!
//! Phase 3c: the [`Engine`] bundles the four stores + the causal DAG over one
//! data directory; the REST/gRPC server and the Python SDK build on it.

pub mod engine;

pub use engine::Engine;

/// The semantic version of the Engram query crate.
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
