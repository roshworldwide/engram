#![forbid(unsafe_code)]
//! # engram-core
//!
//! Shared, dependency-light types used across every Engram crate: identifiers,
//! timestamps, the memory/edge/decay enums, and the workspace error type.
//!
//! **Phase 0 scaffold.** The concrete data model from the build spec (§5) lands
//! in Phase 1a, test-first. For now this crate exposes only build metadata so it
//! has a stable, documented public surface and a green test to anchor CI.

/// The semantic version of the Engram core crate, captured from Cargo at build time.
///
/// ```
/// assert!(!engram_core::version().is_empty());
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_is_reported() {
        assert!(!version().is_empty(), "crate version must be non-empty");
    }
}
