//! P6: multi-instance write throughput with ACC on (10 concurrent instances).
//! Target ≥ 250,000 ev/s. Run with
//! `cargo bench -p engram-consistency --bench multi_instance`.

use std::sync::Arc;
use std::thread;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use engram_consistency::CausalMemory;
use engram_core::AgentInstanceId;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const INSTANCES: u64 = 10;
const PER_INSTANCE: u64 = 50_000;

fn bench_multi_instance(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_instance");
    group.throughput(Throughput::Elements(INSTANCES * PER_INSTANCE));
    group.sample_size(20);

    group.bench_function("p6_10_instances_write", |b| {
        b.iter_batched(
            CausalMemory::new,
            |mem| {
                let handles: Vec<_> = (1..=INSTANCES)
                    .map(|n| {
                        let mem = Arc::clone(&mem);
                        thread::spawn(move || {
                            let mut session = mem.session(AgentInstanceId(n));
                            for _ in 0..PER_INSTANCE {
                                session.write(b"k".to_vec(), b"v".to_vec());
                            }
                        })
                    })
                    .collect();
                for h in handles {
                    h.join().unwrap();
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_multi_instance);
criterion_main!(benches);
