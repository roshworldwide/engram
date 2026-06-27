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

    /// An append-only store was given an id that already exists.
    #[error("duplicate memory id: {0}")]
    Duplicate(engram_core::MemoryId),

    /// Adding a causal edge would create a cycle (the provenance DAG must stay acyclic).
    #[error("adding edge {from} -> {to} would create a cycle")]
    Cycle {
        /// The cause endpoint of the rejected edge.
        from: engram_core::MemoryId,
        /// The effect endpoint of the rejected edge.
        to: engram_core::MemoryId,
    },
}
