//! Property tests proving the [`VectorClock`] causal order is a well-formed
//! partial order (3a): irreflexive/asymmetric/transitive happens-before, exactly
//! one causal relation between any two clocks, and `merge` is the least upper
//! bound (the join of the clock lattice). Plus byte round-trip + the Q5 size.

use std::cmp::Ordering;

use engram_consistency::{VectorClock, BYTES_PER_SLOT};
use engram_core::AgentInstanceId;
use proptest::prelude::*;

/// Small instance space so causality and concurrency are frequently exercised.
fn arb_clock() -> impl Strategy<Value = VectorClock> {
    prop::collection::vec((0u64..4, 0u64..20), 0..8).prop_map(|pairs| {
        VectorClock::from_counts(pairs.into_iter().map(|(i, c)| (AgentInstanceId(i), c)))
    })
}

fn le(a: &VectorClock, b: &VectorClock) -> bool {
    matches!(a.partial_cmp(b), Some(Ordering::Less | Ordering::Equal))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn happens_before_is_irreflexive(a in arb_clock()) {
        prop_assert!(!a.happens_before(&a));
        prop_assert_eq!(a.partial_cmp(&a), Some(Ordering::Equal));
    }

    #[test]
    fn happens_before_is_asymmetric(a in arb_clock(), b in arb_clock()) {
        if a.happens_before(&b) {
            prop_assert!(!b.happens_before(&a));
        }
    }

    #[test]
    fn happens_before_is_transitive(a in arb_clock(), b in arb_clock(), c in arb_clock()) {
        if a.happens_before(&b) && b.happens_before(&c) {
            prop_assert!(a.happens_before(&c));
        }
    }

    /// Any two clocks are related in exactly one way, consistent with `partial_cmp`.
    #[test]
    fn exactly_one_causal_relation(a in arb_clock(), b in arb_clock()) {
        let lt = a.happens_before(&b);
        let gt = b.happens_before(&a);
        let eq = a == b;
        let conc = a.concurrent_with(&b);
        prop_assert_eq!([lt, gt, eq, conc].iter().filter(|x| **x).count(), 1);

        let matches_cmp = match a.partial_cmp(&b) {
            Some(Ordering::Less) => lt,
            Some(Ordering::Greater) => gt,
            Some(Ordering::Equal) => eq,
            None => conc,
        };
        prop_assert!(matches_cmp);
    }

    #[test]
    fn concurrency_is_symmetric(a in arb_clock(), b in arb_clock()) {
        prop_assert_eq!(a.concurrent_with(&b), b.concurrent_with(&a));
    }

    /// `merge` is commutative and an upper bound of both operands.
    #[test]
    fn merge_is_commutative_upper_bound(a in arb_clock(), b in arb_clock()) {
        let ab = a.merged(&b);
        prop_assert_eq!(&ab, &b.merged(&a));
        prop_assert!(le(&a, &ab));
        prop_assert!(le(&b, &ab));
    }

    /// `merge` is the LEAST upper bound: any common upper bound `c` dominates it.
    #[test]
    fn merge_is_least_upper_bound(a in arb_clock(), b in arb_clock(), c in arb_clock()) {
        if le(&a, &c) && le(&b, &c) {
            prop_assert!(le(&a.merged(&b), &c));
        }
    }

    /// An instance's own event strictly advances its causal position.
    #[test]
    fn increment_advances_causally(a in arb_clock(), i in 0u64..6) {
        let mut b = a.clone();
        b.increment(AgentInstanceId(i));
        prop_assert!(a.happens_before(&b));
    }

    #[test]
    fn bytes_round_trip_and_size(a in arb_clock()) {
        let bytes = a.to_bytes();
        prop_assert_eq!(bytes.len(), a.len() * BYTES_PER_SLOT); // Q5: 16 bytes/slot
        prop_assert_eq!(VectorClock::from_bytes(&bytes), Some(a));
    }
}
