//! Early signal for P3 (point-query latency) and write cost: CoW B-tree `get`
//! and `insert`. The official P3 (semantic point query) is measured in Phase 2b;
//! this is the underlying tree primitive. Run with
//! `cargo bench -p engram-storage --bench btree`.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use engram_storage::CowBTree;

const N: u64 = 1_000_000;

fn fill(n: u64) -> CowBTree<u64, u64> {
    let t = CowBTree::new();
    let mut x = 1u64;
    for _ in 0..n {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let k = x >> 16;
        t.insert(k, k);
    }
    t
}

fn bench_btree(c: &mut Criterion) {
    let tree = fill(N);
    // Collect some present keys to probe.
    let probes: Vec<u64> = tree
        .iter()
        .step_by(997)
        .map(|(k, _)| k)
        .take(1024)
        .collect();

    let mut group = c.benchmark_group("btree");

    let mut i = 0usize;
    group.bench_function("get_hit_1m", |b| {
        b.iter(|| {
            i = (i + 1) % probes.len();
            black_box(tree.get(black_box(&probes[i])))
        });
    });

    group.bench_function("get_miss_1m", |b| {
        b.iter(|| black_box(tree.get(black_box(&1u64)))); // odd key, never inserted (k = x>>16 even-ish)
    });

    // Insert cost into an already-large tree (copy-on-write path clone).
    group.throughput(Throughput::Elements(1));
    group.bench_function("insert_into_1m", |b| {
        b.iter_batched(
            || tree.snapshot(),
            |_snap| {
                // Insert a fresh key; measures path-clone + atomic swap.
                tree.insert(black_box(u64::MAX - (i as u64)), 0);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_btree);
criterion_main!(benches);
