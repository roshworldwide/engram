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
| P1 | Single-thread episodic write throughput (durable, fsync-batched) | ≥ 100,000 ev/s | WAL-only upper bound **1.56 M/s** (durable, 10k-batch group commit) | ⬜ Phase 2a (store) — WAL signal ✅ |
| P2 | Bulk/batched write throughput | ≥ 300,000 ev/s | WAL-only upper bound **4.46 M/s** (append, no fsync) | ⬜ Phase 2a (store) — WAL signal ✅ |
| P3 | Semantic point-query latency (current time) | < 400 µs p99, < 80 µs p50 | B-tree `get` primitive **~56 ns** hit / **~33 ns** miss on 1M keys | ⬜ Phase 2b (store) — tree primitive ✅ |
| P4 | Time-travel query (≥ 12 mo / ≥ 200 versions) | < 5 ms p99 | — | ⬜ Phase 2b |
| P5 | Provenance-chain trace (depth ≤ 1,000) | < 2 ms p99 | — | ⬜ Phase 2d |
| P6 | Multi-instance write throughput (10 instances, ACC on) | ≥ 250,000 ev/s | — | ⬜ Phase 3b |
| P7 | Confidence-decay evaluation cost (per belief, on read) | < 500 ns, zero background CPU | **eval primitive 1.1–5.3 ns p50** (exp 3.0, pow 5.3, step 1.2, none 1.1) | 🟡 primitive ✅; full read-path Phase 2b |
| P8 | Crash recovery: replay 1,000,000 WAL entries | < 2 s, 100% committed recovered | **128 ms median** (p99 ≈ 136 ms), 100% committed recovered | ✅ Phase 1b (~15× under) |
| Q4 | Time-travel vs. hand-built PostgreSQL bitemporal schema | ≥ 10× faster | — | ⬜ Phase 4c |

## Quality gates

| #  | Gate | Target | Status |
|----|------|--------|--------|
| Q1 | Randomized multi-agent property histories asserting ACC invariants | ≥ 1,000 | ⬜ Phase 3b |
| Q2 | Fuzz iterations, zero crashes (wal_reader / btree_ops / dag_decode / record_codec) | ≥ 10,000,000 each | 🟡 `record_codec` 2.1M + `wal_reader` 390k + `btree_ops` 1.1M local smoke, **0 crashes**; full 10M nightly + `dag_decode` (Phase 2d) pending |
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

## Phase 1b measured (2026-06-26, reference machine)

- **P8 — recover 1,000,000 committed WAL entries** (`cargo bench -p engram-storage --bench recovery`,
  criterion, 10 samples): **123.6 / 128.6 / 135.6 ms** (min/median/max), 100% of committed records recovered.
  Target < 2 s ⇒ ~15× headroom.
- **WAL append throughput** (`cargo bench -p engram-storage --bench wal_append`, 32-byte payload):
  durable 10k-batch group commit **1.48–1.62 Melem/s**; buffered append (no fsync) **4.39–4.51 Melem/s**.
  These are WAL-only upper bounds; the official P1/P2 (with B-tree indexing) are measured in Phase 2a.
- **`wal_reader` fuzz smoke** (`cargo +nightly fuzz run wal_reader -max_total_time=15`):
  **390,211 runs, 0 crashes**.

## Phase 1c measured (2026-06-26, reference machine)

- **1c gate — 1,000,000 random-order inserts + full sorted scan** (`cargo test --release --lib
  one_million_keys -- --ignored`): **~1.1 s** end-to-end; the scan is strictly ascending and complete, and
  old snapshots remain fully readable while the writer advances (MVCC).
- **B-tree `get`** on a 1M-key tree (`cargo bench -p engram-storage --bench btree`): hit **~56 ns**,
  miss **~33 ns**.
- **`btree_ops` fuzz smoke** (differential vs `std::collections::BTreeMap`): **>1.1M runs across two sessions,
  0 crashes**, no behavioral divergence.
- **Adversarial review:** a 5-dimension multi-agent review (each finding independently verified) found the
  B-tree's correctness and MVCC/concurrency dimensions clean, and three real WAL/iterator issues that were
  fixed: parent-directory `fsync` on `create`/`compact`, position-aware recovery (tx_id-reuse safe), and a
  removed per-element `Arc` clone on the scan hot path.
