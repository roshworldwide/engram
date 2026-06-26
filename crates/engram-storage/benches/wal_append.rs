//! Early P1/P2 signal: WAL append throughput. This is the durable-write *upper
//! bound* — the episodic store (Phase 2a) adds B-tree indexing on top, where the
//! official P1/P2 numbers are measured. Run with
//! `cargo bench -p engram-storage --bench wal_append`.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use engram_core::RecordKind;
use engram_storage::{Wal, WalOp};
use tempfile::tempdir;

const BATCH: u64 = 10_000;

fn bench_append(c: &mut Criterion) {
    let payload = [0x5Au8; 32];
    let mut group = c.benchmark_group("wal_append");
    group.throughput(Throughput::Elements(BATCH));

    // Durable group commit: BATCH appends followed by a single fsync (P2-style).
    group.bench_function("batched_group_commit_10k", |b| {
        b.iter_batched(
            || {
                let dir = tempdir().unwrap();
                let wal = Wal::create(dir.path().join("a.wal")).unwrap();
                (dir, wal)
            },
            |(dir, mut wal)| {
                for _ in 0..BATCH {
                    wal.append(1, WalOp::Put(RecordKind::Episodic), &payload)
                        .unwrap();
                }
                wal.commit(1).unwrap();
                drop(wal);
                drop(dir);
            },
            BatchSize::SmallInput,
        );
    });

    // Buffered append without fsync — encoding + buffered-IO speed only.
    group.bench_function("append_no_fsync_10k", |b| {
        b.iter_batched(
            || {
                let dir = tempdir().unwrap();
                let wal = Wal::create(dir.path().join("b.wal")).unwrap();
                (dir, wal)
            },
            |(dir, mut wal)| {
                for _ in 0..BATCH {
                    wal.append(1, WalOp::Put(RecordKind::Episodic), &payload)
                        .unwrap();
                }
                wal.flush().unwrap();
                drop(wal);
                drop(dir);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_append);
criterion_main!(benches);
