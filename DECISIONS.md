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

## ADR-0008 — WAL frame format & recovery semantics (Phase 1b)

**Status:** accepted (Phase 1b).

- **Frame = `[region_len:u32][region][crc32:u32]`, `region = [meta_len:u32][meta][record]`.** The `record`
  payload is kept *outside* the MessagePack `meta` so large already-serialized records are not re-encoded as
  MessagePack integer arrays (serde encodes `&[u8]`/`Vec<u8>` as sequences). `meta` (lsn, tx_id, op) stays
  small and named/evolvable. A leading 12-byte magic+version header identifies the file.
- **CRC32 per entry (covers `region`).** On read, any frame whose length is implausible (`< 4` or
  `> 256 MiB`), whose bytes are short (torn), or whose CRC/meta fails to decode is treated as **the end of
  the log** — recovery stops there. This makes a torn tail from a crash a non-event.
- **Redo only committed transactions.** `recover` returns `Put`/`Delete` entries whose `tx_id` has a durable
  `Commit` marker. A crash that tore the tail before a transaction's commit loses exactly that transaction
  and nothing committed earlier. `open` additionally truncates the torn tail so subsequent appends never
  follow garbage, and resumes LSNs at `max_valid + 1`.
- **Batched fsync (group commit).** `append` only buffers; `commit`/`sync` flush + `fsync`. Throughput scales
  with batch size (measured 1.56 M durable writes/s at a 10k batch) without sacrificing the durability point.
- **Compaction is copy-and-rename.** `compact(up_to_lsn)` rewrites surviving frames to a temp file, fsyncs,
  and atomically `rename`s over the original — never leaving a partially-written live log.
- **`record` stored raw, not interpreted at the WAL layer.** The WAL is a durable, ordered, checksummed byte
  log; routing `record` bytes to the right store on replay (via `WalOp`'s `RecordKind`) is the stores' job
  (Phase 2). This keeps the WAL reusable across all four memory types.

## ADR-0009 — `Arc`-based persistent copy-on-write B-tree for MVCC (Phase 1c)

**Status:** accepted (Phase 1c).

We implement the B-tree as a **persistent (immutable) structure**: nodes are `Arc<Node>`, a write clones only
the root→leaf path it touches and reuses every other subtree, and the `(root, len)` state is published with a
single atomic `arc_swap::ArcSwap` store. Chosen over an mmap'd paged on-disk B-tree for Phase 1c because:

- It is **fully safe Rust** (no `unsafe`), so MVCC correctness is easy to audit and test.
- **MVCC falls out for free:** a `Snapshot` is just a retained `Arc` to an old root; readers are lock-free
  (an `ArcSwap` load) and see an immutable version that never mutates underneath them. Writers are serialized
  by a `Mutex` (the engine writes through the WAL, one at a time); reads never block.
- Structural sharing keeps per-insert work to `O(B · height)` node copies (B = 32 ⇒ height ≈ 4 at 1M keys).

The trade-off — the tree lives on the heap, not in an mmap'd page file — is acceptable for now; a paged
on-disk representation (where the storage crate's allowed `unsafe` would live, for zero-copy reads) is
revisited in Phase 2 if the P3/P4 latency targets need it. The WAL already provides durability; this tree is
the in-memory index the WAL replays into.

## ADR-0010 — Durability hardening from the Phase 1c adversarial review

**Status:** accepted (Phase 1c).

A multi-agent adversarial review (5 dimensions, each finding independently verified by a separate skeptic)
drove three fixes; recording them so the rationale isn't lost:

- **Parent-directory `fsync`** on `Wal::create` and after the `Wal::compact` rename (Unix). A file `fsync`
  (`fdatasync`) persists the file's bytes/size but **not** its directory entry; without a directory `fsync` a
  freshly created WAL or a compaction rename can vanish on power loss even though committed bytes were
  flushed. This is required for the durability claim to hold against real crashes, not just process kills.
- **Position-aware recovery.** `recover` now redoes a data entry only if a `Commit` for the same `tx_id`
  exists at a strictly greater LSN (was: membership in a global committed-tx set). This stays correct even if
  a caller reuses a `tx_id` after it commits — the reused, uncommitted entries are dropped.
- **Removed a per-element `Arc` clone** in the B-tree scan iterator (it existed only to satisfy the borrow
  checker); the iterator now clones only the child `Arc` on descent.

The review also confirmed the B-tree's algorithmic-correctness and MVCC/concurrency dimensions had **no**
defects, and correctly rejected two non-issues (`fdatasync` is sufficient for append growth; non-durable
torn-tail truncation is harmless because recovery is CRC-gated).

## ADR-0011 — Episodic store: four CoW B-tree indexes, eager indexing, allocator choice (Phase 2a)

**Status:** accepted (Phase 2a).

- **Four indexes over one shared `Arc<EpisodicRecord>`:** primary (`MemoryId`), by-session
  (`(SessionId, MemoryId)`), by-time (`(valid_time, MemoryId)`), by-cause (`(cause_id, effect_id)`). Composite
  keys are plain Rust tuples (lexicographically `Ord`) — no manual byte-packing. The single `Arc` means the
  record is stored once and shared across indexes (refcount bumps, not copies).
- **Eager indexing** (index on `append`, before `commit`): simplest and fastest (one pass; 295 K/414 K vs
  ~250 K for a two-pass index-at-commit), and gives read-your-writes within the process. The trade-off — a
  reader can briefly see cross-index skew during an append, and uncommitted appends are visible in-memory — is
  documented; a true atomic cross-index snapshot is deferred to the ACC layer (Phase 3). A parallel
  index-at-commit was prototyped and **rejected**: the CoW node churn is allocation-bound and three threads
  contend on the system allocator, making it slower than eager, not faster.
- **Allocator, not algorithm, is the write bottleneck.** Profiling showed the CoW path-clone is allocation-
  bound; the throughput bench therefore links **mimalloc** (the allocator a production binary uses), which
  takes P2 from 295 K (system) to 414 K. This is an honest tuning choice, not a benchmark trick — `fsync` is
  never disabled, and both allocator numbers are reported in `BENCHMARKS.md`.
- **Append-only is enforced**, not assumed: a duplicate id is rejected (`StorageError::Duplicate`) under the
  writer lock before any WAL write, because the secondary indexes are insert-only (the CoW B-tree has no
  delete) and would otherwise accumulate phantom keys. Surfaced and fixed by the Phase 2a adversarial review.

## ADR-0012 — Semantic store: transaction-time version chain, `floor`-based as-of, lazy decay (Phase 2b)

**Status:** accepted (Phase 2b).

- **Versioning by a transaction-time chain.** Each `(subject, predicate)` is a sequence of versions keyed
  `(subject, predicate, tx_from)`; an upsert stamps a new version `tx_from = now` and closes the prior with
  `tx_until = now`. Transaction-time is made **strictly monotonic** in the store (`now.max(last_tx + 1)`), so
  two versions never collide on a key and the chain is contiguous. "What was believed as-of `T`" is then a
  single `CowBTree::floor((subject, predicate, T))` plus a `tx_until > T` check — `O(log n)`, measured
  ~160–320 ns even at 50k beliefs / 200 versions. Chosen over a per-belief version vector or a separate
  history table because it reuses the one B-tree primitive and makes time-travel a point lookup, not a scan.
- **Valid-time is stored but as-of queries are transaction-time.** Records carry both temporal intervals
  (R3), and retract closes valid-time too; the headline `get_at_tx` answers "what was *believed* as-of T"
  (transaction-time). Full valid-time-axis history (the bitemporal "trapezoid": multiple valid-time records
  per tx snapshot) is intentionally deferred — the transaction-time chain satisfies R3's "as-of" semantics
  and the 10-versions-retrievable contract.
- **Decay is lazy and on-read (P7).** Confidence is never stored decayed and no scheduler exists; reads return
  a `BeliefView` whose confidence is `DecayFunction::eval`uated (~3 ns) at the query's evaluation time. Zero
  background CPU by construction.
- **Writes are all-or-nothing in memory + can't half-commit.** An upsert closes-then-inserts two records; the
  Phase 2b review showed a mid-write WAL error could leave the chain inconsistent. Fixed: all encoding/WAL
  appends happen before any (infallible) index update, the new open version is published before the closed
  predecessor (so a concurrent reader never sees the belief vanish), and a partial WAL append **poisons the
  writer** so `commit` refuses — recovery then drops the orphaned, uncommitted frames. Recovery also folds
  `tx_until` (not just `tx_from`) into the resumed monotonic clock so a retract+reopen can't regress tx-time.

## ADR-0013 — Causal-DAG store: dual adjacency, BFS traversal, eager cycle rejection (Phase 2d)

**Status:** accepted (Phase 2d).

- **Two CoW B-trees, not a graph library.** R5 forbids a third-party graph crate, and the engine already has
  the CoW B-tree. An edge is stored as both `(from, to) → edge_type` (forward) and `(to, from) → edge_type`
  (reverse), so `effects_of`/`causes_of` are prefix range scans and `find_provenance_chain`/`find_path` are
  plain BFS over them. No separate graph structure to keep consistent or persist.
- **Eager cycle rejection.** `add_edge(from, to)` is rejected iff `from == to` or `from` is already reachable
  from `to` (a BFS before the write). This keeps the **acyclic invariant** as a hard precondition, validated
  on every add, rather than detected later. Replay trusts committed edges (each was validated at add time),
  so recovery does not re-run the check. The invariant is fuzzed (`dag_ops`) and proptested against an
  independent Kahn's-algorithm oracle.
- **Durable or ephemeral.** The DAG works `in_memory()` (no WAL) for the query layer, demos, benches, and
  fuzzing, or WAL-backed for durable provenance — one `Option<Wal>` in the writer, no API split.

## ADR-0014 — Working memory is an in-memory bounded FIFO with an eviction hook (Phase 2e)

**Status:** accepted (Phase 2e).

Working memory is the agent's short-term scratchpad — small, fast, ephemeral — so it is a plain bounded
`VecDeque` behind a mutex, **not** WAL-backed (losing scratch on restart is correct). Overflow evicts FIFO;
the evicted entry is both returned from `push` and handed to an optional **consolidation hook**, which is the
clean seam to the Phase 4 consolidation engine (an evicted working item is exactly what consolidation promotes
into a longer-term belief). A single global scratchpad (capacity 50) keeps 2e minimal; per-session
partitioning can layer on top later.

## ADR-0015 — Vector clock as a causal partial-order lattice (Phase 3a)

**Status:** accepted (Phase 3a).

`VectorClock` exposes the causal order through `PartialOrd` — `Less` = happens-before, `None` = concurrent —
which is the most idiomatic Rust encoding and lets the proptest assert lattice laws directly. We keep a
**no-zero-entry invariant** (an instance at counter 0 ≡ absent) so equality is structural. The wire form
[`to_bytes`] is a canonical fixed-width **16 bytes/slot** (8-byte id + 8-byte counter, sorted) rather than
MessagePack — MessagePack's `uint64` can be 9 bytes, which would break the ≤ 16 B/slot Q5 bound; the
fixed-width form makes the bound exact and the encoding deterministic.

## ADR-0016 — ACC as local causal delivery; monotonic single-value reads (Phase 3b)

**Status:** accepted (Phase 3b).

- **Causal delivery, no coordinator.** ACC is realized as classic vector-clock causal-order delivery over a
  shared append-only log: an instance delivers a write only once it is the next from that writer and all the
  write's cross-instance dependencies are already delivered locally. Visibility is a *local* decision from
  clocks — no consensus, total order, or coordinator — which is what makes the O(\|agents\|)-metadata,
  no-global-sync claim (R7/§9) true. The shared `Mutex<Vec>` is just the append substrate (a single-process
  stand-in for per-replica logs + gossip), not a consistency coordinator.
- **Dependencies are derived, not stored.** A write carries exactly one clock; its dependency set is the
  clock minus the writer's own latest tick. That keeps per-op metadata at one 16 B/slot clock (Q5).
- **Reads: frontier vs. monotonic single value.** `read` returns the causal frontier (all concurrent
  siblings) and is inherently monotonic (a value leaves the frontier only when causally superseded).
  `read_latest` projects a single deterministic value — and the Phase 3b adversarial review found that the
  naïve tiebreak could *regress to a concurrent sibling* when one arrived after a value had been read,
  violating Monotonic Sessions (§9.3). Fixed with a per-session `last_read` cache that only accepts a winner
  causally ≥ the last value returned. `by_key` is also pruned to the live frontier on every write/deliver,
  bounding memory and read cost to the concurrency width.
