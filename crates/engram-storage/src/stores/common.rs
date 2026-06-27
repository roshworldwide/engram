//! Shared serialized-write machinery for the WAL-backed stores (episodic,
//! semantic, procedural, causal).
//!
//! Every such store guards its writes with a `Mutex` over a small writer that owns
//! the WAL handle, the group-commit transaction id, and a poison flag; three of
//! the four also mint monotonic transaction-time and ids. This module hoists that
//! duplicated machinery so each store **composes** it (holds a [`WalWriter`], and
//! optionally a [`TxClock`]) instead of copying it.

use std::path::Path;

use engram_core::{Clock, MemoryId, RecordKind};

use crate::error::{Result, StorageError};
use crate::wal::{Recovered, Wal, WalOp};

/// The base WAL writer every durable store embeds. Causal uses the `None` arm for
/// its in-memory mode, so the same type covers both. Serialized behind the store's
/// `Mutex`.
pub(crate) struct WalWriter {
    /// The write-ahead log, or `None` for an in-memory store.
    pub(crate) wal: Option<Wal>,
    /// Group-commit transaction id: many appends share one and are made durable by
    /// one [`commit`](Self::commit) (one fsync).
    pub(crate) wal_tx_id: u64,
    /// Set when a WAL append fails; subsequent writes/commits are refused (see
    /// [`guard`](Self::guard)) so a half-written batch can never be committed.
    pub(crate) failed: bool,
}

impl WalWriter {
    /// A writer over `wal` (or `None` for in-memory), starting at `wal_tx_id`.
    pub(crate) fn new(wal: Option<Wal>, wal_tx_id: u64) -> Self {
        WalWriter {
            wal,
            wal_tx_id,
            failed: false,
        }
    }

    /// Buffer a record into the WAL under the current transaction id. On failure
    /// the writer is poisoned. A no-op for an in-memory writer.
    pub(crate) fn append(&mut self, kind: RecordKind, bytes: &[u8]) -> Result<()> {
        // Read the tx id into a local before borrowing `self.wal` mutably, so the
        // borrow checker does not see a whole-`self` borrow through the field.
        let tx = self.wal_tx_id;
        let res = match self.wal.as_mut() {
            Some(w) => w.append(tx, WalOp::Put(kind), bytes),
            None => Ok(0),
        };
        if let Err(e) = res {
            self.failed = true;
            return Err(e);
        }
        Ok(())
    }

    /// Make every append since the last commit durable (one fsync) and advance to a
    /// fresh transaction id. A no-op for an in-memory writer.
    pub(crate) fn commit(&mut self) -> Result<()> {
        if let Some(w) = self.wal.as_mut() {
            w.commit(self.wal_tx_id)?;
            self.wal_tx_id = self.wal_tx_id.saturating_add(1);
        }
        Ok(())
    }

    /// Refuse to proceed if a prior write poisoned the writer. `noun` names the
    /// thing to reopen (`"store"` / `"DAG"`) in the error message.
    pub(crate) fn guard(&self, noun: &str) -> Result<()> {
        if self.failed {
            return Err(aborted(noun));
        }
        Ok(())
    }
}

/// The bitemporal mixin embedded — as a sibling field of [`WalWriter`] — by the
/// stores that mint monotonic transaction-time and ids (semantic, procedural).
pub(crate) struct TxClock {
    /// Highest transaction-time stamped so far (kept strictly increasing).
    pub(crate) last_tx: i64,
    /// Next id-randomness counter (makes minted ids unique within a tx-millisecond).
    pub(crate) next_counter: u128,
}

impl TxClock {
    /// A fresh clock (`last_tx = i64::MIN`, `next_counter = 0`).
    pub(crate) fn new() -> Self {
        TxClock {
            last_tx: i64::MIN,
            next_counter: 0,
        }
    }
}

/// The poisoned-writer error: a prior write failed, so the store must be reopened.
pub(crate) fn aborted(noun: &str) -> StorageError {
    StorageError::Wal(format!(
        "writer aborted by a prior failed write; reopen the {noun}"
    ))
}

/// The open prelude shared by every durable store: recover the committed redo set,
/// then reopen the WAL for appending (its torn tail truncated).
pub(crate) fn open_prelude(path: &Path) -> Result<(Recovered, Wal)> {
    let recovered = Wal::recover(path)?;
    let wal = Wal::open(path)?;
    Ok((recovered, wal))
}

/// Strictly-increasing transaction-time in nanoseconds: never collides, even if the
/// wall clock stalls or two writes land in the same instant.
pub(crate) fn next_tx(clock: &dyn Clock, tc: &mut TxClock) -> i64 {
    let now = clock.now().0;
    let t = now.max(tc.last_tx.saturating_add(1));
    tc.last_tx = t;
    t
}

/// Mint a time-sortable id from a transaction timestamp (ns) plus the writer's
/// counter.
pub(crate) fn mint_id(tc: &mut TxClock, tx_nanos: i64) -> MemoryId {
    let counter = tc.next_counter;
    tc.next_counter = tc.next_counter.wrapping_add(1);
    let ms = (tx_nanos / 1_000_000).max(0) as u64;
    MemoryId::from_parts(ms, counter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_core::{MockClock, Timestamp};

    #[test]
    fn guard_refuses_after_poison() {
        let mut w = WalWriter::new(None, 5);
        assert!(w.guard("store").is_ok());
        w.failed = true;
        let err = w.guard("store").unwrap_err();
        assert!(err.to_string().contains("reopen the store"));
        // The noun is threaded through.
        assert!(aborted("DAG").to_string().contains("reopen the DAG"));
    }

    #[test]
    fn in_memory_writer_append_and_commit_are_noops() {
        let mut w = WalWriter::new(None, 0);
        assert!(w.append(RecordKind::CausalEdge, b"x").is_ok());
        assert!(w.commit().is_ok());
        assert_eq!(
            w.wal_tx_id, 0,
            "in-memory commit must not advance the tx id"
        );
        assert!(!w.failed);
    }

    #[test]
    fn commit_advances_wal_tx_id_with_a_wal() {
        let dir = tempfile::tempdir().unwrap();
        let wal = Wal::create(dir.path().join("w.wal")).unwrap();
        let mut w = WalWriter::new(Some(wal), 0);
        w.append(RecordKind::Episodic, b"hello").unwrap();
        w.commit().unwrap();
        assert_eq!(w.wal_tx_id, 1);
    }

    #[test]
    fn next_tx_is_strictly_monotonic_and_ids_are_unique() {
        let clock = MockClock::new(Timestamp(1_000));
        let mut tc = TxClock::new();
        let a = next_tx(&clock, &mut tc);
        let b = next_tx(&clock, &mut tc); // clock stalled — must still advance
        assert!(b > a);
        let id1 = mint_id(&mut tc, a);
        let id2 = mint_id(&mut tc, a); // same tx-ms, different counter
        assert_ne!(id1, id2);
    }
}
