# Changelog

All notable changes to Engram are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses 0.0.0 until the first tagged
milestone.

## [Unreleased]

### Phase 4c — Evaluation (Q4 + Q3)

- `benches/compare_postgres/` — a standalone harness timing the **same** bitemporal as-of query (a belief with
  200 versions) in Engram vs a hand-built PostgreSQL bitemporal schema (indexed). **Q4 met: 165.8× faster**
  (146.7 ns in-process `floor` vs 24,322.9 ns over Postgres's client/server protocol). Excluded from the
  workspace; run against a live Postgres.
- **Q3 met:** `cargo llvm-cov` reports **93.44% line coverage** (94.27% region) across `engram-storage` +
  `engram-consistency`.

### Phase 4b — SRE killer demo

- `demos/sre` (`sre-demo`, `cargo xtask demo`) reproduces "why did the agent restart service X?" → traces the
  causal-provenance DAG `action → belief(unhealthy) → observation(Y > Z)`. `Engine::get_belief` resolves
  semantic provenance nodes.

### Phase 4a — Consolidation engine

- `engram-query::consolidation` — conservative rule-based promotion of beliefs from repeated episodic evidence
  (pluggable `SignalExtractor`; dominant-object-per-`(subject,predicate)`; confidence `1−(1−w)^n`; provenance =
  the supporting event ids). `Engine::consolidate_session` upserts each belief causally linked to its sources;
  `engram-server::consolidation::spawn_consolidation` runs it as a periodic tokio task. Test: 20 "be concise"
  events → 1 belief (conf ≈0.88, 20-id provenance traceable through the DAG).

### Phase 3d — Python SDK (PyO3)

- `engram-py` exposes the engine to Python as the `engram` module (R8; pyo3 0.29, abi3-py39): `Engram(path)`
  with `record_event`, `upsert_belief`, `current_belief`, `belief_at` (time-travel), `provenance`. Cause and
  provenance ids are linked into the DAG, so `mem.provenance(action)` traces back to the root-cause event.
- PyO3 is behind an opt-in `python` feature (default-off), so the Rust workspace gate builds an empty cdylib
  with no Python toolchain; maturin builds the wheel with the feature on. Verified: `maturin develop` builds
  an abi3 wheel and `tests/test_engram.py` runs the SRE flow through `import engram`.

### Phase 3c — REST + gRPC surfaces

- `engram-query::Engine` — bundles the four stores + causal DAG over one data directory and links writes into
  provenance; the integration point for the surfaces and the SDK.
- `engram-server` REST (axum): `POST/GET /memories/episodic`, `POST/GET /memories/semantic` (current or
  `at_ms` time-travel), `GET /memories/provenance/:id`, `/health`. In-process `tower::oneshot` test of the
  full SRE flow.
- gRPC (tonic) mirrors the same ops, generated from `proto/engram.proto`. Behind an opt-in `grpc` feature so
  the default build needs no `protoc`; CI/xtask gain dedicated grpc + python steps (skipped when the tool is
  absent), and `--all-features` is dropped from the default clippy to stay protoc-free.

### Phase 3b — Agent Causal Consistency enforcement

- `engram-consistency::acc` — the ACC engine (R7). A `CausalMemory` is a shared append-only log of writes;
  each instance's `Session` delivers writes in **causal order** via vector clocks (`can_deliver` = FIFO from
  the writer + all cross-instance dependencies present), with **no consensus, total order, or coordinator** on
  the consistency path. A write's dependencies are *derived* from its clock (clock minus the writer's own
  tick), so each op carries exactly one clock (Q5: 16 B/slot).
- API: `session`, `write`, `refresh`, `read` (causal frontier — all concurrent siblings), `read_latest`
  (single deterministic, **monotonic** value), `clock`, `is_delivered`. The storage engine uses no vector
  clocks, so single-agent mode has zero ACC overhead.
- **Q1 met:** 1,200 randomized multi-agent histories assert prefix-closure / read-your-writes / monotonic
  sessions / convergence against an independent clock oracle. **P6 met:** ~2.5 M ev/s across 10 instances
  (≥ 250 K target). **Q5 met:** one 16-B/slot clock per op.
- Hardened by an adversarial review (delivery / invariants / concurrency / API, each finding verified): fixed
  a real **Monotonic-Reads violation** — `read_latest` could flip to a *concurrent* sibling when one was
  delivered after a value had been read; now a per-session `last_read` cache pins it to causally-≥ results
  (+ regression test). Also: `by_key` is pruned to the live causal frontier on every write/deliver (bounds
  memory + read cost, was O(n²)/unbounded), and a debug-only guard catches reused instance ids. Three
  findings were correctly rejected.

### Phase 3a — Vector-clock engine

- `engram-consistency::vector_clock::VectorClock` (R7) — the causal-history metadata for ACC.
  `increment`/`merge`/`merged`/`happens_before`/`concurrent_with`, with the causal partial order exposed via
  `PartialOrd` (`Less` = happens-before, `None` = concurrent). Invariant: zero counters are never stored, so
  equality is structural.
- `to_bytes`/`from_bytes` — a canonical, fixed-width wire form of **exactly 16 bytes per instance** (8-byte
  id + 8-byte counter, sorted), proving the Q5 metadata bound (`O(|agents|)`, ≤ 16 B/slot).
- Tests: 6 unit + a 9-property proptest suite (happens-before is irreflexive/asymmetric/transitive; exactly
  one causal relation between any two clocks; `merge` is the commutative **least upper bound** of the clock
  lattice; increment advances causally; byte round-trip + 16 B/slot size).

### Phase 2e — Working memory

- `engram-storage::stores::working::WorkingMemory` — a bounded FIFO scratchpad (R2), default capacity 50,
  ephemeral (no WAL). `push` evicts the oldest entry over capacity, returning it *and* passing it to an
  optional **consolidation hook** (`with_hook`) — the hand-off point to Phase 4 consolidation. `items`/`ids`/
  `len`/`capacity`/`clear`. 4 unit tests (FIFO eviction, hook invocation, default cap).

### Phase 2d — Causal-provenance DAG store

- `engram-storage::stores::causal::CausalDag` (R5) — edges stored twice (forward `(from, to)` + reverse
  `(to, from)` adjacency) in two CoW B-trees, so `effects_of`/`causes_of` are `O(log n + k)` prefix scans.
  Durable (WAL-backed `create`/`open`) or ephemeral (`in_memory`).
- `add_edge` keeps the graph **acyclic**: rejects self-loops and any edge whose reverse path already exists
  (`StorageError::Cycle`), checked by BFS before the WAL write. `find_provenance_chain` (BFS over reverse) and
  `find_path` (BFS over forward) trace provenance. `open` replays committed (already-validated) edges.
- Tests: 5 unit + a proptest checking the acyclic invariant (independent Kahn's topo-sort) and that
  `add_edge` accepts/rejects exactly per an independent reachability oracle. `dag_ops` cargo-fuzz target
  (170k-run smoke, 0 crashes). **P5 met: provenance trace @ depth 1000 ~91 µs** (target < 2 ms).

### Phase 2c — Procedural store

- `engram-storage::stores::procedural::ProceduralStore` — versioned skills (R2) keyed `(agent_id, name,
  version)`; `put_skill` appends `version = prev + 1` with `supersedes` linking the prior version. `latest`
  (floor at `version = u32::MAX`), `get_version`, `history`, `get_by_id`; WAL-durable with recovery. 3 unit
  tests.

### Phase 2b — Semantic store (bitemporal + lazy decay)

- `CowBTree::floor` — the greatest entry `≤ key` in `O(log n)` (the as-of primitive), with a unit test and a
  `BTreeMap` differential-proptest check.
- `engram-storage::stores::semantic::SemanticStore` — mutable, bitemporal, versioned beliefs (R2/R3) with
  lazy confidence decay (R4). Each `upsert`/`retract` appends a new **version** keyed
  `(subject, predicate, tx_from)`; transaction-time is monotonic; "what was believed as-of T" is a single
  `floor` lookup. API: `upsert_belief`, `retract_belief`, `current`, `get_at_tx` (time-travel), `history`,
  `get_by_id`, `commit`; injectable `Clock`. Reads return a `BeliefView` whose confidence is decayed on read
  (no stored decayed value, no background task).
- Tests: 6 unit (10-edit history retained + time-travel to each version, decay monotonicity on read, retract
  hides current but keeps history, recovery, tx-time monotonicity after retract-then-reopen).
- **P3 met (~318 ns current point read), P4 met (~162 ns time-travel @ 200 versions), P7 met (~191 ns full
  decay-on-read; eval ≈ 3 ns)** — all 2–4 orders of magnitude under target.
- Hardened by an adversarial review (bitemporal/recovery/decay/concurrency, each finding verified): fixed a
  **non-atomic upsert** (a mid-write error could silently lose a live belief; now all WAL work precedes any
  index update, with a poisoned-writer guard so a half-written batch can't be committed) and a **recovery
  water-mark** that ignored `tx_until` (a rewound clock after retract+reopen could mint a version inside a
  closed interval; now both endpoints are covered, with a regression test). Documented per-tree cross-index
  visibility (atomic snapshot deferred to ACC). Two findings were correctly rejected.

### Phase 2a — Episodic store

- `engram-storage::stores::episodic::EpisodicStore` — immutable, append-only events (R2) on the WAL
  (durability) + four CoW B-tree indexes over a shared `Arc<EpisodicRecord>`: **primary** (`MemoryId`),
  **by-session** (`(SessionId, MemoryId)`), **by-time** (`(valid_time, MemoryId)`), and **by-cause**
  (`(cause_id, effect_id)` — the seed of the causal-provenance DAG, R5). API: `append`/`commit`/
  `append_committed`, `get`, `scan_session`, `scan_time_range` (half-open `[lo, hi)` by valid-time),
  `effects_of`. `open` replays the committed redo set; uncommitted tail events are dropped.
- Append-only contract enforced: re-appending an existing id returns `StorageError::Duplicate` (prevents
  phantom secondary-index entries). `Recovered` gained `max_tx_id` so a reopened store resumes tx ids above
  every id on disk (keeps position-aware redo sound).
- Tests: 6 unit + a 10k-event integration test (exact time slices, session isolation, full recovery).
- **P1 met:** 330 K ev/s durable at group-commit 4096 (169 K @ 1024); **P2 met:** 414 K ev/s bulk (mimalloc).
  Small-batch P1 is macOS-`fsync`-bound; the CoW write path is allocation-bound, so the throughput bench links
  mimalloc (system-allocator P2 is 295 K, logged honestly).
- Hardened by an adversarial review (recovery/index/concurrency/API dimensions, each finding verified):
  recovery and concurrency came back sound; fixed duplicate-id phantom entries and added decode-error context
  + a cross-index-visibility consistency note (atomic cross-index snapshot deferred to ACC).

### Phase 1c — Copy-on-write B-tree (MVCC)

- `engram-storage::btree` — `CowBTree<K, V>`, a persistent copy-on-write B-tree (R6). Writes clone only the
  touched root→leaf path (structural sharing) and swap the root atomically via `arc-swap`; reads are
  lock-free. `Snapshot` pins a version for MVCC, so concurrent readers at independent snapshots are
  unaffected by writers. API: `insert` (upsert), `get`, `range`, `iter`, `snapshot`, `len`, plus a seeking
  range iterator.
- Tests: 7 unit (split validity, MVCC snapshot isolation, concurrent-readers-during-writes) + a `BTreeMap`
  differential proptest + an ignored 1M-key gate test. `btree_ops` cargo-fuzz target (differential vs
  `BTreeMap`).
- **1c gate met:** 1,000,000 random-order inserts + full sorted scan in **~1.1 s** (release); old snapshots
  stay fully readable while the writer advances. `get` ~56 ns hit / ~33 ns miss on 1M keys.
- **Hardened by an adversarial multi-agent review** (5 dimensions, each finding independently verified): the
  B-tree's correctness and MVCC dimensions returned no findings; three fixes were applied — parent-directory
  `fsync` on WAL `create`/`compact` (durable directory entry on Unix), position-aware WAL recovery (a reused
  `tx_id` after commit can no longer replay uncommitted entries), and removal of a per-element `Arc` clone on
  the B-tree scan hot path.

### Phase 1b — Write-Ahead Log

- `engram-storage::wal` — an append-only, crash-safe WAL (R6): length-prefixed frames with a per-entry
  **CRC32** and a file header, an LSN writer with **batched fsync-on-commit** (group commit), a scanner that
  stops cleanly at the first torn/corrupt frame, `Wal::recover` (replays only the committed-transaction redo
  set), `Wal::open` (truncates a torn tail and resumes LSNs), and `Wal::compact` (atomic checkpoint/compaction
  via temp-file + rename).
- `WalOp` (`Put`/`Delete` carrying `RecordKind`, `Commit`, `Checkpoint`), `WalEntry`, `Recovered`, and a
  `StorageError`/`Result` (`thiserror`).
- Tests: 9 unit + 4 proptest properties (all-committed round-trip; only-committed recovery; truncate-at-any-
  offset is a safe prefix; arbitrary bytes never panic) + 1 doctest. `wal_reader` cargo-fuzz target
  (390k-run smoke, 0 crashes).
- **P8 met:** recover 1,000,000 committed entries in **~128 ms** (target < 2 s), 100% recovered. WAL append
  upper bound **1.56 M durable writes/s** (batched group commit).

### Phase 1a — Core types & MessagePack codec

- `engram-core` now implements the full §5 data model: `MemoryId` (128-bit, ULID-style, time-sortable,
  Crockford-base32 `Display`/`FromStr`, format-aware serde), `AgentId`/`AgentInstanceId`/`SessionId`,
  `Timestamp`, the `EventType`/`EdgeType` taxonomies, and the four record types
  (`EpisodicRecord`/`SemanticRecord`/`ProceduralRecord`/`WorkingMemoryRecord`) plus `CausalEdge`.
- Injectable determinism: `Clock` (`SystemClock`/`MockClock`) and `Rng` (`SplitMix64`/`SystemRng`) traits,
  plus a monotonic `MemoryIdGenerator` that stays strictly increasing within a millisecond.
- `DecayFunction::eval` (R4) — lazy, allocation-free confidence decay (exponential / regularized power-law /
  step / none); measured at **1.1–5.3 ns** per call, well under the 500 ns P7 target.
- MessagePack codec (`to_msgpack`/`from_msgpack`, named fields for schema evolution) and a `Record` trait
  with stable `RecordKind` wire tags for Phase 1b WAL framing.
- `thiserror`-based `EngramError`/`IdParseError`; no `unwrap`/`panic` on library paths.
- Tests: 19 unit + 7 proptest properties (every record type round-trips; arbitrary bytes never panic;
  id string + generator monotonicity) + 5 doctests.
- `fuzz/` cargo-fuzz workspace with the `record_codec` target — builds on nightly; 2.1M-run local smoke,
  0 crashes.

### Phase 0 — Scaffold & CI

- Cargo workspace with eight crates: `engram-core`, `engram-storage`, `engram-consistency`, `engram-query`,
  `engram-server`, `engram-py`, `engram-cli`, and `xtask`. All compile as documented, tested stubs over
  `std` with zero external dependencies.
- `cargo xtask` developer automation (`ci`, `fmt`, `clippy`, `build`, `test`, `bench`, `deny`, `cov`, `fuzz`,
  `demo`) wired through a cargo alias. The `ci` task is green on the reference machine.
- GitHub Actions CI: core gate (fmt-check, clippy `-D warnings`, build, test) plus supply-chain
  (`cargo-deny`), coverage (`cargo-llvm-cov`), bench-smoke, and nightly fuzz-smoke jobs.
- Supply-chain policy in `deny.toml`, including a hard ban on embedding any third-party storage engine.
- Living docs: `README.md`, `CLAUDE.md`, `DECISIONS.md` (ADR-0001…0006), `BENCHMARKS.md` (reference machine +
  target table), `docs/TRACEABILITY.md` (requirement → module → proving test).
- The `engram` CLI command surface (`init`/`put`/`get`/`as-of`/`why`/`bench`/`demo`/`version`/`help`) with a
  tested exit-code contract; subcommands are stubbed pending later phases.
