#![forbid(unsafe_code)]
//! # engram-server
//!
//! Network front-ends over the query engine: a REST API (axum) and a gRPC API
//! (tonic + prost), exposing episodic/semantic writes, time-travel reads, and
//! provenance traces. Lands in Phase 3c.
//!
//! **Phase 0 scaffold** — the async runtime and service deps are wired in when
//! the endpoints are implemented, to keep the Phase 0 build dependency-free.

/// The semantic version of the Engram server crate.
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
