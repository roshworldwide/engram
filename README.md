# Engram

> *PostgreSQL was built for humans storing records. Engram is built for AI agents that remember.*

**Engram** is a purpose-built storage engine for AI-agent memory, written in Rust from scratch. Unlike a
conventional database — which stores facts that are true until overwritten — Engram is designed for memory
that **fades**, that knows **why** it believes things, that can be **rewound in time**, and that can be
**safely shared between many concurrent agent instances**.

> **Status:** complete and green (`cargo xtask ci`). All nine requirements (R1–R9) are implemented; the
> eight performance gates (P1–P8) and five of six quality gates (Q1, Q3–Q6) are met, with Q2 (full 10M-iter
> fuzz) running nightly. See [`CLAUDE.md`](CLAUDE.md) for the phase-by-phase state,
> [`docs/TRACEABILITY.md`](docs/TRACEABILITY.md) for the requirement → module → proving-test map,
> [`BENCHMARKS.md`](BENCHMARKS.md) for the numbers, [`docs/paper/engram.md`](docs/paper/engram.md) for the
> paper, and [`docs/book/`](docs/book/) for the architecture book.

## Results

On the reference machine (Apple M3, 8 cores, 16 GB) — every number reproducible via `cargo xtask bench`:

| | |
|---|---|
| Durable episodic writes | **330K /s** (P1); **414K /s** bulk (P2) |
| Semantic point read · time-travel | **~318 ns** (P3) · **~162 ns** @ 200 versions (P4) |
| Provenance trace, depth 1000 | **~91 µs** (P5) |
| 10-instance throughput, ACC on | **~2.5M ev/s** (P6) |
| Decay-on-read (zero background CPU) | **~191 ns** (P7) |
| Recover 1,000,000 WAL entries | **~128 ms** (P8) |
| **Time-travel vs hand-built PostgreSQL** | **165.8× faster** (146.7 ns vs 24,322.9 ns) — Q4 |
| Coverage (storage + consistency) | **93.44%** lines — Q3 |

## What makes it different

| Capability | What it means |
|---|---|
| **Four native memory types** | `episodic` (immutable events), `semantic` (mutable beliefs), `procedural` (versioned skills), `working` (bounded auto-consolidating scratchpad) |
| **Bitemporal time-travel** | Every record carries *valid-time* and *transaction-time*; ask "what was believed as-of time T" |
| **Confidence decay** | A first-class, lazily-evaluated storage primitive (exponential / power-law / step) — evaluated only on read, zero background CPU |
| **Causal-provenance DAG** | Every memory links to the memories/events that caused it; provenance chains are queryable |
| **Custom CoW B-tree + WAL** | A copy-on-write B-tree and write-ahead log built from scratch, giving MVCC: lock-free concurrent readers at independent snapshots |
| **Agent Causal Consistency (ACC)** | A multi-agent consistency model guaranteeing read-your-writes, monotonic reads, and causal memory — with **no global synchronization** |

No third-party storage engine is embedded (no RocksDB / LMDB / SQLite / sled / redb). Engram writes its own
bytes — the on-disk format, the WAL, and the B-tree are all hand-built.

## Architecture

```
Client APIs    Rust SDK · Python SDK (PyO3) · REST (axum) · gRPC (tonic)
Query Engine   type dispatch · time-travel resolver · DAG traversal · decay eval · provenance tracer
Consistency    vector-clock engine · session coordinator · read-your-writes / monotonic / causal validators
Storage Engine episodic · semantic · procedural · working · causal-DAG store · write-ahead log
Background     consolidation engine · decay scheduler · working-memory GC · provenance-index builder
```

Requests flow down; results flow up. The consistency layer is bypassable: in single-agent mode there is
**zero** vector-clock overhead.

## Workspace layout

```
crates/
  engram-core         shared types: ids, timestamps, decay functions, errors
  engram-storage      WAL, copy-on-write B-tree (MVCC), the four stores, causal-DAG store
  engram-consistency  vector clocks, session coordinator, ACC enforcement
  engram-query        dispatch, time-travel resolve, decay eval, provenance trace
  engram-server       axum REST + tonic gRPC
  engram-py           PyO3 bindings → the `engram` Python package
  engram-cli          the `engram` CLI: init, put, get, as-of, why, bench, demo
xtask/                dev automation (`cargo xtask ci|bench|demo`)
benches/  fuzz/  examples/  demos/sre/  docs/{book,paper}/
```

## Quickstart

```bash
# Build everything and run the full local gate (fmt-check, clippy -D warnings, build, test, deny*)
cargo xtask ci

# Individual tasks
cargo xtask fmt        # format in place
cargo xtask clippy     # lint with warnings denied
cargo xtask test       # run tests
cargo xtask bench      # criterion benchmarks
cargo xtask demo       # the SRE provenance demo

# The CLI — a memory in five commands
cargo run -p engram-cli -- init /tmp/mem
cargo run -p engram-cli -- put belief /tmp/mem service-x health unhealthy 0.92
cargo run -p engram-cli -- get belief /tmp/mem service-x health   # unhealthy (confidence 0.920)
cargo run -p engram-cli -- demo                                   # why did the agent restart X?
```

Requires a stable Rust toolchain (1.96+). Optional tooling — `cargo-deny`, `cargo-llvm-cov`,
`cargo-fuzz` (nightly), `protoc` (gRPC), `maturin` (Python), `postgresql` (Q4) — is auto-detected:
`cargo xtask ci` reports absent tools as `SKIP`, and CI installs them to enforce the full gate.

## Roadmap

- **Phase 0** — scaffold + CI ✅
- **Phase 1** — records, write-ahead log, copy-on-write B-tree (MVCC) ✅
- **Phase 2** — the four stores + causal-provenance DAG ✅
- **Phase 3** — ACC consistency layer + REST/gRPC + Python SDK ✅
- **Phase 4** — consolidation + SRE case study + evaluation (165.8× vs PostgreSQL) ✅
- **Phase 5** — VLDB-style paper + mdBook + causal-DAG playground + example agents ✅

See [`docs/paper/engram.md`](docs/paper/engram.md) for the research framing (targeting VLDB 2027),
[`docs/book/`](docs/book/) for the architecture book, and [`BENCHMARKS.md`](BENCHMARKS.md) for the
performance targets and the reference machine.

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
