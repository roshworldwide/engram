//! Q1: ≥ 1,000 randomized multi-agent histories must satisfy every ACC invariant.
//!
//! Each case builds 2–5 instances and runs a random interleaving of writes and
//! refreshes, then checks the §9 contract against an independent vector-clock
//! oracle over the global write log:
//!
//! 1. **Prefix Closure / Causal Memory** — every delivered op's causal ancestors
//!    (by happens-before on clocks) are also delivered.
//! 2. **Read-your-writes** — a session has delivered all of its own writes.
//! 3. **Monotonic Sessions** — a session's delivered count never decreases.
//! 4. **Convergence** — after stabilizing refreshes, every session has delivered
//!    every write (eventual consistency, achieved with no global coordinator).

use engram_consistency::CausalMemory;
use engram_core::AgentInstanceId;
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    Write(u8, u8),
    Refresh,
}

fn arb_step() -> impl Strategy<Value = (u8, Op)> {
    (
        any::<u8>(), // raw instance selector (taken modulo instance count)
        prop_oneof![
            (0u8..4, any::<u8>()).prop_map(|(k, v)| Op::Write(k, v)),
            Just(Op::Refresh),
        ],
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1200))]

    #[test]
    fn acc_invariants_hold(
        num_instances in 2usize..=5,
        steps in prop::collection::vec(arb_step(), 0..60),
    ) {
        let mem = CausalMemory::new();
        let mut sessions: Vec<_> = (1..=num_instances as u64)
            .map(|n| mem.session(AgentInstanceId(n)))
            .collect();
        let mut prev_count = vec![0usize; num_instances];

        for (sel, op) in steps {
            let idx = (sel as usize) % num_instances;
            match op {
                Op::Write(k, v) => {
                    sessions[idx].write(vec![k], vec![v]);
                }
                Op::Refresh => {
                    sessions[idx].refresh();
                }
            }
            // (3) Monotonic Sessions: delivery never regresses.
            let count = sessions[idx].delivered_count();
            prop_assert!(count >= prev_count[idx]);
            prev_count[idx] = count;
        }

        // Stabilize: deliver everything that can be delivered.
        for s in &mut sessions {
            while s.refresh() > 0 {}
        }

        let log = mem.ops();
        for s in &sessions {
            // (2) Read-your-writes: own writes are all delivered.
            for op in &log {
                if op.instance == s.instance() {
                    prop_assert!(s.is_delivered(op.id), "RYW: missing own write {}", op.id);
                }
            }
            // (1) Prefix Closure / Causal Memory: a delivered op's causal
            // ancestors are delivered too.
            for op in &log {
                if s.is_delivered(op.id) {
                    for dep in &log {
                        if dep.clock.happens_before(&op.clock) {
                            prop_assert!(
                                s.is_delivered(dep.id),
                                "prefix closure: op {} visible without ancestor {}",
                                op.id,
                                dep.id
                            );
                        }
                    }
                }
            }
            // (4) Convergence: after stabilizing, everything is delivered.
            prop_assert_eq!(s.delivered_count(), log.len());
        }
    }
}
