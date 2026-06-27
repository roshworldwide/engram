//! The **causal-provenance DAG** store (R5, 2d).
//!
//! Every memory can link to the memories/events that caused it. Edges are stored
//! twice — a forward adjacency `(from, to)` and a reverse adjacency `(to, from)`
//! — in two CoW B-trees, so "what did X cause" and "what caused X" are both
//! `O(log n + k)` prefix scans. The graph is kept **acyclic**: [`add_edge`](CausalDag::add_edge)
//! rejects any edge that would close a cycle.
//!
//! Provenance is traced with breadth-first search:
//! [`find_provenance_chain`](CausalDag::find_provenance_chain) returns all
//! transitive causes of a memory, and [`find_path`](CausalDag::find_path) returns
//! a causal path between two memories.
//!
//! A DAG can be durable (WAL-backed, [`create`](CausalDag::create)/[`open`](CausalDag::open))
//! or ephemeral ([`in_memory`](CausalDag::in_memory)).
//!
//! ```
//! use engram_core::{EdgeType, MemoryId};
//! use engram_storage::CausalDag;
//!
//! let dag = CausalDag::in_memory();
//! let (o, a) = (MemoryId(42), MemoryId(7)); // observation O caused action A
//! dag.add_edge(o, a, EdgeType::Triggered).unwrap();
//! assert_eq!(dag.find_provenance_chain(a), vec![o]); // A's provenance is O
//! assert!(dag.add_edge(a, o, EdgeType::Triggered).is_err()); // would create a cycle
//! ```

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use engram_core::{from_msgpack, CausalEdge, EdgeType, MemoryId, Record, RecordKind};

use crate::btree::CowBTree;
use crate::error::{Result, StorageError};
use crate::wal::{Wal, WalOp};

struct Writer {
    /// `None` for an in-memory (ephemeral) DAG.
    wal: Option<Wal>,
    wal_tx_id: u64,
    failed: bool,
}

/// An acyclic causal-provenance graph with forward and reverse adjacency.
pub struct CausalDag {
    writer: Mutex<Writer>,
    /// `(from, to) → edge_type` — forward adjacency (effects of `from`).
    forward: CowBTree<(u128, u128), EdgeType>,
    /// `(to, from) → edge_type` — reverse adjacency (causes of `to`).
    reverse: CowBTree<(u128, u128), EdgeType>,
}

impl CausalDag {
    /// An ephemeral, non-durable DAG (no WAL).
    #[must_use]
    pub fn in_memory() -> Self {
        Self::with_writer(None, 0)
    }

    /// Create a fresh durable DAG backed by a new WAL at `path`.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::with_writer(Some(Wal::create(path)?), 0))
    }

    /// Open an existing durable DAG, replaying committed edges. Edges were
    /// validated acyclic when added, so replay is trusted (no re-checking).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let recovered = Wal::recover(path)?;
        let wal = Wal::open(path)?;
        let dag = Self::with_writer(Some(wal), recovered.max_tx_id.saturating_add(1));
        for entry in &recovered.entries {
            if matches!(entry.op, WalOp::Put(RecordKind::CausalEdge)) {
                let edge: CausalEdge = from_msgpack(&entry.record).map_err(|e| {
                    engram_core::EngramError::Decode(format!(
                        "causal edge at lsn {} tx {}: {e}",
                        entry.lsn, entry.tx_id
                    ))
                })?;
                dag.link(edge);
            }
        }
        Ok(dag)
    }

    fn with_writer(wal: Option<Wal>, wal_tx_id: u64) -> Self {
        CausalDag {
            writer: Mutex::new(Writer {
                wal,
                wal_tx_id,
                failed: false,
            }),
            forward: CowBTree::new(),
            reverse: CowBTree::new(),
        }
    }

    fn writer(&self) -> MutexGuard<'_, Writer> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Insert an edge into both adjacency indexes (no validation, no WAL).
    fn link(&self, edge: CausalEdge) {
        self.forward
            .insert((edge.from_id.0, edge.to_id.0), edge.edge_type);
        self.reverse
            .insert((edge.to_id.0, edge.from_id.0), edge.edge_type);
    }

    /// Whether `target` is reachable from `start` by following forward edges.
    fn reachable(&self, start: u128, target: u128) -> bool {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);
        while let Some(cur) = queue.pop_front() {
            if cur == target {
                return true;
            }
            for (effect, _) in self.effects_of(MemoryId(cur)) {
                if visited.insert(effect.0) {
                    queue.push_back(effect.0);
                }
            }
        }
        false
    }

    /// Add a causal edge `from → to`. Rejects a self-loop or any edge that would
    /// create a cycle (keeping the provenance graph acyclic). Re-adding an
    /// existing edge updates its [`EdgeType`].
    pub fn add_edge(&self, from: MemoryId, to: MemoryId, edge_type: EdgeType) -> Result<()> {
        let mut w = self.writer();
        if w.failed {
            return Err(StorageError::Wal(
                "writer aborted by a prior failed write; reopen the DAG".into(),
            ));
        }
        if from == to {
            return Err(StorageError::Cycle { from, to });
        }
        // Adding from→to closes a cycle iff `from` is already reachable from `to`.
        if self.reachable(to.0, from.0) {
            return Err(StorageError::Cycle { from, to });
        }

        let edge = CausalEdge {
            from_id: from,
            to_id: to,
            edge_type,
        };
        if w.wal.is_some() {
            // Read fields out before the mutable deref-borrow of `w.wal`
            // (field access through MutexGuard's Deref borrows the whole guard).
            let tx = w.wal_tx_id;
            let bytes = edge.encode()?;
            let res = match w.wal.as_mut() {
                Some(wal) => wal.append(tx, WalOp::Put(RecordKind::CausalEdge), &bytes),
                None => Ok(0),
            };
            if let Err(e) = res {
                w.failed = true;
                return Err(e);
            }
        }
        self.link(edge);
        Ok(())
    }

    /// The direct effects of `from` — the memories it caused — with edge types.
    #[must_use]
    pub fn effects_of(&self, from: MemoryId) -> Vec<(MemoryId, EdgeType)> {
        self.forward
            .range((from.0, u128::MIN)..=(from.0, u128::MAX))
            .map(|((_, to), et)| (MemoryId(to), et))
            .collect()
    }

    /// The direct causes of `to` — the memories that caused it — with edge types.
    #[must_use]
    pub fn causes_of(&self, to: MemoryId) -> Vec<(MemoryId, EdgeType)> {
        self.reverse
            .range((to.0, u128::MIN)..=(to.0, u128::MAX))
            .map(|((_, from), et)| (MemoryId(from), et))
            .collect()
    }

    /// All transitive causes of `node` (its provenance), in breadth-first order.
    /// `node` itself is not included.
    #[must_use]
    pub fn find_provenance_chain(&self, node: MemoryId) -> Vec<MemoryId> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut order = Vec::new();
        visited.insert(node.0);
        queue.push_back(node.0);
        while let Some(cur) = queue.pop_front() {
            for (cause, _) in self.causes_of(MemoryId(cur)) {
                if visited.insert(cause.0) {
                    order.push(cause);
                    queue.push_back(cause.0);
                }
            }
        }
        order
    }

    /// A causal path `from → … → to` (shortest, breadth-first), inclusive of both
    /// endpoints, or `None` if `to` is not reachable from `from`.
    #[must_use]
    pub fn find_path(&self, from: MemoryId, to: MemoryId) -> Option<Vec<MemoryId>> {
        if from == to {
            return Some(vec![from]);
        }
        let mut visited = HashSet::new();
        let mut parent: HashMap<u128, u128> = HashMap::new();
        let mut queue = VecDeque::new();
        visited.insert(from.0);
        queue.push_back(from.0);
        while let Some(cur) = queue.pop_front() {
            for (effect, _) in self.effects_of(MemoryId(cur)) {
                if visited.insert(effect.0) {
                    parent.insert(effect.0, cur);
                    if effect == to {
                        // Reconstruct from `to` back to `from`.
                        let mut path = vec![to.0];
                        let mut p = to.0;
                        while p != from.0 {
                            p = parent[&p];
                            path.push(p);
                        }
                        path.reverse();
                        return Some(path.into_iter().map(MemoryId).collect());
                    }
                    queue.push_back(effect.0);
                }
            }
        }
        None
    }

    /// Make all edges since the last commit durable (no-op for an in-memory DAG).
    pub fn commit(&self) -> Result<()> {
        let mut w = self.writer();
        if w.failed {
            return Err(StorageError::Wal(
                "writer aborted by a prior failed write; reopen the DAG".into(),
            ));
        }
        let tx = w.wal_tx_id;
        if let Some(wal) = w.wal.as_mut() {
            wal.commit(tx)?;
            w.wal_tx_id = w.wal_tx_id.saturating_add(1);
        }
        Ok(())
    }

    /// The number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.forward.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const T: EdgeType = EdgeType::Triggered;

    fn id(n: u128) -> MemoryId {
        MemoryId(n)
    }

    #[test]
    fn effects_and_causes() {
        let dag = CausalDag::in_memory();
        dag.add_edge(id(1), id(2), T).unwrap();
        dag.add_edge(id(1), id(3), T).unwrap();
        dag.add_edge(id(4), id(2), EdgeType::Contradicted).unwrap();

        let effects: Vec<u128> = dag.effects_of(id(1)).iter().map(|(m, _)| m.0).collect();
        assert_eq!(effects, vec![2, 3]);
        let causes: Vec<u128> = dag.causes_of(id(2)).iter().map(|(m, _)| m.0).collect();
        assert_eq!(causes, vec![1, 4]);
        assert_eq!(dag.causes_of(id(2))[1].1, EdgeType::Contradicted);
    }

    #[test]
    fn rejects_cycles_and_self_loops() {
        let dag = CausalDag::in_memory();
        dag.add_edge(id(1), id(2), T).unwrap();
        dag.add_edge(id(2), id(3), T).unwrap();
        // 3 -> 1 would close 1->2->3->1.
        assert!(matches!(
            dag.add_edge(id(3), id(1), T),
            Err(StorageError::Cycle { .. })
        ));
        // Self-loop.
        assert!(matches!(
            dag.add_edge(id(5), id(5), T),
            Err(StorageError::Cycle { .. })
        ));
        // The rejected edges left the graph unchanged.
        assert_eq!(dag.edge_count(), 2);
        // A non-cycling edge is still fine.
        dag.add_edge(id(3), id(4), T).unwrap();
        assert_eq!(dag.edge_count(), 3);
    }

    #[test]
    fn provenance_chain_and_path() {
        let dag = CausalDag::in_memory();
        // 1 -> 2 -> 4, 1 -> 3 -> 4 (diamond)
        for (a, b) in [(1, 2), (1, 3), (2, 4), (3, 4)] {
            dag.add_edge(id(a), id(b), T).unwrap();
        }
        // Provenance of 4 is {1,2,3}.
        let mut prov: Vec<u128> = dag
            .find_provenance_chain(id(4))
            .iter()
            .map(|m| m.0)
            .collect();
        prov.sort_unstable();
        assert_eq!(prov, vec![1, 2, 3]);
        assert!(dag.find_provenance_chain(id(1)).is_empty()); // a root

        // A path from 1 to 4 exists and is a real causal chain.
        let path: Vec<u128> = dag
            .find_path(id(1), id(4))
            .unwrap()
            .iter()
            .map(|m| m.0)
            .collect();
        assert_eq!(path.first(), Some(&1));
        assert_eq!(path.last(), Some(&4));
        assert!(dag.find_path(id(4), id(1)).is_none()); // acyclic: no back-path
        assert_eq!(dag.find_path(id(1), id(1)), Some(vec![id(1)]));
    }

    #[test]
    fn deep_chain_provenance() {
        let dag = CausalDag::in_memory();
        for i in 0..1000u128 {
            dag.add_edge(id(i), id(i + 1), T).unwrap(); // 0 -> 1 -> ... -> 1000
        }
        let prov = dag.find_provenance_chain(id(1000));
        assert_eq!(prov.len(), 1000); // all ancestors 0..=999
        assert_eq!(prov.first().unwrap().0, 999); // nearest cause first (BFS)
        let path = dag.find_path(id(0), id(1000)).unwrap();
        assert_eq!(path.len(), 1001);
    }

    #[test]
    fn durable_edges_recover() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dag.wal");
        {
            let dag = CausalDag::create(&path).unwrap();
            for (a, b) in [(1, 2), (2, 3), (1, 3)] {
                dag.add_edge(id(a), id(b), T).unwrap();
            }
            dag.commit().unwrap();
            // Uncommitted edge must not survive.
            dag.add_edge(id(3), id(4), T).unwrap();
        }
        let dag = CausalDag::open(&path).unwrap();
        assert_eq!(dag.edge_count(), 3);
        assert!(dag.find_path(id(4), id(1)).is_none());
        assert_eq!(dag.find_provenance_chain(id(3)).len(), 2); // 1 and 2
                                                               // The acyclic invariant still holds after reopen.
        assert!(matches!(
            dag.add_edge(id(3), id(1), T),
            Err(StorageError::Cycle { .. })
        ));
    }
}
