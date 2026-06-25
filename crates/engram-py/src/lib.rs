#![forbid(unsafe_code)]
//! # engram-py
//!
//! PyO3 bindings exposing Engram to Python as the `engram` package: the
//! `EngramClient`, `EpisodicMemory`, `SemanticMemory`, and `MemoryQuery` types.
//! Built into a wheel with maturin. Lands in Phase 3d.
//!
//! **Phase 0 scaffold.**

/// The semantic version of the Engram Python-bindings crate.
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
