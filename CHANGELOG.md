# Changelog

All notable changes to Engram are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses 0.0.0 until the first tagged
milestone.

## [Unreleased]

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
