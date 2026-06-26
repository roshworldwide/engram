//! The **semantic** store (R2, 2b): mutable, bitemporal, versioned beliefs with
//! lazy confidence decay (R4).
//!
//! A belief is a `(subject, predicate)` whose value, confidence, and decay curve
//! change over time. Every `upsert`/`retract` appends a new **version** rather
//! than overwriting, so all history is retained and queryable (R3).
//!
//! ## Bitemporal model
//!
//! Each version carries valid-time and transaction-time. The store maintains the
//! **transaction-time** chain: an upsert stamps the new version `tx_from = now`
//! (`now` is monotonic) and closes the previously-current version with
//! `tx_until = now`. Versions of a belief are therefore contiguous in tx-time, so
//! "what was believed as-of `T`" is a single `floor` lookup: the version with the
//! greatest `tx_from ≤ T` whose `tx_until` is still open at `T`.
//!
//! ## Lazy decay (P7)
//!
//! Confidence is never stored decayed and no background task touches it. Reads
//! return a [`BeliefView`] whose `confidence` is [`DecayFunction::eval`]uated at
//! the query's evaluation time — a handful of nanoseconds per belief.
//!
//! **Consistency note.** The `versions` and `by_id` indexes are independent MVCC
//! trees published per-tree, and an upsert publishes the new open version then
//! the closed predecessor in two steps. Any *single* query is internally
//! consistent, but a reader correlating across query methods (or reading mid-
//! upsert) may transiently disagree. A consistent cross-index snapshot is
//! deferred to the ACC layer (Phase 3).
//!
//! ```
//! use engram_core::{AgentId, DecayFunction, Timestamp};
//! use engram_storage::{BeliefInput, SemanticStore};
//!
//! let dir = std::env::temp_dir().join("engram-doctest-semantic");
//! let _ = std::fs::remove_file(&dir);
//! let store = SemanticStore::create(&dir).unwrap();
//! store.upsert_belief(BeliefInput {
//!     agent_id: AgentId(1),
//!     subject: "user".into(), predicate: "tone".into(), object: b"formal".to_vec(),
//!     valid_from: Timestamp::from_millis(0),
//!     confidence_init: 0.9, decay_fn: DecayFunction::None, provenance_ids: vec![],
//! }).unwrap();
//! store.commit().unwrap();
//! assert_eq!(store.current("user", "tone").unwrap().record.object, b"formal");
//! # let _ = std::fs::remove_file(&dir);
//! ```

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use engram_core::{
    from_msgpack, AgentId, Clock, DecayFunction, EngramError, MemoryId, Record, RecordKind,
    SemanticRecord, SystemClock, Timestamp,
};

use crate::btree::CowBTree;
use crate::error::{Result, StorageError};
use crate::wal::{Wal, WalOp};

/// The fields a caller supplies to record a belief; the store assigns the id and
/// the transaction-time.
#[derive(Clone, Debug)]
pub struct BeliefInput {
    /// The agent that holds the belief.
    pub agent_id: AgentId,
    /// Subject of the belief triple.
    pub subject: String,
    /// Predicate of the belief triple.
    pub predicate: String,
    /// Object (opaque bytes).
    pub object: Vec<u8>,
    /// When the belief becomes true in the modeled world (valid-time start).
    pub valid_from: Timestamp,
    /// Confidence at `valid_from`, before decay.
    pub confidence_init: f32,
    /// How confidence decays after `valid_from`.
    pub decay_fn: DecayFunction,
    /// Ids of memories justifying this belief.
    pub provenance_ids: Vec<MemoryId>,
}

/// A belief version together with its confidence evaluated at a query time.
#[derive(Clone, Debug)]
pub struct BeliefView {
    /// The underlying versioned record.
    pub record: Arc<SemanticRecord>,
    /// Confidence after decay at the query's evaluation time, clamped to `[0, c0]`.
    pub confidence: f32,
}

/// The serialized write side.
struct Writer {
    wal: Wal,
    /// WAL transaction id for group commit (distinct from bitemporal tx-time).
    wal_tx_id: u64,
    /// Highest transaction-time stamped so far (kept strictly increasing).
    last_tx: i64,
    /// Monotonic counter feeding the randomness field of minted ids.
    next_counter: u128,
    /// Set when a WAL append failed mid-transaction; blocks `commit` so a
    /// half-written batch can never be made durable.
    failed: bool,
}

/// A bitemporal, versioned belief store with lazy decay.
pub struct SemanticStore {
    writer: Mutex<Writer>,
    clock: Arc<dyn Clock>,
    /// `(subject, predicate, tx_from) → version`.
    versions: CowBTree<(String, String, i64), Arc<SemanticRecord>>,
    /// `id → version` (for provenance / by-id lookups).
    by_id: CowBTree<u128, Arc<SemanticRecord>>,
}

impl SemanticStore {
    /// Create a fresh store using the system clock.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        Self::create_with_clock(path, Arc::new(SystemClock))
    }

    /// Create a fresh store with an injected clock (deterministic tests).
    pub fn create_with_clock(path: impl AsRef<Path>, clock: Arc<dyn Clock>) -> Result<Self> {
        Ok(Self::new_store(Wal::create(path)?, clock, 0))
    }

    /// Open an existing store using the system clock.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_clock(path, Arc::new(SystemClock))
    }

    /// Open an existing store with an injected clock, replaying the committed
    /// belief versions into the indexes.
    pub fn open_with_clock(path: impl AsRef<Path>, clock: Arc<dyn Clock>) -> Result<Self> {
        let path = path.as_ref();
        let recovered = Wal::recover(path)?;
        let wal = Wal::open(path)?;
        let store = Self::new_store(wal, clock, recovered.max_tx_id.saturating_add(1));

        let mut max_tx = i64::MIN;
        let mut max_counter = 0u128;
        for entry in &recovered.entries {
            if matches!(entry.op, WalOp::Put(RecordKind::Semantic)) {
                let record: SemanticRecord = from_msgpack(&entry.record).map_err(|e| {
                    EngramError::Decode(format!(
                        "semantic record at lsn {} tx {}: {e}",
                        entry.lsn, entry.tx_id
                    ))
                })?;
                // Cover BOTH endpoints: a retract writes a `tx_until` without a
                // matching new `tx_from`, so ignoring it would let a rewound clock
                // mint a version inside the already-closed interval after reopen.
                max_tx = max_tx.max(record.tx_from.0);
                if let Some(until) = record.tx_until {
                    max_tx = max_tx.max(until.0);
                }
                max_counter = max_counter.max(record.id.randomness());
                store.index(Arc::new(record));
            }
        }
        {
            let mut w = store.writer();
            w.last_tx = max_tx;
            w.next_counter = max_counter.wrapping_add(1);
        }
        Ok(store)
    }

    fn new_store(wal: Wal, clock: Arc<dyn Clock>, wal_tx_id: u64) -> Self {
        SemanticStore {
            writer: Mutex::new(Writer {
                wal,
                wal_tx_id,
                last_tx: i64::MIN,
                next_counter: 0,
                failed: false,
            }),
            clock,
            versions: CowBTree::new(),
            by_id: CowBTree::new(),
        }
    }

    fn writer(&self) -> MutexGuard<'_, Writer> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Strictly-increasing transaction-time in nanoseconds (never collides even
    /// if the wall clock stalls or two upserts land in the same instant).
    fn next_tx(&self, w: &mut Writer) -> i64 {
        let now = self.clock.now().0;
        let t = now.max(w.last_tx.saturating_add(1));
        w.last_tx = t;
        t
    }

    fn mint_id(w: &mut Writer, tx_nanos: i64) -> MemoryId {
        let counter = w.next_counter;
        w.next_counter = w.next_counter.wrapping_add(1);
        let ms = (tx_nanos / 1_000_000).max(0) as u64;
        MemoryId::from_parts(ms, counter)
    }

    /// Publish a version into the `versions` (keyed `(subject, predicate, tx_from)`)
    /// and `by_id` indexes. Infallible: only called after all WAL work succeeds,
    /// so the in-memory chain is updated all-or-nothing per write.
    fn index(&self, record: Arc<SemanticRecord>) {
        let key = (
            record.subject.clone(),
            record.predicate.clone(),
            record.tx_from.0,
        );
        self.versions.insert(key, Arc::clone(&record));
        self.by_id.insert(record.id.0, record);
    }

    /// The belief version live in transaction-time at `tx_at` for `(subject, predicate)`.
    fn version_live_at(
        &self,
        subject: &str,
        predicate: &str,
        tx_at: i64,
    ) -> Option<Arc<SemanticRecord>> {
        let target = (subject.to_owned(), predicate.to_owned(), tx_at);
        match self.versions.floor(&target) {
            Some(((s, p, _tf), rec)) if s == subject && p == predicate => {
                // Contiguous chain: this version is live unless it was closed at
                // or before `tx_at` (superseded or retracted).
                rec.tx_until.is_none_or(|u| tx_at < u.0).then_some(rec)
            }
            _ => None,
        }
    }

    /// Append to the WAL; on failure poison the writer so the in-flight
    /// (uncommitted) transaction can never be committed half-written — recovery
    /// then correctly drops the orphaned frames (no Commit marker).
    fn append_or_poison(w: &mut Writer, wal_tx: u64, bytes: &[u8]) -> Result<()> {
        match w
            .wal
            .append(wal_tx, WalOp::Put(RecordKind::Semantic), bytes)
        {
            Ok(_) => Ok(()),
            Err(e) => {
                w.failed = true;
                Err(e)
            }
        }
    }

    fn aborted() -> StorageError {
        StorageError::Wal("writer aborted by a prior failed write; reopen the store".into())
    }

    /// Record a new belief version. Closes the previously-current version in
    /// transaction-time. Buffered into the WAL; durable at the next [`commit`](Self::commit).
    ///
    /// All fallible work (encoding, WAL appends) happens before any index update,
    /// so a failed upsert leaves the in-memory chain untouched — the live belief
    /// is never silently lost.
    pub fn upsert_belief(&self, input: BeliefInput) -> Result<MemoryId> {
        let mut w = self.writer();
        if w.failed {
            return Err(Self::aborted());
        }
        let now = self.next_tx(&mut w);
        let wal_tx = w.wal_tx_id;

        // Prepare (fallible) the close-of-previous and the new version up front.
        let closed = match self.version_live_at(&input.subject, &input.predicate, now) {
            Some(current) if current.tx_until.is_none() => {
                let mut closed = (*current).clone();
                closed.tx_until = Some(Timestamp(now));
                Some((closed.encode()?, Arc::new(closed)))
            }
            _ => None,
        };
        let id = Self::mint_id(&mut w, now);
        let record = SemanticRecord {
            id,
            agent_id: input.agent_id,
            subject: input.subject,
            predicate: input.predicate,
            object: input.object,
            valid_from: input.valid_from,
            valid_until: None,
            tx_from: Timestamp(now),
            tx_until: None,
            confidence_init: input.confidence_init,
            decay_fn: input.decay_fn,
            provenance_ids: input.provenance_ids,
        };
        let new_bytes = record.encode()?;

        // Append both frames (poisoning on partial failure), then publish to the
        // indexes — new open version FIRST so a concurrent reader never observes
        // the continuously-held belief momentarily absent.
        Self::append_or_poison(&mut w, wal_tx, &new_bytes)?;
        if let Some((bytes, _)) = &closed {
            Self::append_or_poison(&mut w, wal_tx, bytes)?;
        }
        self.index(Arc::new(record));
        if let Some((_, closed_rec)) = closed {
            self.index(closed_rec);
        }
        Ok(id)
    }

    /// Retract the current belief for `(subject, predicate)`: close it in both
    /// time dimensions as of now. Returns whether a belief was open to retract.
    /// History remains queryable via [`get_at_tx`](Self::get_at_tx).
    pub fn retract_belief(&self, subject: &str, predicate: &str) -> Result<bool> {
        let mut w = self.writer();
        if w.failed {
            return Err(Self::aborted());
        }
        let now = self.next_tx(&mut w);
        let wal_tx = w.wal_tx_id;
        if let Some(current) = self.version_live_at(subject, predicate, now) {
            if current.tx_until.is_none() {
                let mut closed = (*current).clone();
                closed.tx_until = Some(Timestamp(now));
                closed.valid_until = closed.valid_until.or(Some(Timestamp(now)));
                let bytes = closed.encode()?;
                Self::append_or_poison(&mut w, wal_tx, &bytes)?;
                self.index(Arc::new(closed));
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The belief as currently known and currently valid, with confidence decayed
    /// to now. `None` if there is no open belief.
    #[must_use]
    pub fn current(&self, subject: &str, predicate: &str) -> Option<BeliefView> {
        let now = self.clock.now();
        self.version_live_at(subject, predicate, now.0)
            .map(|rec| view(rec, now))
    }

    /// Time-travel (R3): what was believed for `(subject, predicate)` as-of the
    /// transaction time `tx_at`, with confidence decayed as of that time.
    #[must_use]
    pub fn get_at_tx(
        &self,
        subject: &str,
        predicate: &str,
        tx_at: Timestamp,
    ) -> Option<BeliefView> {
        self.version_live_at(subject, predicate, tx_at.0)
            .map(|rec| view(rec, tx_at))
    }

    /// Every version of `(subject, predicate)`, ascending by transaction-time.
    #[must_use]
    pub fn history(&self, subject: &str, predicate: &str) -> Vec<Arc<SemanticRecord>> {
        let lo = (subject.to_owned(), predicate.to_owned(), i64::MIN);
        let hi = (subject.to_owned(), predicate.to_owned(), i64::MAX);
        self.versions.range(lo..=hi).map(|(_, v)| v).collect()
    }

    /// Look up a specific belief version by id.
    #[must_use]
    pub fn get_by_id(&self, id: MemoryId) -> Option<Arc<SemanticRecord>> {
        self.by_id.get(&id.0)
    }

    /// Make all upserts/retracts since the last commit durable (one `fsync`).
    /// Refuses (without committing) if a prior write failed mid-transaction, so a
    /// half-written batch is never given a durable Commit marker.
    pub fn commit(&self) -> Result<()> {
        let mut w = self.writer();
        if w.failed {
            return Err(Self::aborted());
        }
        let tx = w.wal_tx_id;
        w.wal.commit(tx)?;
        w.wal_tx_id = w.wal_tx_id.saturating_add(1);
        Ok(())
    }

    /// The total number of belief versions stored.
    #[must_use]
    pub fn version_count(&self) -> usize {
        self.by_id.len()
    }
}

/// Decay a version's confidence as of `eval_at` and wrap it in a [`BeliefView`].
fn view(record: Arc<SemanticRecord>, eval_at: Timestamp) -> BeliefView {
    let elapsed = eval_at.0.saturating_sub(record.valid_from.0);
    let confidence = record.decay_fn.eval(record.confidence_init, elapsed);
    BeliefView { record, confidence }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_core::MockClock;
    use tempfile::tempdir;

    const DAY_NS: i64 = 86_400 * 1_000_000_000;

    fn input(object: &[u8], valid_ms: i64) -> BeliefInput {
        BeliefInput {
            agent_id: AgentId(1),
            subject: "user".into(),
            predicate: "tone".into(),
            object: object.to_vec(),
            valid_from: Timestamp::from_millis(valid_ms),
            confidence_init: 0.9,
            decay_fn: DecayFunction::None,
            provenance_ids: vec![],
        }
    }

    #[test]
    fn upsert_then_current() {
        let dir = tempdir().unwrap();
        let store = SemanticStore::create(dir.path().join("s.wal")).unwrap();
        store.upsert_belief(input(b"formal", 0)).unwrap();
        store.commit().unwrap();
        let view = store.current("user", "tone").unwrap();
        assert_eq!(view.record.object, b"formal");
        assert!((view.confidence - 0.9).abs() < 1e-6);
        assert!(store.current("user", "missing").is_none());
    }

    #[test]
    fn ten_edits_keep_all_versions_and_time_travel() {
        let dir = tempdir().unwrap();
        let clock = Arc::new(MockClock::new(Timestamp(1000 * DAY_NS)));
        let store =
            SemanticStore::create_with_clock(dir.path().join("s.wal"), clock.clone()).unwrap();

        let mut tx_points = Vec::new();
        for v in 0..10u32 {
            let id = store
                .upsert_belief(input(format!("v{v}").as_bytes(), 1000 * DAY_NS / 1_000_000))
                .unwrap();
            // record the tx-time this version became current
            let rec = store.get_by_id(id).unwrap();
            tx_points.push(rec.tx_from);
            clock.advance(DAY_NS); // next edit a day later
        }
        store.commit().unwrap();

        // All 10 versions retained.
        let history = store.history("user", "tone");
        assert_eq!(history.len(), 10);
        assert_eq!(history[0].object, b"v0");
        assert_eq!(history[9].object, b"v9");

        // Time-travel: as-of each version's tx-time returns that version.
        for (v, tx) in tx_points.iter().enumerate() {
            let view = store.get_at_tx("user", "tone", *tx).unwrap();
            assert_eq!(view.record.object, format!("v{v}").into_bytes());
        }
        // Current is the latest.
        assert_eq!(store.current("user", "tone").unwrap().record.object, b"v9");
    }

    #[test]
    fn decay_is_monotonic_on_read() {
        let dir = tempdir().unwrap();
        let clock = Arc::new(MockClock::new(Timestamp(0)));
        let store =
            SemanticStore::create_with_clock(dir.path().join("s.wal"), clock.clone()).unwrap();
        store
            .upsert_belief(BeliefInput {
                decay_fn: DecayFunction::Exponential { lambda: 1e-6 },
                ..input(b"x", 0)
            })
            .unwrap();
        store.commit().unwrap();

        let mut prev = store.current("user", "tone").unwrap().confidence;
        for _ in 0..365 {
            clock.advance(DAY_NS);
            let cur = store.current("user", "tone").unwrap().confidence;
            assert!(cur <= prev, "confidence rose: {cur} > {prev}");
            prev = cur;
        }
        assert!(prev < 0.9); // decayed below the initial value
    }

    #[test]
    fn retract_hides_current_but_keeps_history() {
        let dir = tempdir().unwrap();
        let clock = Arc::new(MockClock::new(Timestamp(1000 * DAY_NS)));
        let store =
            SemanticStore::create_with_clock(dir.path().join("s.wal"), clock.clone()).unwrap();
        let id = store.upsert_belief(input(b"formal", 0)).unwrap();
        let created_tx = store.get_by_id(id).unwrap().tx_from;
        clock.advance(DAY_NS);
        assert!(store.retract_belief("user", "tone").unwrap());
        store.commit().unwrap();

        // No longer current...
        assert!(store.current("user", "tone").is_none());
        // ...but still believed at the time it was held.
        assert_eq!(
            store
                .get_at_tx("user", "tone", created_tx)
                .unwrap()
                .record
                .object,
            b"formal"
        );
        // Retracting again is a no-op.
        assert!(!store.retract_belief("user", "tone").unwrap());
    }

    #[test]
    fn reopen_after_retract_keeps_tx_time_monotonic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.wal");
        let closed_tx;
        {
            let clock = Arc::new(MockClock::new(Timestamp(100 * DAY_NS)));
            let store = SemanticStore::create_with_clock(&path, clock.clone()).unwrap();
            store.upsert_belief(input(b"v0", 0)).unwrap();
            clock.advance(100 * DAY_NS); // retract 100 days later
            assert!(store.retract_belief("user", "tone").unwrap());
            closed_tx = store.history("user", "tone")[0].tx_until.unwrap();
            store.commit().unwrap();
        }
        // Reopen with the clock REWOUND to inside the now-closed interval.
        let rewound = Arc::new(MockClock::new(Timestamp(150 * DAY_NS)));
        let store = SemanticStore::open_with_clock(&path, rewound).unwrap();
        let id = store.upsert_belief(input(b"v1", 0)).unwrap();
        let new_tx = store.get_by_id(id).unwrap().tx_from;
        // Must be stamped strictly after the retract close, not inside it.
        assert!(
            new_tx.0 > closed_tx.0,
            "tx-time regressed into a closed interval: {} <= {}",
            new_tx.0,
            closed_tx.0
        );
    }

    #[test]
    fn recovers_versions_after_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.wal");
        let tx_points;
        {
            let clock = Arc::new(MockClock::new(Timestamp(1000 * DAY_NS)));
            let store = SemanticStore::create_with_clock(&path, clock.clone()).unwrap();
            let mut points = Vec::new();
            for v in 0..5u32 {
                let id = store
                    .upsert_belief(input(format!("v{v}").as_bytes(), 0))
                    .unwrap();
                points.push(store.get_by_id(id).unwrap().tx_from);
                clock.advance(DAY_NS);
            }
            store.commit().unwrap();
            tx_points = points;
        }
        let store = SemanticStore::open(&path).unwrap();
        assert_eq!(store.history("user", "tone").len(), 5);
        assert_eq!(store.current("user", "tone").unwrap().record.object, b"v4");
        assert_eq!(
            store
                .get_at_tx("user", "tone", tx_points[1])
                .unwrap()
                .record
                .object,
            b"v1"
        );
    }
}
