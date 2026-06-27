# Traceability matrix

Every requirement and metric maps to (a) the implementing module and (b) at least one automated test or
benchmark that proves it. A row is **done** only when its proof is green in CI. Keep this current — it is the
contract between the spec and the code.

Status: ⬜ pending · 🟡 in progress · ✅ proven (green test/bench in CI).

## Requirements (R1–R9)

| Req | Capability | Module / file (planned) | Proving test or bench | Status |
|-----|------------|-------------------------|-----------------------|--------|
| R1 | From-scratch Rust storage engine (no embedded engine) | `crates/engram-storage/*` | `deny.toml` storage-engine ban + absence of such deps | 🟡 banned in CI; engine lands Phase 1 |
| R2 | Record types + MessagePack codec | `engram-core/src/{records,codec,ids}.rs` | `tests/codec_roundtrip.rs` (proptest round-trips; arbitrary bytes never panic) | ✅ Phase 1a |
| R2 | Episodic memory type (store) | `engram-storage/src/stores/episodic.rs` | `stores::episodic::tests` + `tests/episodic.rs` (10k) + `benches/write_throughput.rs` | ✅ Phase 2a |
| R2 | Semantic memory type | `engram-storage/src/stores/semantic.rs` | `stores::semantic::tests` (10-version history, time-travel, decay, retract, recovery) | ✅ Phase 2b |
| R2 | Procedural memory type | `engram-storage/src/stores/procedural.rs` | `stores::procedural::tests` (supersedes chain) | ✅ Phase 2c |
| R2 | Working memory type | `engram-storage/src/stores/working.rs` | `stores::working::tests` (bounded FIFO + consolidation hook) | ✅ Phase 2e |
| R3 | Bitemporal time-travel | `engram-storage/src/stores/semantic.rs` (`get_at_tx`) | `semantic::tests` time-travel + `benches/semantic_read.rs` (P4) | ✅ Phase 2b (query layer wraps in Phase 4) |
| R4 | Confidence decay (lazy) | `engram-core/src/decay.rs` + `semantic.rs` (on read) | `decay::tests` + `semantic::tests::decay_is_monotonic_on_read` + benches | ✅ Phase 1a/2b (eval ~3 ns; lazy on read) |
| R5 | Causal-provenance DAG | `engram-storage/src/stores/causal.rs` | `causal::tests` + `tests/dag.rs` (acyclic proptest) + `dag_ops` fuzz + `benches/provenance.rs` (P5) | ✅ Phase 2d |
| R6 | Write-Ahead Log | `engram-storage/src/wal.rs` | `wal::tests` + `tests/wal_recovery.rs` (proptest) + recovery bench (P8) | ✅ Phase 1b |
| R6 | CoW B-tree → MVCC | `engram-storage/src/btree.rs` | `btree::tests` (MVCC isolation, concurrent readers) + `tests/btree_oracle.rs` + `btree_ops` fuzz + 1M gate | ✅ Phase 1c |
| R7 | Vector-clock engine | `engram-consistency/src/vector_clock.rs` | `vector_clock::tests` + `tests/vector_clock_props.rs` (partial-order + LUB) | ✅ Phase 3a |
| R7 | ACC enforcement (RYW/monotonic/causal) | `engram-consistency/src/acc.rs` | `acc::tests` + `tests/acc_histories.rs` (Q1, 1200 histories) + `benches/multi_instance.rs` (P6) | ✅ Phase 3b |
| R8 | Python SDK via PyO3 | `crates/engram-py/*` (pyo3 0.29, maturin) | `tests/test_engram.py` (SRE flow via `import engram`) + CI python job | ✅ Phase 3d |
| §3 | Client APIs (engine + REST + gRPC) | `engram-query::Engine`, `engram-server` (axum/tonic) | `engram-query::engine::tests` + `tests/rest.rs` + `grpc::tests` | ✅ Phase 3c |
| R9 | Research-grade rigor | `docs/paper/engram.md`, `docs/book/`, `compare_postgres`, `demos/sre`, `examples/`, `playground/` | paper (formal ACC + efficiency proof) + reproducible eval (Q4) + SRE case study + mdBook + 5 example agents | ✅ Phase 5 |
| 4a | Consolidation (repeated evidence → belief) | `engram-query::consolidation` + `engram-server::consolidation` | `consolidation::tests` + `engine::tests` + background-task test | ✅ Phase 4a |
| 5b | `engram` CLI over the engine | `crates/engram-cli` | `engram-cli::tests` (exit-code contract + `demo` end-to-end) | ✅ Phase 5b |
| 5d | Python SDK example agents | `examples/*.py` | each script self-asserts; run green against the `engram` wheel | ✅ Phase 5d |

## Performance metrics (P1–P8)

| # | Metric | Proving bench | Status |
|---|--------|---------------|--------|
| P1 | ≥ 100k ev/s episodic write (durable) | `benches/write_throughput.rs` | ✅ Phase 2a (330k @ group-commit 4096) |
| P2 | ≥ 300k ev/s bulk write | `benches/write_throughput.rs` | ✅ Phase 2a (414k bulk, mimalloc; 295k system) |
| P3 | < 400 µs p99 semantic point read | `engram-storage/benches/semantic_read.rs` | ✅ Phase 2b (~318 ns) |
| P4 | < 5 ms p99 time-travel @ 12 mo | `engram-storage/benches/semantic_read.rs` | ✅ Phase 2b (~162 ns @ 200 versions) |
| P5 | < 2 ms p99 provenance trace (≤ 1000) | `engram-storage/benches/provenance.rs` | ✅ Phase 2d (~91 µs @ depth 1000) |
| P6 | ≥ 250k ev/s, 10 instances, ACC on | `engram-consistency/benches/multi_instance.rs` | ✅ Phase 3b (~2.5 M ev/s) |
| P7 | < 500 ns/belief decay, 0 background CPU | `decay_eval.rs` (primitive) + `semantic_read.rs` (read-path) | ✅ Phase 2b (~191 ns full read; eval ~3 ns; 0 background) |
| P8 | recover 1M WAL entries < 2 s | `crates/engram-storage/benches/recovery.rs` | ✅ Phase 1b (128 ms median, 100% recovered) |

## Quality metrics (Q1–Q6)

| # | Metric | Proving artifact | Status |
|---|--------|------------------|--------|
| Q1 | ≥ 1,000 randomized multi-agent histories, ACC holds | `engram-consistency/tests/acc_histories.rs` | ✅ Phase 3b (1,200 cases) |
| Q2 | ≥ 10M fuzz iters, zero crashes (×4 targets) | `fuzz/fuzz_targets/*` | ✅ all 4 targets at full **10M, 0 crashes** (`record_codec`/`wal_reader`/`btree_ops`/`dag_ops`) |
| Q3 | ≥ 90% coverage (storage + consistency) | `cargo llvm-cov` | ✅ Phase 4c (93.44% lines) |
| Q4 | ≥ 10× faster time-travel vs PostgreSQL | `benches/compare_postgres/` | ✅ Phase 4c (165.8×) |
| Q5 | ACC overhead O(\|agents\|), ≤ 16 B/slot, proven | `vector_clock::to_bytes` + `WriteOp::metadata_bytes` | ✅ Phase 3b (one 16 B/slot clock per op) |
| Q6 | fmt/clippy(-D)/test/deny green every commit | `cargo xtask ci` + CI | ✅ (all green locally + in CI) |
