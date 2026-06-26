//! Differential property test (1c): a `CowBTree` must behave exactly like a
//! `std::collections::BTreeMap` across random operations — same `insert`
//! return/`get`/`len`, same ordered iteration, same range queries — and
//! snapshots must be isolated from later writes.

use std::collections::BTreeMap;

use engram_storage::CowBTree;
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    Insert(u16, u16),
    Get(u16),
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (any::<u16>(), any::<u16>()).prop_map(|(k, v)| Op::Insert(k, v)),
        any::<u16>().prop_map(Op::Get),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn matches_btreemap(
        ops in prop::collection::vec(arb_op(), 0..600),
        ranges in prop::collection::vec((any::<u16>(), any::<u16>()), 0..12),
    ) {
        let tree = CowBTree::new();
        let mut oracle: BTreeMap<u16, u16> = BTreeMap::new();

        for op in &ops {
            match op {
                Op::Insert(k, v) => {
                    prop_assert_eq!(tree.insert(*k, *v), oracle.insert(*k, *v));
                }
                Op::Get(k) => {
                    prop_assert_eq!(tree.get(k), oracle.get(k).copied());
                }
            }
            prop_assert_eq!(tree.len(), oracle.len());
        }

        // Full ordered iteration is identical.
        let got: Vec<(u16, u16)> = tree.iter().collect();
        let want: Vec<(u16, u16)> = oracle.iter().map(|(k, v)| (*k, *v)).collect();
        prop_assert_eq!(got, want);

        // Range queries (half-open and inclusive) are identical.
        for (a, b) in ranges {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let got: Vec<(u16, u16)> = tree.range(lo..hi).collect();
            let want: Vec<(u16, u16)> = oracle.range(lo..hi).map(|(k, v)| (*k, *v)).collect();
            prop_assert_eq!(got, want);

            let got_inc: Vec<(u16, u16)> = tree.range(lo..=hi).collect();
            let want_inc: Vec<(u16, u16)> = oracle.range(lo..=hi).map(|(k, v)| (*k, *v)).collect();
            prop_assert_eq!(got_inc, want_inc);

            // floor(k) must equal the greatest entry <= k.
            for k in [lo, hi] {
                let got = tree.floor(&k);
                let want = oracle.range(..=k).next_back().map(|(k, v)| (*k, *v));
                prop_assert_eq!(got, want);
            }
        }

        // Snapshot isolation: a snapshot is unchanged by subsequent writes.
        let snap = tree.snapshot();
        let before: Vec<(u16, u16)> = snap.iter().collect();
        tree.insert(0, 12345);
        tree.insert(u16::MAX, 54321);
        let after: Vec<(u16, u16)> = snap.iter().collect();
        prop_assert_eq!(before, after);
    }
}
