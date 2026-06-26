//! Storage-layer errors.

use thiserror::Error;

/// Convenience alias for storage results.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Errors raised by the storage engine (WAL, B-tree, stores).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StorageError {
    /// An underlying I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A codec / core error (e.g. MessagePack decode of WAL metadata).
    #[error(transparent)]
    Core(#[from] engram_core::EngramError),

    /// A WAL invariant was violated.
    #[error("wal: {0}")]
    Wal(String),
}
