//! The **working** memory store (R2, 2e): a bounded FIFO scratchpad.
//!
//! Working memory is the agent's short-term scratchpad: small, fast, and
//! ephemeral (no WAL). It holds at most [`capacity`](WorkingMemory::capacity)
//! entries; pushing beyond that evicts the oldest entry FIFO-style. Eviction is
//! the hand-off point to **consolidation** (Phase 4): the evicted entry is both
//! returned from [`push`](WorkingMemory::push) and passed to an optional
//! consolidation hook, so a promoted long-term belief can be derived from it.
//!
//! ```
//! use engram_core::{AgentId, MemoryId, SessionId, Timestamp, WorkingMemoryRecord};
//! use engram_storage::WorkingMemory;
//!
//! let wm = WorkingMemory::new(2);
//! let rec = |n: u128| WorkingMemoryRecord {
//!     id: MemoryId(n), agent_id: AgentId(1), session_id: SessionId(1),
//!     created: Timestamp(n as i64), payload: vec![n as u8],
//! };
//! assert_eq!(wm.push(rec(1)), None);
//! assert_eq!(wm.push(rec(2)), None);
//! // Over capacity: the oldest (id 1) is evicted and returned.
//! assert_eq!(wm.push(rec(3)).map(|r| r.id), Some(MemoryId(1)));
//! assert_eq!(wm.ids(), vec![MemoryId(2), MemoryId(3)]);
//! ```

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use engram_core::{MemoryId, WorkingMemoryRecord};

/// The default working-memory capacity.
pub const DEFAULT_CAPACITY: usize = 50;

/// A consolidation hook, invoked with each evicted entry.
type EvictHook = Box<dyn Fn(&WorkingMemoryRecord) + Send + Sync>;

/// A bounded FIFO scratchpad. Cheap to share (`Arc<WorkingMemory>`).
pub struct WorkingMemory {
    capacity: usize,
    items: Mutex<VecDeque<WorkingMemoryRecord>>,
    on_evict: Option<EvictHook>,
}

impl Default for WorkingMemory {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl WorkingMemory {
    /// Create a working memory bounded to `capacity` entries.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        WorkingMemory {
            capacity,
            items: Mutex::new(VecDeque::new()),
            on_evict: None,
        }
    }

    /// Create a working memory with a consolidation hook called on each eviction.
    #[must_use]
    pub fn with_hook<F>(capacity: usize, hook: F) -> Self
    where
        F: Fn(&WorkingMemoryRecord) + Send + Sync + 'static,
    {
        WorkingMemory {
            capacity,
            items: Mutex::new(VecDeque::new()),
            on_evict: Some(Box::new(hook)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<WorkingMemoryRecord>> {
        self.items.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Push an entry. If this exceeds the capacity, the oldest entry is evicted,
    /// passed to the consolidation hook (if any), and returned.
    pub fn push(&self, record: WorkingMemoryRecord) -> Option<WorkingMemoryRecord> {
        let mut items = self.lock();
        items.push_back(record);
        if items.len() > self.capacity {
            let evicted = items.pop_front();
            // Release the lock before the hook so it can call back into us.
            drop(items);
            if let (Some(evicted), Some(hook)) = (&evicted, &self.on_evict) {
                hook(evicted);
            }
            evicted
        } else {
            None
        }
    }

    /// A snapshot of the current entries, oldest first.
    #[must_use]
    pub fn items(&self) -> Vec<WorkingMemoryRecord> {
        self.lock().iter().cloned().collect()
    }

    /// The ids of the current entries, oldest first.
    #[must_use]
    pub fn ids(&self) -> Vec<MemoryId> {
        self.lock().iter().map(|r| r.id).collect()
    }

    /// The number of entries currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether the scratchpad is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// The maximum number of entries retained.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Drop all entries (without invoking the hook).
    pub fn clear(&self) {
        self.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_core::{AgentId, SessionId, Timestamp};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn rec(n: u128) -> WorkingMemoryRecord {
        WorkingMemoryRecord {
            id: MemoryId(n),
            agent_id: AgentId(1),
            session_id: SessionId(1),
            created: Timestamp(n as i64),
            payload: vec![n as u8],
        }
    }

    #[test]
    fn default_capacity_is_50() {
        assert_eq!(WorkingMemory::default().capacity(), 50);
    }

    #[test]
    fn fifo_eviction_when_full() {
        let wm = WorkingMemory::new(3);
        assert_eq!(wm.push(rec(1)), None);
        assert_eq!(wm.push(rec(2)), None);
        assert_eq!(wm.push(rec(3)), None);
        assert_eq!(wm.len(), 3);
        // Fourth push evicts the oldest (id 1).
        assert_eq!(wm.push(rec(4)).map(|r| r.id), Some(MemoryId(1)));
        assert_eq!(wm.ids(), vec![MemoryId(2), MemoryId(3), MemoryId(4)]);
        assert_eq!(wm.push(rec(5)).map(|r| r.id), Some(MemoryId(2)));
        assert_eq!(wm.ids(), vec![MemoryId(3), MemoryId(4), MemoryId(5)]);
    }

    #[test]
    fn eviction_invokes_consolidation_hook() {
        let consolidated = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&consolidated);
        let wm = WorkingMemory::with_hook(2, move |evicted| {
            // The hook receives each evicted entry (would promote a belief here).
            assert!(evicted.id.0 <= 3);
            seen.fetch_add(1, Ordering::SeqCst);
        });
        for n in 1..=5 {
            wm.push(rec(n));
        }
        // cap 2, pushed 5 -> 3 evictions (ids 1,2,3).
        assert_eq!(consolidated.load(Ordering::SeqCst), 3);
        assert_eq!(wm.ids(), vec![MemoryId(4), MemoryId(5)]);
    }

    #[test]
    fn clear_and_empty() {
        let wm = WorkingMemory::new(4);
        wm.push(rec(1));
        wm.push(rec(2));
        assert!(!wm.is_empty());
        wm.clear();
        assert!(wm.is_empty());
        assert_eq!(wm.ids(), vec![]);
    }
}
