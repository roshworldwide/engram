//! Vector clocks — the metadata that makes Agent Causal Consistency cheap (R7).
//!
//! A [`VectorClock`] maps each agent instance to a monotonic counter. It is the
//! standard causal-history summary: comparing two clocks tells you whether one
//! event causally precedes another or whether they are concurrent, in
//! `O(|instances|)` time and space — no global coordination required.
//!
//! The causal "happens-before" relation `→` is the componentwise partial order:
//! `a → b` iff `a[i] ≤ b[i]` for every instance `i` and `a ≠ b`. Two clocks that
//! are incomparable are **concurrent**. This is exposed through [`PartialOrd`]:
//! `Less` ⇒ happens-before, `Greater` ⇒ happens-after, `Equal` ⇒ identical,
//! `None` ⇒ concurrent.
//!
//! **Invariant:** zero-valued entries are never stored, so two clocks are equal
//! iff their stored maps are equal (an instance at counter 0 is the same as an
//! absent instance).
//!
//! ## Metadata cost (Q5)
//!
//! [`VectorClock::to_bytes`] emits exactly **16 bytes per instance** (an 8-byte
//! instance id + an 8-byte counter), so ACC metadata is `O(|agents|)` with a
//! proven ≤ 16-byte-per-slot bound.
//!
//! ```
//! use engram_consistency::VectorClock;
//! use engram_core::AgentInstanceId;
//!
//! let (a, b) = (AgentInstanceId(1), AgentInstanceId(2));
//! let mut x = VectorClock::new();
//! x.increment(a);                 // a's event
//! let mut y = x.clone();
//! y.increment(b);                 // b observed a, then did its own event
//! assert!(x.happens_before(&y));  // x → y
//! assert!(!y.happens_before(&x));
//!
//! let mut z = VectorClock::new();
//! z.increment(b);                 // independent b event
//! assert!(x.concurrent_with(&z)); // neither precedes the other
//! ```

use std::cmp::Ordering;
use std::collections::HashMap;

use engram_core::AgentInstanceId;
use serde::{Deserialize, Serialize};

/// Bytes per instance slot in the [`to_bytes`](VectorClock::to_bytes) wire form.
pub const BYTES_PER_SLOT: usize = 16;

/// A vector clock: a causal-history summary over agent instances.
///
/// Maintains the invariant that no entry maps to `0` (absent ≡ zero), so
/// equality is structural.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorClock {
    clocks: HashMap<AgentInstanceId, u64>,
}

impl VectorClock {
    /// An empty clock (every instance at 0).
    #[must_use]
    pub fn new() -> Self {
        VectorClock {
            clocks: HashMap::new(),
        }
    }

    /// Build a clock from `(instance, counter)` pairs, dropping any zero counters.
    /// Later pairs for the same instance overwrite earlier ones.
    pub fn from_counts<I>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (AgentInstanceId, u64)>,
    {
        let mut clocks = HashMap::new();
        for (id, count) in pairs {
            if count == 0 {
                clocks.remove(&id);
            } else {
                clocks.insert(id, count);
            }
        }
        VectorClock { clocks }
    }

    /// The counter for `id` (0 if the instance is absent).
    #[must_use]
    pub fn get(&self, id: AgentInstanceId) -> u64 {
        self.clocks.get(&id).copied().unwrap_or(0)
    }

    /// Advance `id`'s counter by one, returning the new value. This is what an
    /// instance does for each of its own events.
    pub fn increment(&mut self, id: AgentInstanceId) -> u64 {
        let entry = self.clocks.entry(id).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Merge `other` in elementwise: `self[i] = max(self[i], other[i])`. This is
    /// what an instance does when it observes another's state (the join in the
    /// clock lattice).
    pub fn merge(&mut self, other: &VectorClock) {
        for (&id, &count) in &other.clocks {
            let entry = self.clocks.entry(id).or_insert(0);
            if count > *entry {
                *entry = count;
            }
        }
    }

    /// A new clock that is the elementwise max (least upper bound) of `self` and
    /// `other`, without mutating either.
    #[must_use]
    pub fn merged(&self, other: &VectorClock) -> VectorClock {
        let mut out = self.clone();
        out.merge(other);
        out
    }

    /// Whether `self` causally precedes `other` (`self → other`): `self ≤ other`
    /// componentwise and `self ≠ other`.
    #[must_use]
    pub fn happens_before(&self, other: &VectorClock) -> bool {
        self.partial_cmp(other) == Some(Ordering::Less)
    }

    /// Whether `self` and `other` are concurrent (causally incomparable).
    #[must_use]
    pub fn concurrent_with(&self, other: &VectorClock) -> bool {
        self.partial_cmp(other).is_none()
    }

    /// The number of instances tracked (non-zero counters).
    #[must_use]
    pub fn len(&self) -> usize {
        self.clocks.len()
    }

    /// Whether the clock is empty (all instances at 0).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.clocks.is_empty()
    }

    /// Iterate `(instance, counter)` pairs (unordered).
    pub fn iter(&self) -> impl Iterator<Item = (AgentInstanceId, u64)> + '_ {
        self.clocks.iter().map(|(&id, &c)| (id, c))
    }

    /// Encode as a canonical, fixed-width byte string: for each instance (sorted
    /// by id) an 8-byte big-endian id followed by its 8-byte big-endian counter.
    /// Exactly `16 × len()` bytes (Q5).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut entries: Vec<(u64, u64)> = self.clocks.iter().map(|(&id, &c)| (id.0, c)).collect();
        entries.sort_unstable();
        let mut out = Vec::with_capacity(entries.len() * BYTES_PER_SLOT);
        for (id, count) in entries {
            out.extend_from_slice(&id.to_be_bytes());
            out.extend_from_slice(&count.to_be_bytes());
        }
        out
    }

    /// Decode the [`to_bytes`](Self::to_bytes) form. Returns `None` if the length
    /// is not a multiple of 16. Zero counters are dropped (invariant preserved).
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<VectorClock> {
        if !bytes.len().is_multiple_of(BYTES_PER_SLOT) {
            return None;
        }
        let mut clocks = HashMap::new();
        for chunk in bytes.chunks_exact(BYTES_PER_SLOT) {
            let id = u64::from_be_bytes(chunk[0..8].try_into().ok()?);
            let count = u64::from_be_bytes(chunk[8..16].try_into().ok()?);
            if count != 0 {
                clocks.insert(AgentInstanceId(id), count);
            }
        }
        Some(VectorClock { clocks })
    }
}

impl PartialOrd for VectorClock {
    /// The causal partial order: `Less` ⇒ `self → other`, `Greater` ⇒
    /// `other → self`, `Equal` ⇒ identical, `None` ⇒ concurrent.
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let mut self_le = true; // self ≤ other (componentwise)
        let mut other_le = true; // other ≤ self

        for (&id, &a) in &self.clocks {
            let b = other.get(id);
            if a > b {
                self_le = false;
            } else if b > a {
                other_le = false;
            }
        }
        // Instances present only in `other` have self-counter 0 < other-counter,
        // so `other` is not ≤ `self`.
        for id in other.clocks.keys() {
            if !self.clocks.contains_key(id) {
                other_le = false;
                break;
            }
        }

        match (self_le, other_le) {
            (true, true) => Some(Ordering::Equal),
            (true, false) => Some(Ordering::Less),
            (false, true) => Some(Ordering::Greater),
            (false, false) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(n: u64) -> AgentInstanceId {
        AgentInstanceId(n)
    }

    #[test]
    fn increment_and_get() {
        let mut c = VectorClock::new();
        assert_eq!(c.get(inst(1)), 0);
        assert_eq!(c.increment(inst(1)), 1);
        assert_eq!(c.increment(inst(1)), 2);
        assert_eq!(c.get(inst(1)), 2);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn happens_before_and_concurrent() {
        let mut a = VectorClock::new();
        a.increment(inst(1));
        let mut b = a.clone();
        b.increment(inst(2));
        assert!(a.happens_before(&b));
        assert!(!b.happens_before(&a));
        assert!(!a.concurrent_with(&b));

        let mut c = VectorClock::new();
        c.increment(inst(2));
        // a = {1:1}, c = {2:1} -> concurrent
        assert!(a.concurrent_with(&c));
        assert!(!a.happens_before(&c));
        assert!(!c.happens_before(&a));
    }

    #[test]
    fn merge_is_least_upper_bound() {
        let mut a = VectorClock::from_counts([(inst(1), 3), (inst(2), 1)]);
        let b = VectorClock::from_counts([(inst(2), 5), (inst(3), 2)]);
        a.merge(&b);
        assert_eq!(a.get(inst(1)), 3);
        assert_eq!(a.get(inst(2)), 5); // max(1, 5)
        assert_eq!(a.get(inst(3)), 2);
    }

    #[test]
    fn zero_counters_are_normalized_away() {
        let c = VectorClock::from_counts([(inst(1), 0), (inst(2), 4)]);
        assert_eq!(c.len(), 1);
        assert_eq!(c, VectorClock::from_counts([(inst(2), 4)]));
    }

    #[test]
    fn bytes_round_trip_and_size() {
        let c = VectorClock::from_counts([(inst(7), 100), (inst(3), 9), (inst(42), 1)]);
        let bytes = c.to_bytes();
        assert_eq!(bytes.len(), 3 * BYTES_PER_SLOT); // Q5: 16 bytes/slot
        assert_eq!(VectorClock::from_bytes(&bytes), Some(c));
        // Canonical (sorted) encoding.
        assert_eq!(VectorClock::from_bytes(&[0u8; 7]), None);
    }
}
