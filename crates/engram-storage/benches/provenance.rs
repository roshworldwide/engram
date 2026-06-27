//! P5: provenance-chain trace (depth ≤ 1000 nodes) `< 2 ms p99`. Run with
//! `cargo bench -p engram-storage --bench provenance`.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use engram_core::{EdgeType, MemoryId};
use engram_storage::CausalDag;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const DEPTH: u128 = 1000;

fn bench_provenance(c: &mut Criterion) {
    // A linear causal chain 0 -> 1 -> ... -> DEPTH.
    let dag = CausalDag::in_memory();
    for i in 0..DEPTH {
        dag.add_edge(MemoryId(i), MemoryId(i + 1), EdgeType::Triggered)
            .unwrap();
    }

    let mut group = c.benchmark_group("provenance");
    group.bench_function("p5_provenance_chain_depth_1000", |b| {
        b.iter(|| black_box(dag.find_provenance_chain(black_box(MemoryId(DEPTH)))));
    });
    group.bench_function("find_path_depth_1000", |b| {
        b.iter(|| black_box(dag.find_path(black_box(MemoryId(0)), black_box(MemoryId(DEPTH)))));
    });
    group.finish();
}

criterion_group!(benches, bench_provenance);
criterion_main!(benches);
