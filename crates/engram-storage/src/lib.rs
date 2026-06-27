#![deny(unsafe_op_in_unsafe_fn)]
//! # engram-storage
//!
//! The from-scratch storage engine (R1): the write-ahead log (R6, [`wal`]), and
//! — in later phases — the copy-on-write B-tree providing MVCC, the four memory
//! stores (R2), and the causal-provenance DAG store (R5).
//!
//! ## `unsafe` policy
//!
//! This is the **only** crate in the workspace permitted to use `unsafe`, and
//! only for the memory-mapped / zero-copy I/O paths added with the B-tree
//! (Phase 1c). Every `unsafe` block must carry a `// SAFETY:` proof comment and
//! a focused test. The WAL (Phase 1b) uses no `unsafe`.
//!
//! ```
//! use engram_storage::{Wal, WalOp};
//! use engram_core::RecordKind;
//!
//! let dir = std::env::temp_dir().join("engram-doctest-wal");
//! let _ = std::fs::remove_file(&dir);
//! let mut wal = Wal::create(&dir).unwrap();
//! wal.append(1, WalOp::Put(RecordKind::Episodic), b"hello").unwrap();
//! wal.commit(1).unwrap();
//! let recovered = Wal::recover(&dir).unwrap();
//! assert_eq!(recovered.entries.len(), 1);
//! assert_eq!(recovered.entries[0].record, b"hello");
//! # let _ = std::fs::remove_file(&dir);
//! ```

pub mod btree;
pub mod error;
pub mod stores;
pub mod wal;

pub use btree::{CowBTree, Iter, Snapshot};
pub use error::{Result, StorageError};
pub use stores::{
    BeliefInput, BeliefView, CausalDag, EpisodicStore, ProceduralStore, SemanticStore,
    WorkingMemory,
};
pub use wal::{scan_bytes, Recovered, Wal, WalEntry, WalOp};

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
        assert!(!engram_core::version().is_empty());
    }
}
