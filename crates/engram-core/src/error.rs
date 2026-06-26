//! Workspace error types.
//!
//! Library code never panics or `unwrap()`s on these paths — fallible operations
//! return [`Result`]. Binaries may add `anyhow` context on top.

use thiserror::Error;

/// Convenience alias for results carrying an [`EngramError`].
pub type Result<T> = std::result::Result<T, EngramError>;

/// The top-level error type for the Engram engine. Variants grow per phase; the
/// codec and identifier variants land in Phase 1a.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EngramError {
    /// A value could not be serialized to MessagePack.
    #[error("serialization failed: {0}")]
    Encode(String),

    /// Bytes could not be deserialized into the requested type.
    #[error("deserialization failed: {0}")]
    Decode(String),

    /// A [`crate::MemoryId`] string failed to parse.
    #[error("invalid identifier: {0}")]
    Id(#[from] IdParseError),
}

/// Why a Crockford-base32 [`crate::MemoryId`] string failed to parse.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdParseError {
    /// A `MemoryId` is exactly 26 Crockford-base32 characters.
    #[error("expected 26 characters, found {0}")]
    WrongLength(usize),

    /// A character outside the Crockford base32 alphabet was encountered.
    #[error("invalid base32 character: {0:?}")]
    InvalidChar(char),

    /// The leading character encodes more than 128 bits of value.
    #[error("identifier overflows 128 bits")]
    Overflow,
}
