# Architecture Decision Records

Each ADR records a non-obvious choice and its rationale. Newest at the bottom.

---

## ADR-0001 — Build the storage engine from scratch (no embedded third-party engine)

**Status:** accepted (Phase 0) · **Context:** R1 requires an original engine.

We implement the on-disk format, write-ahead log, and B-tree ourselves rather than embedding
RocksDB/LMDB/SQLite/sled/redb. This is the central claim of the project and the only way the
copy-on-write-B-tree/MVCC and bitemporal storage work is genuinely ours. `deny.toml` bans those crates so the
constraint cannot regress silently. **Consequence:** more work and more tests, which is the point.

## ADR-0002 — Rust + tokio + serde/rmp-serde + memmap2 as the base stack

**Status:** accepted (Phase 0).

Rust for memory safety without GC and predictable latency. `tokio` for async service surfaces. `serde` +
`rmp-serde` (MessagePack) for compact, schema-evolvable record encoding. `memmap2` for zero-copy reads of the
on-disk B-tree. `crc32fast` for per-WAL-entry checksums. `thiserror` (libs) / `anyhow` (bins) for errors.
These are *libraries*, not a storage engine — they do not violate ADR-0001.

## ADR-0003 — Edition 2021, stable toolchain pinned via `rust-toolchain.toml`

**Status:** accepted (Phase 0).

Edition 2021 is the broadest-compatibility modern edition. The engine builds and tests on **stable**; only the
`cargo-fuzz` targets need nightly, which CI installs in an isolated job. Pinning the toolchain keeps benchmark
numbers comparable across machines.

## ADR-0004 — `xtask` pattern for dev automation, written in pure std

**Status:** accepted (Phase 0).

Rather than a Makefile or shell scripts, automation lives in a Rust `xtask` crate invoked via the cargo alias
`cargo xtask <task>` (`.cargo/config.toml`). It has **zero dependencies** so it builds instantly and offline.
Optional tools (`cargo-deny`, `cargo-llvm-cov`, `cargo-fuzz`) are auto-detected and reported as `SKIP` (not
`FAIL`) when absent, so the core gate stays green on a clean machine while CI enforces the full gate.

## ADR-0005 — Phase 0 is dependency-free; heavy deps land in the phase that needs them

**Status:** accepted (Phase 0).

The scaffold pulls **no external crates**: every crate is a documented, tested stub over `std`. Shared
dependency versions are pre-declared in `[workspace.dependencies]` but stay inert until a member opts in.
`engram-py` is a plain library (not yet a PyO3 `cdylib`) and `engram-server` has no axum/tonic yet — those
arrive in Phase 3 so the Phase 0 gate needs neither `maturin` nor `protoc`. This guarantees a fast, offline,
reproducible Phase 0 gate and keeps the dependency graph honest as it grows.

## ADR-0006 — `unsafe` confined to `engram-storage`, everything else `#![forbid(unsafe_code)]`

**Status:** accepted (Phase 0).

Only the storage crate may use `unsafe`, and only for mmap/zero-copy paths, each with a `// SAFETY:` proof
comment and a focused test. All other crates forbid `unsafe` at the crate level. This concentrates the audit
surface to one well-tested place.
