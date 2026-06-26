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

## ADR-0007 — Phase 1a data-model & codec choices

**Status:** accepted (Phase 1a).

- **`MemoryId` is a ULID** (48-bit ms timestamp ‖ 80-bit randomness) so ids are globally unique *and*
  time-sortable — range scans over ids approximate time order for free, which the episodic store exploits in
  Phase 2a. Canonical text form is 26-char Crockford base32 (lenient decode: I/L→1, O→0).
- **Format-aware serde for `MemoryId`:** human-readable formats (JSON/REST) get the Crockford string;
  binary formats (MessagePack/on-disk) get a compact `(u64, u64)` pair. This sidesteps MessagePack's lack of
  a native 128-bit integer without a `serde_bytes` dependency.
- **MessagePack in *named* form** (`to_vec_named`): records serialize as field-name→value maps so the on-disk
  format can evolve (add fields with `#[serde(default)]`) without breaking old data. Slightly larger than
  array form; worth it for a storage engine that must read its own history.
- **`Step` decay is relative:** `eval(c0, elapsed_ns)` only receives elapsed time, so `Step::drop_at` is
  interpreted as an elapsed offset (ns since the belief's reference time), not an absolute wall-clock time.
  `PowerLaw` is regularized as `(1+t)^(−β)` to avoid the `t→0` singularity. Both keep `eval` pure, branch-
  light, and allocation-free (measured 1.1–5.3 ns).
- **Injectable `Rng` via a dependency-free `SplitMix64`** (deterministic, seedable) rather than pulling
  `rand` into core; `SystemRng` seeds it from the wall clock XOR a process counter. Non-cryptographic by
  design — ids must be unique, not unguessable.
