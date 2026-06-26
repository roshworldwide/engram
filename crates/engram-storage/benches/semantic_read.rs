//! P3 / P4 / P7 for the semantic store, run with
//! `cargo bench -p engram-storage --bench semantic_read`.
//!
//! - **P3** — current-time point read: `< 400 µs p99`, `< 80 µs p50`.
//! - **P4** — time-travel into a 200-version belief: `< 5 ms p99`.
//! - **P7** — confidence decay on read: `< 500 ns/belief` (the read path applies
//!   `DecayFunction::eval`, already ~3 ns as a primitive; this confirms it stays
//!   cheap inside a real point read).

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use engram_core::{AgentId, DecayFunction, Timestamp};
use engram_storage::{BeliefInput, SemanticStore};
use tempfile::tempdir;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const N: usize = 50_000;

fn belief(subject: String, predicate: String, decay: DecayFunction) -> BeliefInput {
    BeliefInput {
        agent_id: AgentId(1),
        subject,
        predicate,
        object: vec![0u8; 16],
        valid_from: Timestamp::from_millis(0),
        confidence_init: 0.9,
        decay_fn: decay,
        provenance_ids: vec![],
    }
}

fn bench_reads(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let store = SemanticStore::create(dir.path().join("s.wal")).unwrap();

    // N single-version beliefs (subject "user", distinct predicates), exp decay.
    let decay = DecayFunction::Exponential { lambda: 1e-9 };
    for i in 0..N {
        store
            .upsert_belief(belief("user".into(), format!("pred{i}"), decay))
            .unwrap();
    }
    // One "hot" belief edited 200 times -> a 200-version time-travel chain.
    for _ in 0..200 {
        store
            .upsert_belief(belief("hot".into(), "state".into(), decay))
            .unwrap();
    }
    store.commit().unwrap();

    // A deep historical transaction-time point of the hot belief.
    let history = store.history("hot", "state");
    let deep_tx = history[2].tx_from;

    let probes: Vec<String> = (0..1024).map(|i| format!("pred{}", (i * 37) % N)).collect();
    let mut idx = 0usize;

    let mut group = c.benchmark_group("semantic_read");

    group.bench_function("p3_current_point_read", |b| {
        b.iter(|| {
            idx = (idx + 1) % probes.len();
            black_box(store.current(black_box("user"), black_box(&probes[idx])))
        });
    });

    group.bench_function("p4_time_travel_200_versions", |b| {
        b.iter(|| {
            black_box(store.get_at_tx(black_box("hot"), black_box("state"), black_box(deep_tx)))
        });
    });

    // Point read that exercises the decay-on-read path (current = floor + decay).
    group.bench_function("p7_decay_on_read", |b| {
        b.iter(|| black_box(store.current(black_box("hot"), black_box("state"))));
    });

    group.finish();
    drop(dir);
}

criterion_group!(benches, bench_reads);
criterion_main!(benches);
