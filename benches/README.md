# Benchmarks

`criterion` benchmarks, one per performance metric in [`../BENCHMARKS.md`](../BENCHMARKS.md). They are wired
into the `engram-storage` / `engram-query` crates as `[[bench]]` targets and run via `cargo xtask bench`.

Planned targets (land in Phase 2 and Phase 4):

- `write_throughput.rs` — P1 (≥ 100k ev/s durable), P2 (≥ 300k ev/s bulk)
- `point_query.rs` — P3 (< 400 µs p99 semantic read)
- `time_travel.rs` — P4 (< 5 ms p99 @ 12 months)
- `provenance.rs` — P5 (< 2 ms p99 trace, depth ≤ 1000)
- `decay_eval.rs` — P7 (< 500 ns/belief)
- `recovery.rs` — P8 (replay 1M WAL entries < 2 s)
- `multi_instance.rs` — P6 (≥ 250k ev/s, 10 instances, ACC on)
- `compare_postgres/` — Q4 (≥ 10× vs a hand-built PostgreSQL bitemporal schema)
