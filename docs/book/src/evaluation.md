# Evaluation & benchmarks

Reference machine: Apple M3 (8 cores), 16 GB, macOS 26.5.1, rustc 1.96.0. The CoW
write path links mimalloc (production allocator). Every number is reproducible via
`cargo xtask bench` and `cargo run -p compare-postgres`; the canonical record with
p50/p99, sample sizes, and warm-up is `BENCHMARKS.md`.

## Performance targets (P1–P8)

| Metric | Target | Engram |
|---|---|---|
| P1 durable episodic writes (fsync-batched) | ≥ 100K /s | **330K /s** |
| P2 bulk writes | ≥ 300K /s | **414K /s** |
| P3 semantic point read | < 400 µs p99 | **~318 ns** |
| P4 time-travel @ 200 versions | < 5 ms p99 | **~162 ns** |
| P5 provenance trace, depth 1000 | < 2 ms p99 | **~91 µs** |
| P6 10-instance throughput, ACC on | ≥ 250K /s | **~2.5M /s** |
| P7 decay-on-read | < 500 ns | **~191 ns** (eval ~3 ns) |
| P8 recover 1,000,000 WAL entries | < 2 s | **~128 ms** |

## Quality gates (Q1–Q6)

| Gate | Target | Result |
|---|---|---|
| Q1 randomized multi-agent ACC histories | ≥ 1,000 | **1,200**, all pass |
| Q2 fuzz iterations, zero crashes (×4 targets) | ≥ 10M each | 🟡 smoke-clean; full 10M nightly |
| Q3 line coverage (storage + consistency) | ≥ 90% | **93.44%** |
| Q4 time-travel vs PostgreSQL | ≥ 10× | **165.8×** |
| Q5 ACC metadata | `O(\|agents\|)`, ≤ 16 B/slot | proven |
| Q6 fmt/clippy(`-D`)/test/deny green every commit | always | green |

## The Q4 comparison

We load the same 200-version belief into Engram and into a hand-built PostgreSQL
bitemporal schema — `beliefs(subject, predicate, object, tx_from, tx_until)` with a
`(subject, predicate, tx_from)` index — and run the identical as-of query:

| | latency / query | mode |
|---|---|---|
| Engram `get_at_tx` | **146.7 ns** | in-process `floor` |
| PostgreSQL bitemporal `SELECT` | **24,322.9 ns** | indexed, client/server |

**165.8×.** The gap is the honest deployment reality: an agent's memory is
in-process; a Postgres-backed memory pays a socket round-trip, parse, and plan on
every query. Reproduce with `cargo run -p compare-postgres` against a live Postgres
(`benches/compare_postgres/`).
