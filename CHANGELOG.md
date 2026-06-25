# Changelog

All notable changes to Engram are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses 0.0.0 until the first tagged
milestone.

## [Unreleased]

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
