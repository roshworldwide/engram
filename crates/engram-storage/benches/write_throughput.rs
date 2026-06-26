//! P1 / P2: episodic write throughput on the full stack (WAL durability + the
//! three CoW B-tree indexes per event). Run with
//! `cargo bench -p engram-storage --bench write_throughput`.
//!
//! - **P1** — single-thread durable, fsync-batched (group commit every 256).
//!   Target ≥ 100,000 ev/s.
//! - **P2** — bulk/batched (one commit at the end). Target ≥ 300,000 ev/s.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use engram_core::{AgentId, EpisodicRecord, EventType, MemoryId, SessionId, Timestamp};
use engram_storage::EpisodicStore;
use tempfile::tempdir;

// The CoW B-tree write path is allocation-bound; production binaries link a fast
// allocator. Measure the engine the way it would actually run.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const N: u64 = 20_000;

fn make_events(n: u64) -> Vec<EpisodicRecord> {
    (0..n)
        .map(|i| EpisodicRecord {
            id: MemoryId::from_parts(1_700_000_000_000, u128::from(i)),
            agent_id: AgentId(1),
            session_id: SessionId(i % 16),
            valid_time: Timestamp::from_millis(1_700_000_000_000 + i as i64),
            tx_time: Timestamp(0),
            event_type: EventType::Observation,
            payload: vec![0u8; 32],
            cause_ids: vec![],
        })
        .collect()
}

fn bench_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("episodic_write");
    group.throughput(Throughput::Elements(N));
    group.sample_size(20);

    // P1: durable, fsync-batched at a few group-commit sizes (durability/throughput knob).
    for batch in [256usize, 1024, 4096] {
        group.bench_function(format!("p1_durable_fsync_batched_{batch}"), |b| {
            b.iter_batched(
                || (tempdir().unwrap(), make_events(N)),
                |(dir, events)| {
                    let store = EpisodicStore::create(dir.path().join("w.wal")).unwrap();
                    for (i, e) in events.into_iter().enumerate() {
                        store.append(e).unwrap();
                        if i % batch == batch - 1 {
                            store.commit().unwrap();
                        }
                    }
                    store.commit().unwrap();
                    drop(store);
                    drop(dir);
                },
                BatchSize::SmallInput,
            );
        });
    }

    // Index-only (flush, no fsync) — isolates encode + WAL-append + 3 CoW inserts
    // from fsync latency.
    group.bench_function("index_only_no_fsync", |b| {
        b.iter_batched(
            || (tempdir().unwrap(), make_events(N)),
            |(dir, events)| {
                let store = EpisodicStore::create(dir.path().join("w.wal")).unwrap();
                for e in events {
                    store.append(e).unwrap();
                }
                drop(store);
                drop(dir);
            },
            BatchSize::SmallInput,
        );
    });

    // P2: bulk — append all, one commit (one fsync).
    group.bench_function("p2_bulk_single_commit", |b| {
        b.iter_batched(
            || (tempdir().unwrap(), make_events(N)),
            |(dir, events)| {
                let store = EpisodicStore::create(dir.path().join("w.wal")).unwrap();
                for e in events {
                    store.append(e).unwrap();
                }
                store.commit().unwrap();
                drop(store);
                drop(dir);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_writes);
criterion_main!(benches);
