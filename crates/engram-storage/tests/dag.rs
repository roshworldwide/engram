//! Property test for the causal-DAG store (2d): under any sequence of edge
//! additions, (1) the graph stays acyclic (verified by an independent Kahn's
//! topological sort), and (2) each `add_edge` is accepted/rejected exactly when
//! an independent reachability oracle says it would (not) form a cycle.

use std::collections::{HashMap, HashSet, VecDeque};

use engram_core::{EdgeType, MemoryId};
use engram_storage::CausalDag;
use proptest::prelude::*;

const NODES: u128 = 16;

fn collect_edges(dag: &CausalDag) -> Vec<(u128, u128)> {
    let mut edges = Vec::new();
    for n in 0..NODES {
        for (to, _) in dag.effects_of(MemoryId(n)) {
            edges.push((n, to.0));
        }
    }
    edges
}

/// Independent acyclicity check via Kahn's algorithm.
fn is_acyclic(edges: &[(u128, u128)]) -> bool {
    let mut nodes: HashSet<u128> = HashSet::new();
    let mut adj: HashMap<u128, Vec<u128>> = HashMap::new();
    let mut indeg: HashMap<u128, usize> = HashMap::new();
    for &(a, b) in edges {
        nodes.insert(a);
        nodes.insert(b);
        adj.entry(a).or_default().push(b);
        *indeg.entry(b).or_default() += 1;
        indeg.entry(a).or_default();
    }
    let mut queue: VecDeque<u128> = nodes
        .iter()
        .copied()
        .filter(|n| indeg.get(n).copied().unwrap_or(0) == 0)
        .collect();
    let mut processed = 0usize;
    while let Some(n) = queue.pop_front() {
        processed += 1;
        if let Some(succ) = adj.get(&n) {
            for &m in succ {
                let d = indeg.get_mut(&m).expect("indegree present");
                *d -= 1;
                if *d == 0 {
                    queue.push_back(m);
                }
            }
        }
    }
    processed == nodes.len()
}

/// Independent forward reachability oracle.
fn reachable(edges: &[(u128, u128)], start: u128, target: u128) -> bool {
    let mut adj: HashMap<u128, Vec<u128>> = HashMap::new();
    for &(a, b) in edges {
        adj.entry(a).or_default().push(b);
    }
    let mut seen = HashSet::new();
    let mut q = VecDeque::new();
    q.push_back(start);
    seen.insert(start);
    while let Some(n) = q.pop_front() {
        if n == target {
            return true;
        }
        if let Some(succ) = adj.get(&n) {
            for &m in succ {
                if seen.insert(m) {
                    q.push_back(m);
                }
            }
        }
    }
    false
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn dag_stays_acyclic_and_rejects_exactly_cycles(
        ops in prop::collection::vec((0u128..NODES, 0u128..NODES), 0..80),
    ) {
        let dag = CausalDag::in_memory();
        for (from, to) in ops {
            let before = collect_edges(&dag);
            // The store rejects iff self-loop OR `from` already reachable from `to`.
            let expect_reject = from == to || reachable(&before, to, from);
            let res = dag.add_edge(MemoryId(from), MemoryId(to), EdgeType::Triggered);
            prop_assert_eq!(res.is_err(), expect_reject, "from={} to={}", from, to);
            // Invariant: acyclic after every operation.
            prop_assert!(is_acyclic(&collect_edges(&dag)));
        }
    }
}
