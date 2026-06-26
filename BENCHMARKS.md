# Benchmarks

All performance targets are **hard CI gates**, measured with `criterion` and reproducible via
`cargo xtask bench` on the reference machine below. A target counts as "met" only when a committed,
reproducible benchmark proves it. Numbers are never hand-waved; gaps are logged here with the measured value,
the bottleneck, and the optimization plan.

## Reference machine

| Field | Value |
|---|---|
| Model | Apple Mac15,13 (MacBook) |
| CPU | Apple M3 — 8 cores (8 performance/efficiency, 8 logical) |
| Memory | 16 GB unified |
| OS | macOS 26.5.1 (build 25F80) |
| Toolchain | rustc 1.96.0 (stable, aarch64-apple-darwin) |
| Profile | `release` / `bench`: opt-level 3, thin LTO, codegen-units 1 |

> Note: the reference machine is a laptop. Throughput targets are validated single-process; when results are
> reproduced on server-class hardware that will be recorded as a separate column.

## Methodology

- `criterion` with explicit warm-up; report p50 and p99, sample size, and iteration count.
- Durable-write throughput uses real `fsync` with group commit — fsync is never silently disabled to hit a
  number (anti-cheating guardrail). When a number is measured without durability, it is labeled.
- Time-travel and decay tests drive time through the injectable `Clock` so historical depth is real data, not
  faked timestamps.

## Targets vs. measured

Status legend: ⬜ pending (target phase) · 🟡 measured, below target (gap logged) · ✅ met.

| #  | Metric | Target | Measured | Status |
|----|--------|--------|----------|--------|
| P1 | Single-thread episodic write throughput (durable, fsync-batched) | ≥ 100,000 ev/s | — | ⬜ Phase 2a |
| P2 | Bulk/batched write throughput | ≥ 300,000 ev/s | — | ⬜ Phase 2a |
| P3 | Semantic point-query latency (current time) | < 400 µs p99, < 80 µs p50 | — | ⬜ Phase 2b |
| P4 | Time-travel query (≥ 12 mo / ≥ 200 versions) | < 5 ms p99 | — | ⬜ Phase 2b |
| P5 | Provenance-chain trace (depth ≤ 1,000) | < 2 ms p99 | — | ⬜ Phase 2d |
| P6 | Multi-instance write throughput (10 instances, ACC on) | ≥ 250,000 ev/s | — | ⬜ Phase 3b |
| P7 | Confidence-decay evaluation cost (per belief, on read) | < 500 ns, zero background CPU | **eval primitive 1.1–5.3 ns p50** (exp 3.0, pow 5.3, step 1.2, none 1.1) | 🟡 primitive ✅; full read-path Phase 2b |
| P8 | Crash recovery: replay 1,000,000 WAL entries | < 2 s, 100% committed recovered | — | ⬜ Phase 1b |
| Q4 | Time-travel vs. hand-built PostgreSQL bitemporal schema | ≥ 10× faster | — | ⬜ Phase 4c |

## Quality gates

| #  | Gate | Target | Status |
|----|------|--------|--------|
| Q1 | Randomized multi-agent property histories asserting ACC invariants | ≥ 1,000 | ⬜ Phase 3b |
| Q2 | Fuzz iterations, zero crashes (wal_reader / btree_ops / dag_decode / record_codec) | ≥ 10,000,000 each | 🟡 `record_codec`: 2.1M-run local smoke, 0 crashes (~100k exec/s); full 10M nightly + 3 remaining targets pending |
| Q3 | Line coverage on `engram-storage` + `engram-consistency` | ≥ 90% | ⬜ Phase 4 |
| Q5 | ACC metadata overhead per op | O(\|agents\|), ≤ 16 bytes/agent-slot, proven | ⬜ Phase 3b |
| Q6 | Clippy / rustfmt / `cargo test` / `cargo deny` | green every commit, clippy `-D warnings` | ✅ (all four green locally + in CI) |

## Tooling status on the reference machine

- `cargo-deny` — **installed (0.19.9); passes locally** (advisories/bans/licenses/sources ok) and in CI.
- `cargo-fuzz` — **installed; nightly toolchain installed**; `record_codec` builds and smoke-runs locally.
- `cargo-llvm-cov` — not yet installed locally; CI installs via `taiki-e/install-action`.

## Phase 1a measured (2026-06-26, reference machine)

- **Decay-eval primitive** (`cargo bench -p engram-core --bench decay_eval`, criterion, 100 samples):
  exponential **2.97 ns**, power-law **5.27 ns**, step **1.23 ns**, none **1.13 ns** — all ≈ two orders of
  magnitude under the < 500 ns P7 target. (Full per-belief P7 including the read path lands in Phase 2b.)
- **`record_codec` fuzz smoke** (`cargo +nightly fuzz run record_codec -max_total_time=20`):
  **2,104,093 runs, 0 crashes**, ~100k exec/s, peak RSS 430 MB.
