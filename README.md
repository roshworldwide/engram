# Engram

> *Databases were built for humans storing records. **Engram** is built for AI agents that remember.*

![status](https://img.shields.io/badge/status-complete-success)
![requirements](https://img.shields.io/badge/gates-R1–R9%20·%20P1–P8%20·%20Q1–Q6-brightgreen)
![rust](https://img.shields.io/badge/rust-1.96%2B-orange)
![unsafe](https://img.shields.io/badge/unsafe-forbidden%20(except%20mmap)-informational)
![license](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)

**Engram is a from-scratch Rust storage engine for AI-agent memory.** A conventional database stores facts
that are true until overwritten. Agent memory is different: it should **fade** when evidence stops, **know
why** it believes things, be **rewindable in time**, and be **safely shared across many concurrent agent
instances**. Engram makes each of those a first-class storage primitive — on a copy-on-write B-tree and
write-ahead log it writes itself (no RocksDB / LMDB / SQLite / sled / redb embedded).

---

## See it — *"why did the agent restart service X?"*

```console
$ cargo xtask demo

Q: why did the agent restart service X?
   tracing the provenance of action 01KW…BBEME:
   └─ 01KW…0000   [semantic]  service-x.health = unhealthy
      └─ 01KW…NRDF   [episodic Observation]  metric Y = 95% > threshold Z = 80%

A: it observed metric Y exceed threshold Z (episodic 01KW…NRDF).
```

The action is explained by a belief; the belief by the root-cause observation — Engram walks the
causal-provenance DAG to answer *why*. No log-grepping, no guessing. This is the "causal memory" guarantee,
made operational.

---

## What makes it different

| Capability | What it means |
|---|---|
| **Four native memory types** | `episodic` (immutable events) · `semantic` (mutable beliefs) · `procedural` (versioned skills) · `working` (bounded auto-consolidating scratchpad) |
| **Bitemporal time-travel** | Every record carries *valid-time* and *transaction-time* — ask "what was believed **as of** last Tuesday?" in `O(log n)` |
| **Confidence decay** | A lazily-evaluated storage primitive (exponential / power-law / step) — computed only on read (~3 ns), **zero background CPU** |
| **Causal-provenance DAG** | Every memory links to the memories that caused it; provenance chains are queryable (and acyclic by construction) |
| **Custom CoW B-tree + WAL** | A copy-on-write B-tree + write-ahead log built from scratch, giving MVCC — lock-free readers at independent snapshots |
| **Agent Causal Consistency** | A multi-instance consistency model: read-your-writes, monotonic reads, and causal memory — with **no global synchronization** |

## Results

On the reference machine (Apple M3, 8 cores, 16 GB) — every number reproducible via `cargo xtask bench`:

| Metric | Result |
|---|---|
| Durable episodic writes | **330K /s** (P1) · **414K /s** bulk (P2) |
| Semantic point read · time-travel @ 200 versions | **~318 ns** (P3) · **~162 ns** (P4) |
| Provenance trace, depth 1000 | **~91 µs** (P5) |
| 10-instance throughput, ACC on | **~2.5M ev/s** (P6) |
| Decay-on-read (zero background CPU) | **~191 ns** (P7) |
| Recover 1,000,000 WAL entries | **~128 ms** (P8) |
| **Time-travel vs hand-built PostgreSQL** | **165.8× faster** (146.7 ns vs 24,322.9 ns) — Q4 |
| Line coverage (storage + consistency) | **93.44%** — Q3 |
| Fuzzing | **4 targets × 10,000,000 runs, 0 crashes** — Q2 |

> **Status:** complete and green (`cargo xtask ci`). All nine requirements (**R1–R9**), eight performance
> gates (**P1–P8**), and six quality gates (**Q1–Q6**) are met.

## Quickstart

Requires a stable Rust toolchain (**1.96+**).

```bash
git clone https://github.com/roshworldwide/engram.git && cd engram

cargo xtask ci         # full local gate: fmt · clippy -D warnings · build · test · cargo-deny
cargo xtask demo       # the SRE provenance demo (shown above)
cargo xtask bench      # criterion benchmarks

# The CLI — a memory in five commands
cargo run -p engram-cli -- init  /tmp/mem
cargo run -p engram-cli -- put   belief /tmp/mem service-x health unhealthy 0.92
cargo run -p engram-cli -- get   belief /tmp/mem service-x health      # unhealthy (confidence 0.920)
cargo run -p engram-cli -- why   /tmp/mem <id>                         # trace provenance
```

Optional tooling — `cargo-deny`, `cargo-llvm-cov`, `cargo-fuzz` (nightly), `protoc` (gRPC), `maturin`
(Python), `postgresql` (the Q4 comparison) — is auto-detected; `cargo xtask ci` reports absent tools as
`SKIP`.

## Using it

Engram is reachable four ways, all over the same `engram-query::Engine`.

**Rust**
```rust
use engram_query::Engine;
use engram_core::{AgentId, SessionId, Timestamp, EventType, DecayFunction};

let engine = Engine::open("/tmp/mem")?;
let obs    = engine.record_event(AgentId(1), SessionId(7), Timestamp::from_millis(100),
    EventType::Observation, b"metric Y = 95%".to_vec(), vec![])?;
let belief = engine.upsert_belief(AgentId(1), "service-x".into(), "health".into(),
    b"unhealthy".to_vec(), Timestamp::from_millis(100), 0.92, DecayFunction::None, vec![obs])?;
let action = engine.record_event(AgentId(1), SessionId(7), Timestamp::from_millis(101),
    EventType::Action, b"restart service X".to_vec(), vec![belief])?;

assert_eq!(engine.provenance(action), vec![belief, obs]);   // why?
```

**Python** (`maturin develop --release --features python` in `crates/engram-py`)
```python
import engram
mem = engram.Engram("/tmp/mem")

obs    = mem.record_event(1, 7, 100, "observation", "metric Y = 95%")
belief = mem.upsert_belief(1, "service-x", "health", "unhealthy",
                           valid_from_ms=100, confidence=0.92, provenance_ids=[obs],
                           decay="exponential", decay_rate=1e-7)
action = mem.record_event(1, 7, 101, "action", "restart service X", cause_ids=[belief])

mem.provenance(action)                       # [belief, obs] — why did it restart?
mem.current_belief("service-x", "health")    # ('unhealthy', ~0.92) — confidence decayed on read
mem.belief_at("service-x", "health", t)      # time-travel: as-of transaction time t
```

**REST / gRPC** — `engram-server` exposes the engine over axum (HTTP) and tonic (gRPC, behind the `grpc`
feature). See [`docs/book/using-engram.md`](docs/book/src/using-engram.md).

Five runnable example agents live in [`examples/`](examples/) — a customer-support agent that learns a
preference over 100 conversations (consolidation → provenance → time-travel → decay), plus SRE,
personal-assistant, knowledge-base, and research agents.

## Architecture

```
Client APIs    Rust SDK · Python SDK (PyO3) · REST (axum) · gRPC (tonic)
Query Engine   type dispatch · time-travel resolver · DAG traversal · decay eval · provenance tracer
Consistency    vector-clock engine · session coordinator · read-your-writes / monotonic / causal validators
Storage Engine episodic · semantic · procedural · working · causal-DAG store · write-ahead log
Background     consolidation engine · decay (lazy, on read) · working-memory eviction hook
```

Requests flow down; results flow up. The consistency layer is bypassable — single-agent mode pays **zero**
vector-clock overhead.

```
crates/
  engram-core         shared types: ids, timestamps, decay functions, codec, errors
  engram-storage      WAL, copy-on-write B-tree (MVCC), the four stores, causal-DAG store
  engram-consistency  vector clocks, session coordinator, ACC enforcement
  engram-query        engine, time-travel, decay eval, provenance trace, consolidation
  engram-server       axum REST + tonic gRPC
  engram-py           PyO3 bindings → the `engram` Python wheel
  engram-cli          the `engram` CLI
xtask/                dev automation (cargo xtask ci|bench|demo)
benches/  fuzz/  examples/  demos/sre/  playground/  docs/{book,paper}/
```

## Documentation

| | |
|---|---|
| 📄 **Paper** | [`docs/paper/engram.md`](docs/paper/engram.md) — VLDB-style: the formal ACC contract, the efficiency theorem + proof sketch, evaluation, SRE case study |
| 📖 **Architecture book** | [`docs/book/`](docs/book/) — 9-chapter mdBook (`mdbook build docs/book`) |
| 🎛️ **Playground** | [`playground/index.html`](playground/index.html) — self-contained, no build: click the causal DAG, drag the decay & time-travel sliders |
| 📊 **Benchmarks** | [`BENCHMARKS.md`](BENCHMARKS.md) — targets, methodology, reference machine |
| 🧭 **Traceability** | [`docs/TRACEABILITY.md`](docs/TRACEABILITY.md) — requirement → module → proving test |
| 🧱 **Decisions** | [`DECISIONS.md`](DECISIONS.md) — ADRs |

## How it's built

- **Test-first.** Failing test → implement → green → refactor. Property tests (`proptest`) for every invariant;
  four `cargo-fuzz` targets; a B-tree differential oracle against `BTreeMap`.
- **Green trunk.** Every commit passes `cargo xtask ci` (fmt · clippy `-D warnings` · build · test ·
  `cargo-deny`). Conventional commits.
- **`#![forbid(unsafe_code)]`** everywhere except `engram-storage`, where `unsafe` is allowed only for
  mmap/zero-copy paths with a `// SAFETY:` proof and a focused test.
- **No third-party storage engine, ever** — banned in `deny.toml`.
- **Benchmark honesty** — every number is reproducible on the documented machine; gaps are logged, never
  faked.

## License

Dual-licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
