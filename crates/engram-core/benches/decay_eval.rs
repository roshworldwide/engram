//! Early P7 signal: per-belief confidence-decay evaluation must be < 500 ns and
//! allocation-free. Run with `cargo bench -p engram-core` (or `cargo xtask bench`).

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use engram_core::{DecayFunction, Timestamp};

const DAY_NS: i64 = 86_400 * 1_000_000_000;

fn bench_decay(c: &mut Criterion) {
    let mut group = c.benchmark_group("decay_eval");

    let exponential = DecayFunction::Exponential { lambda: 1e-6 };
    group.bench_function("exponential", |b| {
        b.iter(|| black_box(exponential).eval(black_box(0.9), black_box(30 * DAY_NS)))
    });

    let power_law = DecayFunction::PowerLaw { beta: 0.5 };
    group.bench_function("power_law", |b| {
        b.iter(|| black_box(power_law).eval(black_box(0.9), black_box(30 * DAY_NS)))
    });

    let step = DecayFunction::Step {
        drop_at: Timestamp(10 * DAY_NS),
        c_low: 0.1,
    };
    group.bench_function("step", |b| {
        b.iter(|| black_box(step).eval(black_box(0.9), black_box(30 * DAY_NS)))
    });

    group.bench_function("none", |b| {
        b.iter(|| black_box(DecayFunction::None).eval(black_box(0.9), black_box(30 * DAY_NS)))
    });

    group.finish();
}

criterion_group!(benches, bench_decay);
criterion_main!(benches);
