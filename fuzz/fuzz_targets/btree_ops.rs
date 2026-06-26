//! Fuzz target (Q2): the CoW B-tree must behave identically to a `BTreeMap`
//! under any sequence of operations, and never panic, lose data, or break
//! ordering. Differential against `std::collections::BTreeMap`.
//!
//! Run (nightly): `cargo +nightly fuzz run btree_ops`.
#![no_main]

use std::collections::BTreeMap;

use arbitrary::Arbitrary;
use engram_storage::CowBTree;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum Op {
    Insert(u16, u16),
    Get(u16),
    RangeCount(u16, u16),
}

fuzz_target!(|ops: Vec<Op>| {
    let tree = CowBTree::new();
    let mut oracle: BTreeMap<u16, u16> = BTreeMap::new();

    for op in ops {
        match op {
            Op::Insert(k, v) => assert_eq!(tree.insert(k, v), oracle.insert(k, v)),
            Op::Get(k) => assert_eq!(tree.get(&k), oracle.get(&k).copied()),
            Op::RangeCount(a, b) => {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                assert_eq!(tree.range(lo..hi).count(), oracle.range(lo..hi).count());
            }
        }
        assert_eq!(tree.len(), oracle.len());
    }

    let got: Vec<(u16, u16)> = tree.iter().collect();
    let want: Vec<(u16, u16)> = oracle.iter().map(|(k, v)| (*k, *v)).collect();
    assert_eq!(got, want);
});
