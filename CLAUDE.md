# CLAUDE.md — Engram build context

This is the living context file for building Engram. Keep it current at the end of every phase.

## What Engram is

A from-scratch Rust storage engine for **AI-agent memory**. Memory that fades (confidence decay), knows why
it believes things (causal-provenance DAG), can be rewound (bitemporal time-travel), and is safely shared
across concurrent agent instances (Agent Causal Consistency). No third-party storage engine is embedded — we
write the on-disk format, the WAL, and the B-tree ourselves.

The full specification (requirements R1–R9, performance gates P1–P8, quality gates Q1–Q6, data model,
phase plan, ACC formal contract, anti-cheating guardrails) is the master build prompt. This file is the
quick-orientation companion to it.

## Current status

- **Phase 0 — Scaffold & CI: COMPLETE.** Cargo workspace, 8 crates (7 + xtask) compiling, CI wired,
  living docs in place. `cargo xtask ci` is **GREEN** on the reference machine: fmt-check, clippy
  `-D warnings`, build, test, **and `cargo-deny`** (advisories/bans/licenses/sources ok) all pass.
- **Phase 1a — Core types & codec: COMPLETE.** `engram-core` implements the full §5 data model:
  `MemoryId` (128-bit ULID, time-sortable, Crockford base32 + format-aware serde), the other ids,
  `Timestamp`, injectable `Clock`/`Rng` (+ monotonic `MemoryIdGenerator`), `EventType`/`EdgeType`,
  lazy `DecayFunction::eval`, the four record types + `CausalEdge`, the MessagePack codec
  (`Record`/`RecordKind`), and `thiserror` errors. Tests: 19 unit + 7 proptest + 5 doctest, plus the
  `record_codec` fuzz target (2.1M-run smoke, 0 crashes). Decay eval **1.1–5.3 ns** (P7 primitive met).
  `cargo xtask ci` GREEN.
- **Phase 1b — Write-Ahead Log: COMPLETE.** `engram-storage::wal` — length+CRC32 framed, file header, LSN
  writer with batched fsync-on-commit, scanner that stops at the first torn frame, `recover` (committed-tx
  redo set), `open` (torn-tail truncation), `compact` (atomic checkpoint). 9 unit + 4 proptest + 1 doctest;
  `wal_reader` fuzz (390k smoke, 0 crashes). **P8 met: recover 1M in ~128 ms** (target < 2 s); WAL append
  upper bound 1.56 M durable writes/s. `cargo xtask ci` GREEN.
- **Phase 1c — Copy-on-write B-tree (MVCC): COMPLETE.** `engram-storage::btree::CowBTree` — persistent CoW
  B-tree, path-clone + atomic `arc-swap` root, lock-free reads, `Snapshot` MVCC. `insert`/`get`/`range`/
  `iter`/`snapshot`/`len`. 7 unit + `BTreeMap` differential proptest + `btree_ops` fuzz + ignored 1M gate.
  **Gate met: 1M random inserts + sorted scan ~1.1 s; old snapshots readable while writer advances.**
  `get` ~56 ns. An adversarial multi-agent review found the B-tree correctness/MVCC dimensions clean and led
  to 3 fixes (WAL dir-fsync, position-aware recovery, removed a hot-path Arc clone). `cargo xtask ci` GREEN.
- **Phase 2a — Episodic store: COMPLETE.** `engram-storage::stores::episodic::EpisodicStore` on the WAL +
  four CoW B-tree indexes (primary / by-session / by-time / by-cause) over a shared `Arc<EpisodicRecord>`.
  `append`/`commit`/`get`/`scan_session`/`scan_time_range`/`effects_of`; `open` replays the committed redo
  set. Append-only contract enforced (duplicate id rejected). 6 unit + 10k integration test. **P1 met (330k
  @ group-commit 4096), P2 met (414k bulk, mimalloc).** `Recovered` gained `max_tx_id`. Adversarial review
  fixed duplicate-id phantom entries + doc/diagnostics; recovery/concurrency confirmed sound. CI GREEN.
- **Phase 2b — Semantic store (bitemporal + lazy decay): COMPLETE.** `SemanticStore` — versioned beliefs
  keyed `(subject, predicate, tx_from)`, monotonic transaction-time, `CowBTree::floor`-based as-of queries.
  `upsert_belief`/`retract_belief`/`current`/`get_at_tx`/`history`/`get_by_id`; lazy decay-on-read
  (`BeliefView`); injectable `Clock`. 6 unit tests. **P3 ~318 ns, P4 ~162 ns @ 200 versions, P7 ~191 ns full
  read — all met.** Adversarial review fixed a non-atomic upsert (live-belief loss on mid-write error, now
  poisoned-writer guarded) and a recovery water-mark ignoring `tx_until`. CI GREEN.
- **Phase 2c/2d/2e — Procedural / Causal-DAG / Working memory: NOT STARTED.** Next: 2c procedural store
  (versioned skills, `supersedes` chain); 2d causal-DAG store (adjacency + reverse index, `add_edge`,
  `get_causes`/`get_effects`, `find_provenance_chain` BFS/DFS, **cycle rejection**, bench P5, `dag_decode`
  fuzz); 2e bounded working memory (FIFO cap 50, eviction→consolidation hook). Then Phase 3 (ACC).
- **Note:** the CoW write path is allocation-bound; the throughput bench links mimalloc (production allocator).
  Cross-index atomic snapshots are deferred to the ACC layer (Phase 3); per-index reads are MVCC-consistent.

## How to build, test, gate

```bash
cargo xtask ci        # full local gate (fmt-check, clippy -D warnings, build, test, deny*)
cargo xtask test      # tests only
cargo xtask clippy    # clippy with -D warnings
cargo xtask bench     # criterion benches (Phase 2+)
cargo build --workspace --all-targets
cargo test --workspace
```

The gate must be green before every commit. `*` tasks (deny/cov/fuzz) auto-skip when their tool is absent.

## Repository map

| Path | Role | Phase it lands |
|---|---|---|
| `crates/engram-core` | shared types: ids, timestamps, `DecayFunction`, errors | 1a |
| `crates/engram-storage` | WAL, CoW B-tree (MVCC), 4 stores, causal-DAG store, on-disk format | 1b–2 |
| `crates/engram-consistency` | vector clocks, session coordinator, ACC enforcement | 3a–3b |
| `crates/engram-query` | dispatch, time-travel, decay eval, provenance trace | 2–4 |
| `crates/engram-server` | axum REST + tonic gRPC | 3c |
| `crates/engram-py` | PyO3 bindings → `engram` wheel (maturin) | 3d |
| `crates/engram-cli` | the `engram` CLI | grows each phase |
| `xtask` | dev automation (`cargo xtask ...`) | 0 |
| `benches/` | criterion benches, one per P-metric | 2,4 |
| `fuzz/` | cargo-fuzz targets (separate nightly workspace) | 1+ |
| `demos/sre/` | the killer SRE provenance demo | 4b |
| `docs/paper/` | VLDB-style paper skeleton | 5a |
| `docs/book/` | mdBook architecture book | 5b |

## Conventions (non-negotiable)

- **TDD.** Failing test → implement → green → refactor. Property tests (`proptest`) for every invariant.
- **Green trunk.** No commit leaves CI red. Every public item has a doc comment with an example.
- **`unsafe`** is forbidden everywhere via `#![forbid(unsafe_code)]` **except `engram-storage`**, where it is
  allowed only for mmap/zero-copy paths, only with a `// SAFETY:` proof comment and a focused test.
- **No `unwrap()`/`panic!` on library paths.** Libraries use `thiserror`; binaries use `anyhow`.
- **Determinism.** Time and randomness flow through injectable `Clock`/`Rng` traits so time-travel and ACC
  histories are reproducible without real waiting.
- **No third-party storage engine, ever.** `deny.toml` bans rocksdb/lmdb/sqlite/sled/redb/heed/persy.
- **Commits.** Conventional (`feat:`, `fix:`, `test:`, `bench:`, `docs:`), small, each green.
- **Benchmark honesty.** Every number in `BENCHMARKS.md` is reproducible via `cargo xtask bench` on the
  documented machine; record p50/p99, sample size, warm-up. Never fabricate a number — log gaps instead.

## Reference machine

Apple M3 · 8 cores · 16 GB · macOS 26.5.1 · rustc 1.96.0 (stable, aarch64-apple-darwin). Full spec and the
performance-target table live in `BENCHMARKS.md`.

## Where to look next

- Decisions & rationale → `DECISIONS.md`
- Requirement → module → proving test → `docs/TRACEABILITY.md`
- Performance targets & latest numbers → `BENCHMARKS.md`
- Changelog → `CHANGELOG.md`
