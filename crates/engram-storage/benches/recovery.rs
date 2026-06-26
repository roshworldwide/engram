//! P8: replay 1,000,000 WAL entries in < 2 s with 100% of committed records
//! recovered. Run with `cargo bench -p engram-storage --bench recovery`.

use criterion::{criterion_group, criterion_main, Criterion};
use engram_core::RecordKind;
use engram_storage::{Wal, WalOp};
use tempfile::tempdir;

const N: u64 = 1_000_000;

fn bench_recovery(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("recovery_bench.wal");

    // Prepare a 1M-entry, single-transaction WAL once (outside the measured loop).
    {
        let payload = [0xABu8; 24];
        let mut wal = Wal::create(&path).unwrap();
        for _ in 0..N {
            wal.append(1, WalOp::Put(RecordKind::Episodic), &payload)
                .unwrap();
        }
        wal.commit(1).unwrap();
    }

    let mut group = c.benchmark_group("recovery");
    group.sample_size(10);
    group.bench_function("recover_1m_committed", |b| {
        b.iter(|| {
            let recovered = Wal::recover(&path).unwrap();
            assert_eq!(recovered.entries.len(), N as usize);
        });
    });
    group.finish();

    drop(dir);
}

criterion_group!(benches, bench_recovery);
criterion_main!(benches);
