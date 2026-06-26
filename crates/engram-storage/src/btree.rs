//! A copy-on-write B-tree providing MVCC (R6).
//!
//! Nodes are immutable and shared via `Arc`. A write clones only the root→leaf
//! path it touches (structural sharing — everything else is reused), then swaps
//! the root pointer atomically with [`arc_swap::ArcSwap`]. Readers load the root
//! (or pin a [`Snapshot`]) without locks and observe a consistent version that
//! never changes underneath them, even as writers advance — that is the MVCC
//! guarantee: lock-free concurrent readers at independent snapshots.
//!
//! It is an order-`B` B-tree (keys *and* values live in every node). Writers are
//! serialized by an internal mutex (the engine drives writes through the WAL, one
//! at a time); reads are always lock-free.
//!
//! ```
//! use engram_storage::CowBTree;
//!
//! let tree = CowBTree::new();
//! tree.insert(2u32, "b");
//! tree.insert(1u32, "a");
//! let snap = tree.snapshot();      // pin this version
//! tree.insert(3u32, "c");          // not visible to `snap`
//! assert_eq!(snap.get(&3), None);
//! assert_eq!(tree.get(&3), Some("c"));
//! let all: Vec<_> = snap.iter().map(|(k, _)| k).collect();
//! assert_eq!(all, vec![1, 2]);     // sorted, snapshot-isolated
//! ```

use std::ops::{Bound, RangeBounds};
use std::sync::{Arc, Mutex, PoisonError};

use arc_swap::ArcSwap;

/// Branching factor: a node holds at most `B - 1` keys and `B` children.
const B: usize = 32;
/// Maximum keys per node before it must split.
const MAX_KEYS: usize = B - 1;

/// An immutable B-tree node. `children` is empty for a leaf; otherwise it has
/// exactly `keys.len() + 1` entries.
#[derive(Clone)]
struct Node<K, V> {
    keys: Vec<K>,
    vals: Vec<V>,
    children: Vec<Arc<Node<K, V>>>,
}

impl<K, V> Node<K, V> {
    fn empty_leaf() -> Self {
        Node {
            keys: Vec::new(),
            vals: Vec::new(),
            children: Vec::new(),
        }
    }

    fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

/// The result of inserting into a subtree.
enum Ins<K, V> {
    /// The subtree's new root (path clone, no structural growth above it).
    Done(Arc<Node<K, V>>),
    /// The node overflowed and split; the separator `(key, val)` must be lifted
    /// into the parent between `left` and `right`.
    Split {
        left: Arc<Node<K, V>>,
        key: K,
        val: V,
        right: Arc<Node<K, V>>,
    },
}

/// The atomically-swapped tree state: a root plus its entry count, kept together
/// so a [`Snapshot`] always sees a consistent `(root, len)` pair.
struct Inner<K, V> {
    root: Arc<Node<K, V>>,
    len: usize,
}

/// A copy-on-write, MVCC B-tree mapping `K` to `V`.
pub struct CowBTree<K, V> {
    state: ArcSwap<Inner<K, V>>,
    write_lock: Mutex<()>,
}

impl<K: Ord + Clone, V: Clone> Default for CowBTree<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord + Clone, V: Clone> CowBTree<K, V> {
    /// Create an empty tree.
    #[must_use]
    pub fn new() -> Self {
        CowBTree {
            state: ArcSwap::from_pointee(Inner {
                root: Arc::new(Node::empty_leaf()),
                len: 0,
            }),
            write_lock: Mutex::new(()),
        }
    }

    /// The number of entries in the current version.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.load().len
    }

    /// Whether the current version is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Look up `key` in the current version.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        get_in(&self.state.load().root, key)
    }

    /// Insert or update `key`, returning the previous value if it existed.
    ///
    /// Writers are serialized internally; readers never block.
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        // Recover rather than panic if a previous writer poisoned the lock: a
        // panic mid-insert cannot corrupt the tree (the new root is only
        // published on success), so the old root is always consistent.
        let _w = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let inner = self.state.load_full();
        let (ins, old) = insert_rec(&inner.root, key, value);
        let new_root = match ins {
            Ins::Done(node) => node,
            Ins::Split {
                left,
                key,
                val,
                right,
            } => Arc::new(Node {
                keys: vec![key],
                vals: vec![val],
                children: vec![left, right],
            }),
        };
        let new_len = if old.is_some() {
            inner.len
        } else {
            inner.len + 1
        };
        self.state.store(Arc::new(Inner {
            root: new_root,
            len: new_len,
        }));
        old
    }

    /// Pin the current version as an immutable [`Snapshot`].
    #[must_use]
    pub fn snapshot(&self) -> Snapshot<K, V> {
        Snapshot {
            inner: self.state.load_full(),
        }
    }

    /// Iterate the current version in ascending key order.
    #[must_use]
    pub fn iter(&self) -> Iter<K, V> {
        self.snapshot().into_iter_owned()
    }

    /// Iterate the entries of the current version within `range`, ascending.
    #[must_use]
    pub fn range<R: RangeBounds<K>>(&self, range: R) -> Iter<K, V> {
        self.snapshot().range_owned(range)
    }
}

/// An immutable, point-in-time view of a [`CowBTree`]. Reads on a snapshot are
/// unaffected by later writes to the tree.
pub struct Snapshot<K, V> {
    inner: Arc<Inner<K, V>>,
}

impl<K: Ord + Clone, V: Clone> Snapshot<K, V> {
    /// The number of entries in this snapshot.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len
    }

    /// Whether this snapshot is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.len == 0
    }

    /// Look up `key` in this snapshot.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        get_in(&self.inner.root, key)
    }

    /// Iterate this snapshot in ascending key order.
    #[must_use]
    pub fn iter(&self) -> Iter<K, V> {
        Iter::new(
            Arc::clone(&self.inner.root),
            Bound::Unbounded,
            Bound::Unbounded,
        )
    }

    /// Iterate the entries of this snapshot within `range`, ascending.
    #[must_use]
    pub fn range<R: RangeBounds<K>>(&self, range: R) -> Iter<K, V> {
        Iter::new(
            Arc::clone(&self.inner.root),
            clone_bound(range.start_bound()),
            clone_bound(range.end_bound()),
        )
    }

    fn into_iter_owned(self) -> Iter<K, V> {
        Iter::new(self.inner.root.clone(), Bound::Unbounded, Bound::Unbounded)
    }

    fn range_owned<R: RangeBounds<K>>(self, range: R) -> Iter<K, V> {
        let lo = clone_bound(range.start_bound());
        let hi = clone_bound(range.end_bound());
        Iter::new(self.inner.root.clone(), lo, hi)
    }
}

fn clone_bound<K: Clone>(b: Bound<&K>) -> Bound<K> {
    match b {
        Bound::Unbounded => Bound::Unbounded,
        Bound::Included(k) => Bound::Included(k.clone()),
        Bound::Excluded(k) => Bound::Excluded(k.clone()),
    }
}

/// Descend from `node` to find `key`.
fn get_in<K: Ord, V: Clone>(node: &Node<K, V>, key: &K) -> Option<V> {
    let mut node = node;
    loop {
        match node.keys.binary_search(key) {
            Ok(i) => return Some(node.vals[i].clone()),
            Err(i) => {
                if node.is_leaf() {
                    return None;
                }
                node = node.children[i].as_ref();
            }
        }
    }
}

/// Copy-on-write insert. Clones only the touched path.
fn insert_rec<K: Ord + Clone, V: Clone>(
    node: &Arc<Node<K, V>>,
    key: K,
    value: V,
) -> (Ins<K, V>, Option<V>) {
    match node.keys.binary_search(&key) {
        Ok(i) => {
            // Update in place (CoW): clone this node, replace the value.
            let mut n = (**node).clone();
            let old = std::mem::replace(&mut n.vals[i], value);
            (Ins::Done(Arc::new(n)), Some(old))
        }
        Err(i) => {
            if node.is_leaf() {
                let mut n = (**node).clone();
                n.keys.insert(i, key);
                n.vals.insert(i, value);
                let ins = if n.keys.len() > MAX_KEYS {
                    split_node(n)
                } else {
                    Ins::Done(Arc::new(n))
                };
                (ins, None)
            } else {
                let (child_ins, old) = insert_rec(&node.children[i], key, value);
                let mut n = (**node).clone();
                let ins = match child_ins {
                    Ins::Done(child) => {
                        n.children[i] = child;
                        Ins::Done(Arc::new(n))
                    }
                    Ins::Split {
                        left,
                        key,
                        val,
                        right,
                    } => {
                        n.children[i] = left;
                        n.keys.insert(i, key);
                        n.vals.insert(i, val);
                        n.children.insert(i + 1, right);
                        if n.keys.len() > MAX_KEYS {
                            split_node(n)
                        } else {
                            Ins::Done(Arc::new(n))
                        }
                    }
                };
                (ins, old)
            }
        }
    }
}

/// Split an overflowed node, lifting its median key/value as the separator.
fn split_node<K, V>(mut node: Node<K, V>) -> Ins<K, V> {
    let mid = node.keys.len() / 2;
    let right_children = if node.is_leaf() {
        Vec::new()
    } else {
        node.children.split_off(mid + 1)
    };
    let right_keys = node.keys.split_off(mid + 1);
    let right_vals = node.vals.split_off(mid + 1);
    // The element at `mid` becomes the separator. `remove(mid)` is in-bounds by
    // construction (a node only splits when it has > MAX_KEYS >= 1 keys).
    let sep_key = node.keys.remove(mid);
    let sep_val = node.vals.remove(mid);

    let left = Node {
        keys: node.keys,
        vals: node.vals,
        children: node.children,
    };
    let right = Node {
        keys: right_keys,
        vals: right_vals,
        children: right_children,
    };
    Ins::Split {
        left: Arc::new(left),
        key: sep_key,
        val: sep_val,
        right: Arc::new(right),
    }
}

/// A stack frame for the in-order iterator. `pos` interleaves children and keys
/// for an internal node (even ⇒ child `pos/2`, odd ⇒ key `(pos-1)/2`); for a
/// leaf it is simply the next key index.
struct Frame<K, V> {
    node: Arc<Node<K, V>>,
    pos: usize,
}

/// An ascending iterator over a snapshot, optionally bounded. Owns `Arc` node
/// handles, so it is valid independently of the tree or snapshot it came from.
pub struct Iter<K, V> {
    stack: Vec<Frame<K, V>>,
    hi: Bound<K>,
}

impl<K: Ord + Clone, V: Clone> Iter<K, V> {
    /// Build an iterator over `[lo, hi)` (bounds interpreted per `Bound`),
    /// seeking directly to `lo` so the cost is `O(log n + k)`.
    fn new(root: Arc<Node<K, V>>, lo: Bound<K>, hi: Bound<K>) -> Self {
        let mut stack = Vec::new();
        let mut node = root;
        loop {
            let idx = lower_bound(&node.keys, &lo);
            if node.is_leaf() {
                stack.push(Frame { node, pos: idx });
                break;
            }
            let child = Arc::clone(&node.children[idx]);
            // After `child` (pushed next, processed first) is exhausted, resume
            // at key `idx`.
            stack.push(Frame {
                node,
                pos: 2 * idx + 1,
            });
            node = child;
        }
        Iter { stack, hi }
    }

    /// `true` once `key` has reached or passed the upper bound.
    fn past_hi(&self, key: &K) -> bool {
        match &self.hi {
            Bound::Unbounded => false,
            Bound::Included(h) => key > h,
            Bound::Excluded(h) => key >= h,
        }
    }
}

/// First index in `keys` not excluded by the lower bound.
fn lower_bound<K: Ord>(keys: &[K], lo: &Bound<K>) -> usize {
    match lo {
        Bound::Unbounded => 0,
        Bound::Included(x) => keys.partition_point(|k| k < x),
        Bound::Excluded(x) => keys.partition_point(|k| k <= x),
    }
}

impl<K: Ord + Clone, V: Clone> Iterator for Iter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<(K, V)> {
        loop {
            // Borrow the top frame's node directly — no per-element Arc clone.
            let frame = self.stack.last_mut()?;
            if frame.node.is_leaf() {
                let i = frame.pos;
                if i < frame.node.keys.len() {
                    frame.pos += 1;
                    // Clone key+val while the frame is borrowed, then end the
                    // borrow before the `&self` bound check. (The wasted val
                    // clone happens only for the single out-of-range element.)
                    let key = frame.node.keys[i].clone();
                    let val = frame.node.vals[i].clone();
                    if self.past_hi(&key) {
                        self.stack.clear();
                        return None;
                    }
                    return Some((key, val));
                }
                self.stack.pop();
            } else {
                let m = frame.node.keys.len();
                let pos = frame.pos;
                if pos > 2 * m {
                    self.stack.pop();
                    continue;
                }
                if pos % 2 == 0 {
                    // Descend child `pos / 2` — the only Arc clone on the path.
                    frame.pos += 1;
                    let child = Arc::clone(&frame.node.children[pos / 2]);
                    self.stack.push(Frame {
                        node: child,
                        pos: 0,
                    });
                } else {
                    let ki = (pos - 1) / 2;
                    frame.pos += 1;
                    if ki < m {
                        let key = frame.node.keys[ki].clone();
                        let val = frame.node.vals[ki].clone();
                        if self.past_hi(&key) {
                            self.stack.clear();
                            return None;
                        }
                        return Some((key, val));
                    }
                    self.stack.pop();
                }
            }
        }
    }
}

impl<K: Ord + Clone, V: Clone> IntoIterator for Snapshot<K, V> {
    type Item = (K, V);
    type IntoIter = Iter<K, V>;
    fn into_iter(self) -> Iter<K, V> {
        self.into_iter_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn empty_tree() {
        let t: CowBTree<u32, u32> = CowBTree::new();
        assert!(t.is_empty());
        assert_eq!(t.get(&1), None);
        assert_eq!(t.iter().count(), 0);
    }

    #[test]
    fn insert_get_and_upsert() {
        let t = CowBTree::new();
        assert_eq!(t.insert(1u32, "a"), None);
        assert_eq!(t.insert(2u32, "b"), None);
        assert_eq!(t.get(&1), Some("a"));
        assert_eq!(t.insert(1u32, "A"), Some("a")); // upsert returns old
        assert_eq!(t.get(&1), Some("A"));
        assert_eq!(t.len(), 2); // upsert does not grow
    }

    #[test]
    fn splits_keep_tree_sorted_and_complete() {
        let t = CowBTree::new();
        // Insert enough (descending) to force many splits.
        let n = 5000u32;
        for i in (0..n).rev() {
            t.insert(i, i * 10);
        }
        assert_eq!(t.len() as u32, n);
        for i in 0..n {
            assert_eq!(t.get(&i), Some(i * 10), "missing {i}");
        }
        let collected: Vec<u32> = t.iter().map(|(k, _)| k).collect();
        assert!(collected.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(collected.len() as u32, n);
    }

    #[test]
    fn range_queries_match_btreemap() {
        let t = CowBTree::new();
        let mut oracle = BTreeMap::new();
        for i in (0..1000u32).map(|i| (i * 7) % 1000) {
            t.insert(i, i);
            oracle.insert(i, i);
        }
        for (lo, hi) in [(0u32, 100u32), (250, 750), (900, 1000), (400, 401)] {
            let got: Vec<u32> = t.range(lo..hi).map(|(k, _)| k).collect();
            let want: Vec<u32> = oracle.range(lo..hi).map(|(k, _)| *k).collect();
            assert_eq!(got, want, "range {lo}..{hi}");
        }
        // Inclusive / unbounded variants.
        let inc: Vec<u32> = t.range(10..=20).map(|(k, _)| k).collect();
        assert_eq!(inc, (10..=20).collect::<Vec<_>>());
    }

    #[test]
    fn mvcc_snapshot_isolation() {
        let t = CowBTree::new();
        for i in 0..100u32 {
            t.insert(i, i);
        }
        let snap = t.snapshot();
        assert_eq!(snap.len(), 100);

        // Mutate the tree after taking the snapshot.
        for i in 100..200u32 {
            t.insert(i, i);
        }
        t.insert(0, 9999); // overwrite an existing key too

        // The snapshot is unchanged.
        assert_eq!(snap.len(), 100);
        assert_eq!(snap.get(&0), Some(0));
        assert_eq!(snap.get(&150), None);
        let snap_keys: Vec<u32> = snap.iter().map(|(k, _)| k).collect();
        assert_eq!(snap_keys, (0..100).collect::<Vec<_>>());

        // The live tree reflects the writes.
        assert_eq!(t.len(), 200);
        assert_eq!(t.get(&0), Some(9999));
        assert_eq!(t.get(&150), Some(150));
    }

    #[test]
    fn concurrent_readers_during_writes() {
        use std::sync::Arc as StdArc;
        use std::thread;

        let t = StdArc::new(CowBTree::new());
        for i in 0..1000u32 {
            t.insert(i, i);
        }

        let mut readers = Vec::new();
        for _ in 0..4 {
            let t = StdArc::clone(&t);
            readers.push(thread::spawn(move || {
                for _ in 0..50 {
                    let snap = t.snapshot();
                    let len = snap.len();
                    let mut count = 0;
                    let mut prev: Option<u32> = None;
                    for (k, v) in snap.iter() {
                        assert_eq!(k, v);
                        if let Some(p) = prev {
                            assert!(k > p); // strictly ascending
                        }
                        prev = Some(k);
                        count += 1;
                    }
                    // The snapshot is internally consistent regardless of writers.
                    assert_eq!(count, len);
                }
            }));
        }

        // Writer advances the tree while readers hold older snapshots.
        for i in 1000..6000u32 {
            t.insert(i, i);
        }
        for r in readers {
            r.join().unwrap();
        }
        assert_eq!(t.len(), 6000);
    }

    #[test]
    fn large_random_insert_and_scan() {
        // Deterministic pseudo-random permutation via a multiplicative hash.
        let n: u64 = 100_000;
        let t = CowBTree::new();
        let mut oracle = BTreeMap::new();
        let mut x = 1u64;
        for _ in 0..n {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let k = x >> 16; // spread
            t.insert(k, k);
            oracle.insert(k, k);
        }
        assert_eq!(t.len(), oracle.len());
        let got: Vec<u64> = t.iter().map(|(k, _)| k).collect();
        let want: Vec<u64> = oracle.keys().copied().collect();
        assert_eq!(got, want);
    }

    #[test]
    #[ignore = "heavy; run with --release -- --ignored to prove the 1M-key gate"]
    fn one_million_keys() {
        let n: u64 = 1_000_000;
        let t = CowBTree::new();
        let mut x = 1u64;
        for _ in 0..n {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            t.insert(x, x);
        }
        // Scans are sorted and complete.
        let mut prev = None;
        let mut count = 0u64;
        for (k, _) in t.iter() {
            if let Some(p) = prev {
                assert!(k > p);
            }
            prev = Some(k);
            count += 1;
        }
        assert_eq!(count, t.len() as u64);
        assert!(count > n * 99 / 100); // few collisions from the LCG
    }
}
