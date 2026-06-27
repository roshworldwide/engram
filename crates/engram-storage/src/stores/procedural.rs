//! The **procedural** store (R2, 2c): versioned skills.
//!
//! A skill is identified by `(agent_id, name)` and evolves through versions. Each
//! `put_skill` appends a new version (`version = prev + 1`) whose `supersedes`
//! points at the previous version's id, forming a chain. All versions are
//! retained: fetch the latest or any specific version.
//!
//! Versions are keyed `(agent_id, name, version)` in a CoW B-tree, so "the latest
//! skill" is a single [`floor`](crate::CowBTree::floor) at `version = u32::MAX`
//! and "version N" is a direct lookup.
//!
//! ```
//! use engram_core::AgentId;
//! use engram_storage::ProceduralStore;
//!
//! let dir = std::env::temp_dir().join("engram-doctest-procedural");
//! let _ = std::fs::remove_file(&dir);
//! let store = ProceduralStore::create(&dir).unwrap();
//! let v1 = store.put_skill(AgentId(1), "deploy", b"step-a".to_vec(), Default::default()).unwrap();
//! let v2 = store.put_skill(AgentId(1), "deploy", b"step-a; step-b".to_vec(), Default::default()).unwrap();
//! store.commit().unwrap();
//! let latest = store.latest(AgentId(1), "deploy").unwrap();
//! assert_eq!(latest.version, 2);
//! assert_eq!(latest.supersedes, Some(v1));
//! assert_eq!(latest.id, v2);
//! # let _ = std::fs::remove_file(&dir);
//! ```

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use engram_core::{
    from_msgpack, AgentId, Clock, EngramError, MemoryId, ProceduralRecord, Record, RecordKind,
    SystemClock, Timestamp,
};

use crate::btree::CowBTree;
use crate::error::{Result, StorageError};
use crate::wal::{Wal, WalOp};

struct Writer {
    wal: Wal,
    wal_tx_id: u64,
    last_tx: i64,
    next_counter: u128,
    failed: bool,
}

/// A versioned skill store. Reads are lock-free; writes are serialized.
pub struct ProceduralStore {
    writer: Mutex<Writer>,
    clock: Arc<dyn Clock>,
    /// `(agent_id, name, version) → skill version`.
    by_skill: CowBTree<(u64, String, u32), Arc<ProceduralRecord>>,
    /// `id → skill version`.
    by_id: CowBTree<u128, Arc<ProceduralRecord>>,
}

impl ProceduralStore {
    /// Create a fresh store using the system clock.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        Self::create_with_clock(path, Arc::new(SystemClock))
    }

    /// Create a fresh store with an injected clock.
    pub fn create_with_clock(path: impl AsRef<Path>, clock: Arc<dyn Clock>) -> Result<Self> {
        Ok(Self::new_store(Wal::create(path)?, clock, 0))
    }

    /// Open an existing store using the system clock.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_clock(path, Arc::new(SystemClock))
    }

    /// Open an existing store, replaying committed skill versions.
    pub fn open_with_clock(path: impl AsRef<Path>, clock: Arc<dyn Clock>) -> Result<Self> {
        let path = path.as_ref();
        let recovered = Wal::recover(path)?;
        let wal = Wal::open(path)?;
        let store = Self::new_store(wal, clock, recovered.max_tx_id.saturating_add(1));

        let mut max_tx = i64::MIN;
        let mut max_counter = 0u128;
        for entry in &recovered.entries {
            if matches!(entry.op, WalOp::Put(RecordKind::Procedural)) {
                let record: ProceduralRecord = from_msgpack(&entry.record).map_err(|e| {
                    EngramError::Decode(format!(
                        "procedural record at lsn {} tx {}: {e}",
                        entry.lsn, entry.tx_id
                    ))
                })?;
                max_tx = max_tx.max(record.tx_from.0);
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
        ProceduralStore {
            writer: Mutex::new(Writer {
                wal,
                wal_tx_id,
                last_tx: i64::MIN,
                next_counter: 0,
                failed: false,
            }),
            clock,
            by_skill: CowBTree::new(),
            by_id: CowBTree::new(),
        }
    }

    fn writer(&self) -> MutexGuard<'_, Writer> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_tx(&self, w: &mut Writer) -> i64 {
        let now = self.clock.now().0;
        let t = now.max(w.last_tx.saturating_add(1));
        w.last_tx = t;
        t
    }

    fn mint_id(w: &mut Writer, tx_nanos: i64) -> MemoryId {
        let counter = w.next_counter;
        w.next_counter = w.next_counter.wrapping_add(1);
        MemoryId::from_parts((tx_nanos / 1_000_000).max(0) as u64, counter)
    }

    fn index(&self, record: Arc<ProceduralRecord>) {
        let key = (record.agent_id.0, record.name.clone(), record.version);
        self.by_skill.insert(key, Arc::clone(&record));
        self.by_id.insert(record.id.0, record);
    }

    /// The current (highest-version) skill for `(agent, name)`.
    #[must_use]
    pub fn latest(&self, agent: AgentId, name: &str) -> Option<Arc<ProceduralRecord>> {
        let target = (agent.0, name.to_owned(), u32::MAX);
        match self.by_skill.floor(&target) {
            Some(((a, n, _v), rec)) if a == agent.0 && n == name => Some(rec),
            _ => None,
        }
    }

    /// Record a new version of `(agent, name)`: `version = prev + 1`, with
    /// `supersedes` linking the previous version. Returns the new version's id.
    pub fn put_skill(
        &self,
        agent: AgentId,
        name: &str,
        steps: Vec<u8>,
        valid_from: Timestamp,
    ) -> Result<MemoryId> {
        let mut w = self.writer();
        if w.failed {
            return Err(StorageError::Wal(
                "writer aborted by a prior failed write; reopen the store".into(),
            ));
        }
        let now = self.next_tx(&mut w);
        let wal_tx = w.wal_tx_id;

        let prev = self.latest(agent, name);
        let (version, supersedes) = match &prev {
            Some(p) => (p.version.saturating_add(1), Some(p.id)),
            None => (1, None),
        };
        let id = Self::mint_id(&mut w, now);
        let record = ProceduralRecord {
            id,
            agent_id: agent,
            name: name.to_owned(),
            version,
            steps,
            supersedes,
            valid_from,
            tx_from: Timestamp(now),
        };
        let bytes = record.encode()?;
        match w
            .wal
            .append(wal_tx, WalOp::Put(RecordKind::Procedural), &bytes)
        {
            Ok(_) => {}
            Err(e) => {
                w.failed = true;
                return Err(e);
            }
        }
        self.index(Arc::new(record));
        Ok(id)
    }

    /// A specific version of `(agent, name)`.
    #[must_use]
    pub fn get_version(
        &self,
        agent: AgentId,
        name: &str,
        version: u32,
    ) -> Option<Arc<ProceduralRecord>> {
        self.by_skill.get(&(agent.0, name.to_owned(), version))
    }

    /// Every version of `(agent, name)`, ascending by version.
    #[must_use]
    pub fn history(&self, agent: AgentId, name: &str) -> Vec<Arc<ProceduralRecord>> {
        let lo = (agent.0, name.to_owned(), u32::MIN);
        let hi = (agent.0, name.to_owned(), u32::MAX);
        self.by_skill.range(lo..=hi).map(|(_, v)| v).collect()
    }

    /// Look up a skill version by id.
    #[must_use]
    pub fn get_by_id(&self, id: MemoryId) -> Option<Arc<ProceduralRecord>> {
        self.by_id.get(&id.0)
    }

    /// Make all skill versions since the last commit durable.
    pub fn commit(&self) -> Result<()> {
        let mut w = self.writer();
        if w.failed {
            return Err(StorageError::Wal(
                "writer aborted by a prior failed write; reopen the store".into(),
            ));
        }
        let tx = w.wal_tx_id;
        w.wal.commit(tx)?;
        w.wal_tx_id = w.wal_tx_id.saturating_add(1);
        Ok(())
    }

    /// The total number of skill versions stored.
    #[must_use]
    pub fn version_count(&self) -> usize {
        self.by_id.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const A: AgentId = AgentId(1);

    #[test]
    fn versions_chain_via_supersedes() {
        let dir = tempdir().unwrap();
        let store = ProceduralStore::create(dir.path().join("p.wal")).unwrap();
        let v1 = store
            .put_skill(A, "deploy", b"a".to_vec(), Timestamp(0))
            .unwrap();
        let v2 = store
            .put_skill(A, "deploy", b"ab".to_vec(), Timestamp(0))
            .unwrap();
        let v3 = store
            .put_skill(A, "deploy", b"abc".to_vec(), Timestamp(0))
            .unwrap();
        store.commit().unwrap();

        let latest = store.latest(A, "deploy").unwrap();
        assert_eq!(latest.version, 3);
        assert_eq!(latest.id, v3);
        assert_eq!(latest.supersedes, Some(v2));
        assert_eq!(store.get_version(A, "deploy", 1).unwrap().id, v1);
        assert_eq!(
            store.get_version(A, "deploy", 2).unwrap().supersedes,
            Some(v1)
        );
        assert!(store.get_version(A, "deploy", 99).is_none());

        let history = store.history(A, "deploy");
        assert_eq!(history.len(), 3);
        assert!(history.windows(2).all(|w| w[0].version < w[1].version));
    }

    #[test]
    fn skills_are_isolated_by_name_and_agent() {
        let dir = tempdir().unwrap();
        let store = ProceduralStore::create(dir.path().join("p.wal")).unwrap();
        store
            .put_skill(A, "deploy", b"x".to_vec(), Timestamp(0))
            .unwrap();
        store
            .put_skill(A, "rollback", b"y".to_vec(), Timestamp(0))
            .unwrap();
        store
            .put_skill(AgentId(2), "deploy", b"z".to_vec(), Timestamp(0))
            .unwrap();
        store.commit().unwrap();
        assert_eq!(store.latest(A, "deploy").unwrap().version, 1);
        assert_eq!(store.latest(A, "rollback").unwrap().version, 1);
        assert_eq!(store.latest(AgentId(2), "deploy").unwrap().steps, b"z");
        assert!(store.latest(A, "missing").is_none());
    }

    #[test]
    fn recovers_versions_after_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("p.wal");
        {
            let store = ProceduralStore::create(&path).unwrap();
            for v in 0..5 {
                store
                    .put_skill(A, "deploy", format!("v{v}").into_bytes(), Timestamp(0))
                    .unwrap();
            }
            store.commit().unwrap();
            // Uncommitted tail must not survive.
            store
                .put_skill(A, "deploy", b"orphan".to_vec(), Timestamp(0))
                .unwrap();
        }
        let store = ProceduralStore::open(&path).unwrap();
        assert_eq!(store.history(A, "deploy").len(), 5);
        assert_eq!(store.latest(A, "deploy").unwrap().version, 5);
        // A new version after reopen continues the chain.
        let v6 = store
            .put_skill(A, "deploy", b"v5".to_vec(), Timestamp(0))
            .unwrap();
        assert_eq!(store.latest(A, "deploy").unwrap().version, 6);
        assert_eq!(store.get_by_id(v6).unwrap().version, 6);
    }
}
