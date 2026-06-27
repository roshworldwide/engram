//! Agent Causal Consistency enforcement (R7).
//!
//! A [`CausalMemory`] is a shared, append-only log of writes; each agent instance
//! holds a [`Session`] that delivers writes **in causal order** using vector
//! clocks — no consensus, no total order, no global coordinator on the read/write
//! path. This realizes the §9 ACC contract:
//!
//! 1. **Prefix Closure** — a session only ever sees a causally-closed set of
//!    writes (if `W` is visible, every `W' → W` is visible).
//! 2. **Session Consistency (read-your-writes)** — a session always sees its own
//!    prior writes.
//! 3. **Monotonic Sessions** — visibility only grows; a write, once seen, is
//!    seen by every later read (or superseded by a causally-later write).
//! 4. **Causal Memory** — a write's causal dependencies (captured in its clock)
//!    are delivered before it, so a belief is never visible without the events it
//!    was derived from.
//!
//! ## Metadata (Q5)
//!
//! Each write carries exactly one [`VectorClock`] — `O(|instances|)` with the
//! proven 16-byte-per-slot bound. A write's *dependencies* are derived from its
//! clock (the clock minus the writer's own latest tick), so nothing extra is
//! stored.
//!
//! ## Single-agent mode
//!
//! The storage engine (Phase 2) uses **no** vector clocks; ACC is this separate,
//! opt-in layer. A lone instance's clock has a single slot and it never delivers
//! another instance's writes — there is no cross-instance overhead.
//!
//! ```
//! use engram_consistency::CausalMemory;
//! use engram_core::AgentInstanceId;
//!
//! let mem = CausalMemory::new();
//! let mut a = mem.session(AgentInstanceId(1));
//! let mut b = mem.session(AgentInstanceId(2));
//!
//! a.write("svc", "down");          // A records an observation
//! b.refresh();                     // B delivers A's write (causally)
//! assert_eq!(b.read_latest(b"svc"), Some(b"down".to_vec()));
//! b.write("action", "restart");    // B's action causally depends on A's write
//!
//! // A new instance that delivers B's action must also have A's observation.
//! let mut c = mem.session(AgentInstanceId(3));
//! c.refresh();
//! assert_eq!(c.read_latest(b"action"), Some(b"restart".to_vec()));
//! assert_eq!(c.read_latest(b"svc"), Some(b"down".to_vec())); // causal memory
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use engram_core::AgentInstanceId;

use crate::vector_clock::VectorClock;

/// A single write in the causal memory. The `clock` is the writer's vector clock
/// immediately after its own tick; its causal dependencies are the clock with
/// the writer's latest tick removed.
#[derive(Clone, Debug)]
pub struct WriteOp {
    /// Global append sequence (delivery-independent identity).
    pub id: u64,
    /// The instance that issued the write.
    pub instance: AgentInstanceId,
    /// Application key.
    pub key: Vec<u8>,
    /// Application value.
    pub value: Vec<u8>,
    /// The writer's vector clock after this write.
    pub clock: VectorClock,
}

impl WriteOp {
    /// Bytes of ACC metadata this op carries beyond its key/value (Q5): one
    /// vector clock at 16 bytes per instance slot.
    #[must_use]
    pub fn metadata_bytes(&self) -> usize {
        self.clock.to_bytes().len()
    }
}

/// A shared, append-only causal memory. Cheap to share (`Arc<CausalMemory>`).
///
/// The only synchronization is a brief lock to append/snapshot the log. That
/// lock is **not** a consistency coordinator: visibility is decided locally by
/// each session from vector clocks, with no consensus or total order. (In a real
/// deployment each instance keeps its own log and gossips; the shared `Vec` here
/// is the single-process stand-in for that union.)
#[derive(Default)]
pub struct CausalMemory {
    log: Mutex<Vec<Arc<WriteOp>>>,
    /// Debug-only guard: each `AgentInstanceId` must have at most one live
    /// session (a vector-clock slot has a single writer).
    #[cfg(debug_assertions)]
    issued: Mutex<HashSet<AgentInstanceId>>,
}

impl CausalMemory {
    /// Create an empty causal memory.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(CausalMemory::default())
    }

    /// Open a session for an agent instance.
    ///
    /// **Precondition:** each `AgentInstanceId` has at most one live session at a
    /// time. Two concurrent sessions sharing an instance id violate the
    /// single-writer-per-clock-slot assumption that causal delivery relies on and
    /// can strand writes. A `debug_assert` catches reuse in debug builds.
    pub fn session(self: &Arc<Self>, instance: AgentInstanceId) -> Session {
        #[cfg(debug_assertions)]
        {
            let mut issued = self
                .issued
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            debug_assert!(
                issued.insert(instance),
                "AgentInstanceId {instance:?} already has a live session"
            );
        }
        Session {
            mem: Arc::clone(self),
            instance,
            vc: VectorClock::new(),
            delivered: HashSet::new(),
            by_key: HashMap::new(),
            last_read: HashMap::new(),
            undelivered: Vec::new(),
            cursor: 0,
        }
    }

    /// Append a write under the global order, assigning its id.
    fn append(
        &self,
        instance: AgentInstanceId,
        key: Vec<u8>,
        value: Vec<u8>,
        clock: VectorClock,
    ) -> Arc<WriteOp> {
        let mut log = self.lock();
        let id = log.len() as u64;
        let op = Arc::new(WriteOp {
            id,
            instance,
            key,
            value,
            clock,
        });
        log.push(Arc::clone(&op));
        op
    }

    /// A snapshot of the whole log (used by tests as the global oracle).
    #[must_use]
    pub fn ops(&self) -> Vec<Arc<WriteOp>> {
        self.lock().clone()
    }

    /// Number of writes appended.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether the memory is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Arc<WriteOp>>> {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Copy newly-appended ops (from `cursor`) into `out`, advancing the cursor.
    fn drain_new(&self, cursor: &mut usize, out: &mut Vec<Arc<WriteOp>>) {
        let log = self.lock();
        for op in &log[*cursor..] {
            out.push(Arc::clone(op));
        }
        *cursor = log.len();
    }
}

/// One agent instance's causally-consistent view of the shared memory.
pub struct Session {
    mem: Arc<CausalMemory>,
    instance: AgentInstanceId,
    vc: VectorClock,
    delivered: HashSet<u64>,
    /// Per key, only the causally-maximal delivered writes (the live frontier).
    by_key: HashMap<Vec<u8>, Vec<Arc<WriteOp>>>,
    /// Per key, the clock of the last value returned by `read_latest`, so it can
    /// never regress to a concurrent sibling (Monotonic Sessions).
    last_read: HashMap<Vec<u8>, VectorClock>,
    /// Ops observed in the log but not yet causally deliverable.
    undelivered: Vec<Arc<WriteOp>>,
    /// How far into the global log this session has scanned.
    cursor: usize,
}

impl Session {
    /// The instance this session belongs to.
    #[must_use]
    pub fn instance(&self) -> AgentInstanceId {
        self.instance
    }

    /// This session's current causal frontier.
    #[must_use]
    pub fn clock(&self) -> &VectorClock {
        &self.vc
    }

    /// Record a write. It is appended to the shared memory and immediately
    /// visible to this session (read-your-writes). Returns the op.
    pub fn write(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Arc<WriteOp> {
        self.vc.increment(self.instance);
        let clock = self.vc.clone();
        let op = self
            .mem
            .append(self.instance, key.into(), value.into(), clock);
        // Own write: delivered with no further checks.
        self.delivered.insert(op.id);
        self.record(Arc::clone(&op));
        op
    }

    /// Record a delivered op into `by_key`, keeping only the causally-maximal
    /// frontier per key: skip it if an existing frontier op already supersedes
    /// it, otherwise drop the entries it supersedes and add it. This bounds
    /// `by_key` (and read cost) to the concurrency width, not full history.
    fn record(&mut self, op: Arc<WriteOp>) {
        let entry = self.by_key.entry(op.key.clone()).or_default();
        if entry.iter().any(|e| op.clock.happens_before(&e.clock)) {
            return;
        }
        entry.retain(|e| !e.clock.happens_before(&op.clock));
        entry.push(op);
    }

    /// Whether `op` can be delivered now: it is the next op from its instance and
    /// all of its cross-instance dependencies are already delivered here.
    fn can_deliver(&self, op: &WriteOp) -> bool {
        if op.clock.get(op.instance) != self.vc.get(op.instance) + 1 {
            return false;
        }
        op.clock
            .iter()
            .all(|(k, c)| k == op.instance || c <= self.vc.get(k))
    }

    fn deliver(&mut self, op: Arc<WriteOp>) {
        self.vc.merge(&op.clock);
        self.delivered.insert(op.id);
        self.record(op);
    }

    /// Pull newly-appended writes from the shared memory and deliver everything
    /// that is now causally ready. Returns the number of writes newly delivered.
    pub fn refresh(&mut self) -> usize {
        self.mem.drain_new(&mut self.cursor, &mut self.undelivered);
        let mut delivered = 0;
        loop {
            let mut progressed = false;
            let mut i = 0;
            while i < self.undelivered.len() {
                if self.delivered.contains(&self.undelivered[i].id) {
                    self.undelivered.swap_remove(i); // own write seen via the log
                    continue;
                }
                if self.can_deliver(&self.undelivered[i]) {
                    let op = self.undelivered.swap_remove(i);
                    self.deliver(op);
                    delivered += 1;
                    progressed = true;
                } else {
                    i += 1;
                }
            }
            if !progressed {
                break;
            }
        }
        delivered
    }

    /// The causally-maximal delivered values for `key` (the "frontier"): a single
    /// value unless concurrent writers produced siblings. Monotonic: a value here
    /// stays in the frontier until a causally-later write supersedes it.
    #[must_use]
    pub fn read(&self, key: &[u8]) -> Vec<Vec<u8>> {
        match self.by_key.get(key) {
            Some(ops) => ops.iter().map(|w| w.value.clone()).collect(),
            None => Vec::new(),
        }
    }

    /// A single deterministic value for `key`, chosen from the frontier by a fixed
    /// total-order tiebreak (clock bytes, then instance).
    ///
    /// **Monotonic (Session Consistency):** never regresses to a value concurrent
    /// with one already returned in this session — only a causally-later write can
    /// change the result. (Concurrent siblings are all retrievable via [`read`](Self::read).)
    pub fn read_latest(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        let (value, clock) = {
            let ops = self.by_key.get(key)?;
            let prev = self.last_read.get(key);
            let winner = ops
                .iter()
                .filter(|w| match prev {
                    None => true,
                    Some(p) => p == &w.clock || p.happens_before(&w.clock),
                })
                .max_by(|a, b| {
                    (a.clock.to_bytes(), a.instance.0).cmp(&(b.clock.to_bytes(), b.instance.0))
                })?;
            (winner.value.clone(), winner.clock.clone())
        };
        self.last_read.insert(key.to_vec(), clock);
        Some(value)
    }

    /// Whether the write with global id `op_id` has been delivered here.
    #[must_use]
    pub fn is_delivered(&self, op_id: u64) -> bool {
        self.delivered.contains(&op_id)
    }

    /// How many writes this session has delivered.
    #[must_use]
    pub fn delivered_count(&self) -> usize {
        self.delivered.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i(n: u64) -> AgentInstanceId {
        AgentInstanceId(n)
    }

    #[test]
    fn read_your_writes() {
        let mem = CausalMemory::new();
        let mut a = mem.session(i(1));
        a.write("k", "v1");
        assert_eq!(a.read_latest(b"k"), Some(b"v1".to_vec()));
        a.write("k", "v2");
        assert_eq!(a.read_latest(b"k"), Some(b"v2".to_vec()));
    }

    #[test]
    fn causal_delivery_respects_dependencies() {
        let mem = CausalMemory::new();
        let mut a = mem.session(i(1));
        let mut b = mem.session(i(2));
        let mut c = mem.session(i(3));

        a.write("svc", "down"); // op0
        b.refresh();
        b.write("action", "restart"); // op1 depends on op0 (B saw it)

        // C delivers both, in causal order: it cannot see the action without the
        // observation it was derived from (causal memory).
        c.refresh();
        assert_eq!(c.read_latest(b"action"), Some(b"restart".to_vec()));
        assert_eq!(c.read_latest(b"svc"), Some(b"down".to_vec()));
    }

    #[test]
    fn concurrent_writes_are_siblings_then_converge() {
        let mem = CausalMemory::new();
        let mut a = mem.session(i(1));
        let mut b = mem.session(i(2));
        a.write("k", "from-a");
        b.write("k", "from-b"); // concurrent (B hasn't seen A)
        a.refresh();
        b.refresh();
        // Both see two concurrent siblings...
        let mut sa = a.read(b"k");
        let mut sb = b.read(b"k");
        sa.sort();
        sb.sort();
        assert_eq!(sa, vec![b"from-a".to_vec(), b"from-b".to_vec()]);
        assert_eq!(sa, sb);
        // ...and read_latest converges to the same deterministic winner.
        assert_eq!(a.read_latest(b"k"), b.read_latest(b"k"));
    }

    #[test]
    fn metadata_is_one_clock_per_op() {
        let mem = CausalMemory::new();
        let mut a = mem.session(i(1));
        let mut b = mem.session(i(2));
        a.write("k", "v");
        b.refresh();
        let op = b.write("k", "w"); // clock now spans 2 instances
        assert_eq!(op.metadata_bytes(), 2 * crate::BYTES_PER_SLOT);
    }

    #[test]
    fn read_latest_is_monotonic_across_concurrent_siblings() {
        // Reproduces the reviewed flip: read "from-a", then a CONCURRENT "from-b"
        // is delivered — read_latest must NOT switch to the concurrent sibling.
        let mem = CausalMemory::new();
        let mut a = mem.session(i(1));
        let mut s = mem.session(i(9));
        a.write("k", "from-a"); // op0 {1:1}
        s.refresh();
        assert_eq!(s.read_latest(b"k"), Some(b"from-a".to_vec()));

        let mut b = mem.session(i(2));
        b.write("k", "from-b"); // op1 {2:1}, concurrent with op0
        s.refresh(); // s delivers the concurrent sibling
        let mut siblings = s.read(b"k");
        siblings.sort();
        assert_eq!(siblings, vec![b"from-a".to_vec(), b"from-b".to_vec()]);
        // Monotonic: still the value already returned, never the concurrent one.
        assert_eq!(s.read_latest(b"k"), Some(b"from-a".to_vec()));
    }

    #[test]
    fn read_latest_advances_on_causally_later_write() {
        let mem = CausalMemory::new();
        let mut a = mem.session(i(1));
        let mut s = mem.session(i(9));
        a.write("k", "v1");
        s.refresh();
        assert_eq!(s.read_latest(b"k"), Some(b"v1".to_vec()));
        a.write("k", "v2"); // causally later (same writer)
        s.refresh();
        assert_eq!(s.read_latest(b"k"), Some(b"v2".to_vec())); // advances
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "already has a live session")]
    fn reused_instance_id_panics_in_debug() {
        let mem = CausalMemory::new();
        let _a = mem.session(i(1));
        let _b = mem.session(i(1)); // violates the single-writer-per-slot contract
    }
}
