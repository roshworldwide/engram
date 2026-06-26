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
| R2 | Episodic memory type (store) | `engram-storage/src/stores/episodic.rs` | `stores::episodic::tests` + bench `write_throughput` (P1) | ⬜ Phase 2a |
| R2 | Semantic memory type | `engram-storage/src/stores/semantic.rs` | `stores::semantic::tests` (versioning) | ⬜ Phase 2b |
| R2 | Procedural memory type | `engram-storage/src/stores/procedural.rs` | `stores::procedural::tests` (supersedes chain) | ⬜ Phase 2c |
| R2 | Working memory type | `engram-storage/src/stores/working.rs` | `stores::working::tests` (bounded FIFO eviction) | ⬜ Phase 2e |
| R3 | Bitemporal time-travel | `engram-query/src/time_travel.rs` | `time_travel::tests::query_at_time` + bench (P4) | ⬜ Phase 2b |
| R4 | Confidence decay (lazy) | `engram-core/src/decay.rs` | `decay::tests` (monotonicity/bounds) + `decay_eval` bench | 🟡 eval primitive ✅ (1a, 1.1–5.3 ns); read-path P7 Phase 2b |
| R5 | Causal-provenance DAG | `engram-storage/src/dag.rs` (`CausalEdge` type in core) | `dag::tests` (chains + cycle reject) + bench (P5) | ⬜ Phase 2d (edge type ✅ in 1a) |
| R6 | Write-Ahead Log | `engram-storage/src/wal.rs` | `wal::tests` + `tests/wal_recovery.rs` (proptest) + recovery bench (P8) | ✅ Phase 1b |
| R6 | CoW B-tree → MVCC | `engram-storage/src/btree.rs` | `btree::tests` (MVCC isolation, concurrent readers) + `tests/btree_oracle.rs` + `btree_ops` fuzz + 1M gate | ✅ Phase 1c |
| R7 | Agent Causal Consistency | `engram-consistency/src/*` | ACC property suite (Q1) | ⬜ Phase 3 |
| R8 | Python SDK via PyO3 | `crates/engram-py/*` | end-to-end Python test | ⬜ Phase 3d |
| R9 | Research-grade rigor | `docs/paper/*`, benches, proptests | paper + reproducible eval | ⬜ Phase 5 |

## Performance metrics (P1–P8)

| # | Metric | Proving bench | Status |
|---|--------|---------------|--------|
| P1 | ≥ 100k ev/s episodic write (durable) | `benches/write_throughput.rs` | ⬜ Phase 2a |
| P2 | ≥ 300k ev/s bulk write | `benches/write_throughput.rs` | ⬜ Phase 2a |
| P3 | < 400 µs p99 semantic point read | `benches/point_query.rs` (+ `engram-storage/benches/btree.rs`) | ⬜ Phase 2b — tree `get` primitive ~56 ns ✅ |
| P4 | < 5 ms p99 time-travel @ 12 mo | `benches/time_travel.rs` | ⬜ Phase 2b |
| P5 | < 2 ms p99 provenance trace (≤ 1000) | `benches/provenance.rs` | ⬜ Phase 2d |
| P6 | ≥ 250k ev/s, 10 instances, ACC on | `benches/multi_instance.rs` | ⬜ Phase 3b |
| P7 | < 500 ns/belief decay, 0 background CPU | `crates/engram-core/benches/decay_eval.rs` | 🟡 primitive 1.1–5.3 ns ✅; read-path Phase 2b |
| P8 | recover 1M WAL entries < 2 s | `crates/engram-storage/benches/recovery.rs` | ✅ Phase 1b (128 ms median, 100% recovered) |

## Quality metrics (Q1–Q6)

| # | Metric | Proving artifact | Status |
|---|--------|------------------|--------|
| Q1 | ≥ 1,000 randomized multi-agent histories, ACC holds | `engram-consistency` proptest suite | ⬜ Phase 3b |
| Q2 | ≥ 10M fuzz iters, zero crashes (×4 targets) | `fuzz/fuzz_targets/*` (nightly) | 🟡 `record_codec` 2.1M + `wal_reader` 390k + `btree_ops` 1.1M smoke, 0 crashes; full 10M + `dag_decode` (2d) pending |
| Q3 | ≥ 90% coverage (storage + consistency) | `cargo llvm-cov` in CI | ⬜ Phase 4 |
| Q4 | ≥ 10× faster time-travel vs PostgreSQL | `benches/compare_postgres/` | ⬜ Phase 4c |
| Q5 | ACC overhead O(\|agents\|), ≤ 16 B/slot, proven | `engram-consistency` size test + bench | ⬜ Phase 3b |
| Q6 | fmt/clippy(-D)/test/deny green every commit | `cargo xtask ci` + CI | ✅ (all green locally + in CI) |
