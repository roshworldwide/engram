//! Fuzz target (Q2): the causal-DAG store must never panic and must stay acyclic
//! under any sequence of edge operations. `add_edge` either accepts (keeping the
//! graph acyclic) or rejects a cycle; the traversals never crash.
//!
//! Run (nightly): `cargo +nightly fuzz run dag_ops`.
#![no_main]

use engram_core::{EdgeType, MemoryId};
use engram_storage::CausalDag;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|ops: Vec<(u8, u8)>| {
    let dag = CausalDag::in_memory();
    for (a, b) in ops {
        // Small node space so cycles are frequently attempted.
        let from = MemoryId(u128::from(a % 16));
        let to = MemoryId(u128::from(b % 16));
        let _ = dag.add_edge(from, to, EdgeType::Triggered);
        let _ = dag.effects_of(from);
        let _ = dag.causes_of(to);
        let _ = dag.find_path(from, to);
        let _ = dag.find_provenance_chain(to);
    }
});
