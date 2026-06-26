//! The **episodic** store (R2, 2a): immutable, append-only events.
//!
//! Durability comes from the [`Wal`](crate::Wal); query speed from four
//! copy-on-write B-tree indexes over the same shared `Arc<EpisodicRecord>`:
//!
//! - **primary** `MemoryId → record` — point lookups,
//! - **by-session** `(SessionId, MemoryId) → record` — `scan_session`,
//! - **by-time** `(valid_time, MemoryId) → record` — `scan_time_range`,
//! - **by-cause** `(cause_id, effect_id) → record` — `effects_of` (the seed of
//!   the causal-provenance DAG, R5).
//!
//! Writes are buffered into the WAL and made durable with [`commit`](EpisodicStore::commit)
//! (group commit — many [`append`](EpisodicStore::append)s, one `fsync`). Each
//! append also updates the in-memory indexes immediately. On
//! [`open`](EpisodicStore::open) the committed redo set is replayed into the
//! indexes; uncommitted tail events are dropped, exactly as the WAL recovers.
//!
//! **Consistency note.** Each index is an independent MVCC tree, so any *single*
//! query is internally consistent. The four indexes are updated under the writer
//! lock but published per-tree, so a reader that *correlates results across two
//! query methods* while a writer is mid-append may transiently see them disagree.
//! A consistent cross-index snapshot is deferred to the ACC layer (Phase 3).
//!
//! ```
//! use engram_core::{AgentId, EpisodicRecord, EventType, MemoryId, SessionId, Timestamp};
//! use engram_storage::EpisodicStore;
//!
//! let dir = std::env::temp_dir().join("engram-doctest-episodic");
//! let _ = std::fs::remove_file(&dir);
//! let store = EpisodicStore::create(&dir).unwrap();
//! let id = MemoryId::from_parts(1_700_000_000_000, 1);
//! store.append_committed(EpisodicRecord {
//!     id, agent_id: AgentId(1), session_id: SessionId(9),
//!     valid_time: Timestamp::from_millis(1_700_000_000_000), tx_time: Timestamp(0),
//!     event_type: EventType::Observation, payload: b"hi".to_vec(), cause_ids: vec![],
//! }).unwrap();
//! assert_eq!(store.get(id).unwrap().payload, b"hi");
//! assert_eq!(store.scan_session(SessionId(9)).len(), 1);
//! # let _ = std::fs::remove_file(&dir);
//! ```

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use engram_core::{
    from_msgpack, EngramError, EpisodicRecord, MemoryId, Record, RecordKind, SessionId, Timestamp,
};

use crate::btree::CowBTree;
use crate::error::{Result, StorageError};
use crate::wal::{Wal, WalOp};

/// The serialized write side (single-writer; the WAL is driven one append at a
/// time). Reads never touch this.
struct Writer {
    wal: Wal,
    tx_id: u64,
}

/// An append-only store of episodic events with primary/session/time/causal
/// indexes. Cheap to share across threads (`Arc<EpisodicStore>`); reads are
/// lock-free, writes are serialized.
pub struct EpisodicStore {
    writer: Mutex<Writer>,
    primary: CowBTree<u128, Arc<EpisodicRecord>>,
    by_session: CowBTree<(u64, u128), Arc<EpisodicRecord>>,
    by_time: CowBTree<(i64, u128), Arc<EpisodicRecord>>,
    by_cause: CowBTree<(u128, u128), Arc<EpisodicRecord>>,
}

impl EpisodicStore {
    /// Create a fresh store backed by a new WAL at `path`.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::with_writer(Wal::create(path)?, 0))
    }

    /// Open an existing store: recover the committed events from the WAL, replay
    /// them into the indexes, and resume appending (torn tail truncated).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let recovered = Wal::recover(path)?;
        let wal = Wal::open(path)?;
        // Resume tx ids strictly above every tx id still on disk so a committed
        // tx id is never reused (keeps position-aware redo sound).
        let store = Self::with_writer(wal, recovered.max_tx_id.saturating_add(1));
        for entry in &recovered.entries {
            if matches!(entry.op, WalOp::Put(RecordKind::Episodic)) {
                let record: EpisodicRecord = from_msgpack(&entry.record).map_err(|e| {
                    EngramError::Decode(format!(
                        "episodic record at lsn {} tx {}: {e}",
                        entry.lsn, entry.tx_id
                    ))
                })?;
                store.index(Arc::new(record));
            }
        }
        Ok(store)
    }

    fn with_writer(wal: Wal, tx_id: u64) -> Self {
        EpisodicStore {
            writer: Mutex::new(Writer { wal, tx_id }),
            primary: CowBTree::new(),
            by_session: CowBTree::new(),
            by_time: CowBTree::new(),
            by_cause: CowBTree::new(),
        }
    }

    fn writer(&self) -> MutexGuard<'_, Writer> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Insert a record into every index (one `Arc` shared across them).
    ///
    /// The four trees are published per-tree (each `insert` is its own atomic
    /// root swap), not as a single atomic multi-index step. Called under the
    /// writer lock with allocation-only work (no panicking ops), so the only
    /// observable effect on a concurrent reader is a brief cross-index skew
    /// within one append; a truly atomic multi-index snapshot is deferred to the
    /// MVCC/ACC layer (Phase 3).
    fn index(&self, record: Arc<EpisodicRecord>) {
        let id = record.id.0;
        self.primary.insert(id, Arc::clone(&record));
        self.by_session
            .insert((record.session_id.0, id), Arc::clone(&record));
        self.by_time
            .insert((record.valid_time.0, id), Arc::clone(&record));
        for cause in &record.cause_ids {
            self.by_cause.insert((cause.0, id), Arc::clone(&record));
        }
    }

    /// Append an event, returning its id. Buffered into the WAL and indexed
    /// immediately (so it is visible to reads in this process before it is
    /// durable); made durable at the next [`commit`](Self::commit).
    ///
    /// The store is append-only and ids must be unique: re-appending an existing
    /// id returns [`StorageError::Duplicate`] without touching the WAL or indexes.
    pub fn append(&self, record: EpisodicRecord) -> Result<MemoryId> {
        let id = record.id;
        // Hold the writer lock across the duplicate check, the WAL append, and the
        // index updates so they are atomic with respect to other writers (the
        // check + insert cannot interleave with another append of the same id).
        let mut w = self.writer();
        if self.primary.get(&id.0).is_some() {
            return Err(StorageError::Duplicate(id));
        }
        let bytes = record.encode()?;
        let record = Arc::new(record);
        let tx = w.tx_id;
        w.wal.append(tx, WalOp::Put(RecordKind::Episodic), &bytes)?;
        self.index(record);
        Ok(id)
    }

    /// Make all appends since the last commit durable (one `fsync`), then start a
    /// fresh transaction for subsequent appends.
    pub fn commit(&self) -> Result<()> {
        let mut w = self.writer();
        let tx = w.tx_id;
        w.wal.commit(tx)?;
        w.tx_id = w.tx_id.saturating_add(1);
        Ok(())
    }

    /// Append a single event and make it durable immediately.
    pub fn append_committed(&self, record: EpisodicRecord) -> Result<MemoryId> {
        let id = self.append(record)?;
        self.commit()?;
        Ok(id)
    }

    /// Look up an event by id.
    #[must_use]
    pub fn get(&self, id: MemoryId) -> Option<Arc<EpisodicRecord>> {
        self.primary.get(&id.0)
    }

    /// The number of events currently indexed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.primary.len()
    }

    /// Whether the store holds no events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.primary.is_empty()
    }

    /// All events in `session`, in ascending id (≈ creation-time) order.
    #[must_use]
    pub fn scan_session(&self, session: SessionId) -> Vec<Arc<EpisodicRecord>> {
        self.by_session
            .range((session.0, u128::MIN)..=(session.0, u128::MAX))
            .map(|(_, v)| v)
            .collect()
    }

    /// All events whose `valid_time` is in the half-open interval `[lo, hi)`,
    /// ascending by `(valid_time, id)`.
    #[must_use]
    pub fn scan_time_range(&self, lo: Timestamp, hi: Timestamp) -> Vec<Arc<EpisodicRecord>> {
        self.by_time
            .range((lo.0, u128::MIN)..(hi.0, u128::MIN))
            .map(|(_, v)| v)
            .collect()
    }

    /// Every event that lists `cause` among its `cause_ids` (the direct effects
    /// of `cause`), ascending by effect id.
    #[must_use]
    pub fn effects_of(&self, cause: MemoryId) -> Vec<Arc<EpisodicRecord>> {
        self.by_cause
            .range((cause.0, u128::MIN)..=(cause.0, u128::MAX))
            .map(|(_, v)| v)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_core::{AgentId, EventType};
    use tempfile::tempdir;

    fn event(id: u128, session: u64, valid_ms: i64, causes: Vec<MemoryId>) -> EpisodicRecord {
        EpisodicRecord {
            id: MemoryId(id),
            agent_id: AgentId(1),
            session_id: SessionId(session),
            valid_time: Timestamp::from_millis(valid_ms),
            tx_time: Timestamp(0),
            event_type: EventType::Observation,
            payload: format!("p{id}").into_bytes(),
            cause_ids: causes,
        }
    }

    #[test]
    fn append_get_and_counts() {
        let dir = tempdir().unwrap();
        let store = EpisodicStore::create(dir.path().join("e.wal")).unwrap();
        let id = store.append_committed(event(10, 1, 1000, vec![])).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(id).unwrap().payload, b"p10");
        assert!(store.get(MemoryId(999)).is_none());
    }

    #[test]
    fn scan_session_isolates_sessions() {
        let dir = tempdir().unwrap();
        let store = EpisodicStore::create(dir.path().join("e.wal")).unwrap();
        for i in 0..30u128 {
            store
                .append(event(i, (i % 3) as u64, 1000 + i as i64, vec![]))
                .unwrap();
        }
        store.commit().unwrap();
        let s1 = store.scan_session(SessionId(1));
        assert_eq!(s1.len(), 10);
        assert!(s1.iter().all(|r| r.session_id == SessionId(1)));
        assert!(s1.windows(2).all(|w| w[0].id < w[1].id)); // ascending by id
    }

    #[test]
    fn scan_time_range_is_half_open_and_exact() {
        let dir = tempdir().unwrap();
        let store = EpisodicStore::create(dir.path().join("e.wal")).unwrap();
        for i in 0..100u128 {
            store
                .append(event(i, 0, 1000 + 10 * i as i64, vec![]))
                .unwrap();
        }
        store.commit().unwrap();
        // [1100, 1200) -> i in [10, 20)
        let slice =
            store.scan_time_range(Timestamp::from_millis(1100), Timestamp::from_millis(1200));
        let ids: Vec<u128> = slice.iter().map(|r| r.id.0).collect();
        assert_eq!(ids, (10u128..20).collect::<Vec<_>>());
    }

    #[test]
    fn effects_of_follows_causes() {
        let dir = tempdir().unwrap();
        let store = EpisodicStore::create(dir.path().join("e.wal")).unwrap();
        let cause = MemoryId(1);
        store.append(event(1, 0, 1000, vec![])).unwrap();
        store.append(event(2, 0, 1001, vec![cause])).unwrap();
        store.append(event(3, 0, 1002, vec![cause])).unwrap();
        store.append(event(4, 0, 1003, vec![MemoryId(99)])).unwrap();
        store.commit().unwrap();
        let effects: Vec<u128> = store.effects_of(cause).iter().map(|r| r.id.0).collect();
        assert_eq!(effects, vec![2, 3]);
    }

    #[test]
    fn duplicate_id_is_rejected_with_no_phantom_index_entries() {
        let dir = tempdir().unwrap();
        let store = EpisodicStore::create(dir.path().join("e.wal")).unwrap();
        store.append(event(5, 0, 1000, vec![])).unwrap();
        // Re-appending id 5 with different session/time/cause is rejected outright.
        let err = store
            .append(event(5, 9, 2000, vec![MemoryId(1)]))
            .unwrap_err();
        assert!(matches!(err, StorageError::Duplicate(MemoryId(5))));
        store.commit().unwrap();
        assert_eq!(store.len(), 1);
        // No phantom entries leaked into the secondary indexes.
        assert!(store.scan_session(SessionId(9)).is_empty());
        assert!(store.effects_of(MemoryId(1)).is_empty());
        assert!(store
            .scan_time_range(Timestamp::from_millis(2000), Timestamp::from_millis(2001))
            .is_empty());
    }

    #[test]
    fn recovers_committed_events_after_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("e.wal");
        {
            let store = EpisodicStore::create(&path).unwrap();
            for i in 0..50u128 {
                store
                    .append(event(i, (i % 4) as u64, 1000 + i as i64, vec![]))
                    .unwrap();
            }
            store.commit().unwrap();
            // An uncommitted tail that must NOT survive a reopen.
            store.append(event(999, 0, 9999, vec![])).unwrap();
        }
        let store = EpisodicStore::open(&path).unwrap();
        assert_eq!(store.len(), 50);
        assert!(store.get(MemoryId(999)).is_none());
        assert_eq!(store.scan_session(SessionId(2)).len(), 12); // i in 0..50, i%4==2
                                                                // Appending after reopen continues to work and is durable.
        store
            .append_committed(event(1000, 0, 2000, vec![]))
            .unwrap();
        assert_eq!(store.len(), 51);
    }
}
