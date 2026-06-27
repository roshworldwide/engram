# Engram: Agent Causal Consistency for AI-Agent Memory Storage

*A VLDB-2027-targeted paper. Numbers are reproduced from `BENCHMARKS.md` on the
reference machine (Apple M3, 8 cores, 16 GB, macOS 26.5.1, rustc 1.96.0).*

## Abstract

AI agents need a substrate for *memory*, not merely a database of facts. Agent
memory **fades** (confidence decays), is **introspectable** (every belief knows
why it is held), is **temporal** (what did the agent believe last week?), and is
**shared** across many concurrent instances of the same agent. General-purpose
databases—built for human-entered records that are true until overwritten—model
none of these well. We present **Engram**, a from-scratch storage engine for
AI-agent memory built on a custom copy-on-write B-tree and write-ahead log
(MVCC). Engram provides four native memory types (episodic, semantic, procedural,
working), bitemporal time-travel, confidence decay as a lazily-evaluated storage
primitive, and a queryable causal-provenance DAG. Its central contribution is
**Agent Causal Consistency (ACC)**, a consistency model for multi-instance agent
memory that guarantees read-your-writes, monotonic reads, and causal memory
**without global synchronization**, using `O(|agents|)` metadata per operation.
We formalize ACC, prove its efficiency, and validate it with 1,200 randomized
multi-agent histories. Engram sustains 330K durable episodic writes/s, answers
bitemporal time-travel queries in ~162 ns, traces 1,000-deep provenance chains in
~91 µs, and is **165.8× faster** at time-travel than a hand-built PostgreSQL
bitemporal schema.

## 1. Introduction

The memory of an LLM agent is not a ledger. When an agent observes "the user
prefers concise answers" twenty times, it should hold that belief with growing—but
never absolute—**confidence**, and that confidence should **decay** if the
evidence stops. When the agent acts, an operator must be able to ask **why**: to
trace an action back to the observation that triggered it. When the same agent
runs as ten concurrent instances answering ten conversations, each instance must
see a **causally consistent** view of shared memory—it must never see a belief
without the events that produced it—yet forcing all ten through a global lock or
consensus protocol would destroy throughput. And an operator must be able to ask
what the agent believed **as of last Tuesday**.

These requirements—fading, provenance, time-travel, and coordination-free
sharing—are the workload Engram is built for. Our contributions:

1. **Four native memory types** (§2) with the temporal and confidence semantics
   agents need, on a from-scratch CoW B-tree + WAL (no embedded third-party
   storage engine).
2. **Bitemporal time-travel and lazy confidence decay** as first-class storage
   primitives (§4): every belief carries valid-time and transaction-time; decay
   is evaluated only on read (~3 ns, zero background CPU).
3. **Agent Causal Consistency (ACC)** (§3): a formal consistency model for
   multi-instance agent memory, realizable with `O(|agents|)` metadata and **no
   global synchronization**, with a queryable causal-provenance DAG implementing
   the "causal memory" guarantee.
4. **A reproducible evaluation** (§5) meeting eight performance targets and
   showing a **165.8×** time-travel speedup over a hand-built PostgreSQL
   bitemporal schema, plus an SRE provenance case study (§6).

## 2. The agent-memory taxonomy

Engram models four memory types, mirroring cognitive science:

| Type | Mutability | Temporal model | Example |
|---|---|---|---|
| **Episodic** | immutable, append-only | valid-time + tx-time | "observed metric Y = 95% at 14:03" |
| **Semantic** | mutable, versioned | bitemporal | "service-x is unhealthy (conf 0.92)" |
| **Procedural** | versioned (supersedes chain) | versioned | runbook "restart-on-high-latency" v3 |
| **Working** | bounded FIFO scratchpad | ephemeral | the last 50 scratch items, auto-consolidated |

Episodic events are the ground truth from which semantic beliefs are
**consolidated** (§4.5); procedural skills evolve through explicit versions; the
working set is a bounded buffer whose evictions feed consolidation.

## 3. Agent Causal Consistency

### 3.1 System model

A *cluster* is a set of agent *instances* `{A₁ … Aₙ}`, each running one or more
*sessions*. An operation is a write (record an event / upsert a belief) or a read.
Causality `→` is the transitive closure of (i) program order within a session and
(ii) explicit causal edges (a belief derived from an event). We say a write `W`
is *visible* to a session if a read in that session can observe it.

### 3.2 The ACC contract

Engram guarantees, for every session, the following four properties.

> **(P1) Prefix Closure.** If a write `O` is visible, every `O' → O` is visible.
>
> **(P2) Session Consistency (read-your-writes).** A session's later reads see
> all of that session's earlier writes.
>
> **(P3) Monotonic Sessions.** If a read `R₁` saw write `W`, every later read
> `R₂` in the same session sees `W` or a causally-later write that supersedes it.
>
> **(P4) Causal Memory.** If a belief `B` derived from event `E` (`B → E`) is
> visible to a read, then `E` is retrievable in the same session.

P1–P3 are the standard causal+session guarantees; P4 is specific to agent memory:
a belief is never observable without the evidence it was built from.

### 3.3 Implementation: causal delivery over vector clocks

Each instance maintains a **vector clock** `vc` over instances. A write by `Aᵢ`
increments `vc[i]` and is tagged with the resulting clock; its causal
*dependencies* are precisely that clock with `Aᵢ`'s own latest tick removed, so
**no separate dependency set is stored** (Q5: one clock per op). A session
*delivers* a remote write `w` exactly when it is causally ready:

```
deliver(w)  ⟺  w.clock[w.inst] == vc[w.inst] + 1            (FIFO from the writer)
              ∧  ∀ k ≠ w.inst : w.clock[k] ≤ vc[k]           (cross-instance deps present)
```

On delivery, `vc ← max(vc, w.clock)`. Own writes are delivered immediately (P2).
Visibility is decided **locally** from clocks—no coordinator, consensus, or total
order participates. A read returns the causally-maximal *frontier* of delivered
writes for a key; the single-value projection is pinned per session to never
regress to a concurrent sibling (P3). P4 holds because a derived belief's clock
dominates the events it observed, so delivery orders them first.

### 3.4 Efficiency theorem

> **Theorem (ACC efficiency).** ACC (P1–P4) is realizable with `O(|agents|)`
> metadata per operation—at most 16 bytes per instance slot—and **no global
> synchronization** (no consensus, total order, or coordinator on the read or
> write path).

**Proof sketch.** *Metadata bound.* Each operation carries exactly one vector
clock; its dependency set is derived (clock minus own tick), not stored. The
canonical wire form encodes each instance slot as an 8-byte id plus an 8-byte
counter = 16 bytes, so an op's metadata is `16·|agents|` bytes. *No global
synchronization.* The delivery predicate above references only the writer's clock
and the local `vc`; it is evaluated independently at each session. Writes are
appended to a (per-replica, gossip-able) log; no operation blocks on a global
lock, leader, or agreement step. *Correctness.* Vector-clock causal delivery is
the classical realization of (P1)—a write is withheld until all its predecessors
are delivered, so the delivered set is always causally closed; (P2) follows from
delivering own writes immediately and the monotonicity of `vc`; (P3) from the
delivered set being monotone plus the per-session frontier pin; (P4) from a
derived belief's clock dominating its evidence's clocks, which (P1) then orders
first. The frontier read returns an antichain of delivered writes, so concurrent
updates converge deterministically without coordination. ∎

We validate the theorem operationally: 1,200 randomized multi-agent histories
(2–5 instances, random write/refresh interleavings) assert P1–P4 and eventual
convergence against an independent clock oracle, all passing.

## 4. Storage design

Engram writes its own bytes; no third-party storage engine is embedded.

**Write-ahead log.** Length-prefixed frames carry a per-entry CRC32; the writer
batches `fsync` on commit (group commit). Recovery replays only the
committed-transaction redo set and stops at the first torn frame, so a crash that
tore the tail loses exactly the uncommitted transaction. Recovering 1,000,000
entries takes ~128 ms.

**Copy-on-write B-tree (MVCC).** Writes clone only the touched root→leaf path and
publish the new root with one atomic `arc-swap`; readers load a snapshot
lock-free and observe an immutable version. A `floor(key)` primitive returns the
greatest entry `≤ key` in `O(log n)`—the basis of as-of queries.

**Semantic store (bitemporal).** Each `(subject, predicate)` is a chain of
versions keyed `(subject, predicate, tx_from)`; transaction-time is strictly
monotonic. "What was believed as-of `T`" is a single `floor((s, p, T))` plus an
open-interval check—`O(log n)`, ~162 ns even at 200 versions. Confidence is never
stored decayed: reads evaluate `DecayFunction::eval` (exponential / regularized
power-law / step / none) at the query time in ~3 ns, with **zero background CPU**.

**Causal-DAG store.** Edges are stored twice—forward `(from, to)` and reverse
`(to, from)` adjacency—so `effects_of`/`causes_of` are prefix scans and
`find_provenance_chain` is a BFS. `add_edge` rejects any edge that would close a
cycle, keeping the graph acyclic (validated by a Kahn's-algorithm proptest and a
differential fuzz target).

**Consolidation.** A background task promotes beliefs from repeated episodic
evidence: it groups events by `(subject, predicate)`, promotes the dominant object
once evidence clears a threshold, and assigns confidence `1 − (1 − w)^n` for `n`
confirmations. The belief's provenance is exactly the supporting events—20 "be
concise" observations consolidate to one belief at confidence 0.878 with a 20-id,
DAG-traceable provenance.

## 5. Evaluation

Reference machine: Apple M3 (8 cores), 16 GB, macOS 26.5.1, rustc 1.96.0; the
CoW write path links mimalloc. All numbers are reproducible via `cargo xtask
bench` / `cargo run -p compare-postgres`.

| Metric | Target | Engram |
|---|---|---|
| P1 durable episodic writes (fsync-batched) | ≥ 100K /s | **330K /s** |
| P2 bulk writes | ≥ 300K /s | **414K /s** |
| P3 semantic point read | < 400 µs p99 | **~318 ns** |
| P4 time-travel @ 200 versions | < 5 ms p99 | **~162 ns** |
| P5 provenance trace, depth 1000 | < 2 ms p99 | **~91 µs** |
| P6 10-instance write throughput, ACC on | ≥ 250K /s | **~2.5M /s** |
| P7 decay-on-read | < 500 ns | **~191 ns** (eval ~3 ns) |
| P8 recover 1,000,000 WAL entries | < 2 s | **~128 ms** |
| **Q4 time-travel vs PostgreSQL** | ≥ 10× | **165.8×** |

**Q4 (the headline comparison).** We load the same 200-version belief into Engram
and into a hand-built PostgreSQL bitemporal schema (`beliefs(subject, predicate,
object, tx_from, tx_until)` with a `(subject, predicate, tx_from)` index) and run
the identical as-of query. Engram answers in **146.7 ns** (in-process `floor`);
PostgreSQL answers in **24,322.9 ns** (an indexed bitemporal `SELECT` over its
client/server protocol)—**165.8×**. The gap reflects the honest deployment
reality: an agent's memory is in-process, whereas a Postgres-backed memory pays a
socket round-trip, parse, and plan per query.

**Quality gates.** 1,200 randomized multi-agent ACC histories (Q1); ACC metadata
≤ 16 B/slot, proven (Q5); 93.44% line coverage on the storage + consistency
crates (Q3); fmt/clippy(`-D warnings`)/test/`cargo-deny` green on every commit
(Q6); four cargo-fuzz targets smoke-clean (Q2, full 10M nightly).

## 6. Case study: SRE provenance

An SRE agent stores observations as episodic events, a runbook as a procedural
skill, and inferred state as a semantic belief, then takes an action. Asked *"why
did the agent restart service X?"*, Engram traces the causal-provenance DAG:

```
restart action
  └─ [semantic] service-x.health = unhealthy
      └─ [episodic Observation] metric Y = 95% > threshold Z = 80%
```

The action is explained by the belief, which is explained by the root-cause
observation—exactly the P4 (causal memory) guarantee, made operational. This runs
end-to-end via `cargo xtask demo`.

## 7. Related work

Bitemporal databases (SQL:2011, Snodgrass) model valid- and transaction-time but
target human-scale, server-resident workloads; Engram makes the as-of query an
in-process `O(log n)` lookup. MVCC (PostgreSQL, the persistent-data-structure
literature) underlies our CoW B-tree. Causal consistency (Lamport; vector clocks,
Fidge/Mattern; COPS, Bolt-on causal consistency) provides the foundation for ACC;
we specialize it with the **causal-memory** (P4) guarantee tying beliefs to
evidence, and with the agent-session framing. CRDTs inform our frontier/sibling
convergence. Agent-memory systems (vector stores, scratchpad memories) provide
retrieval but not bitemporal history, confidence decay, provenance, or a formal
multi-instance consistency model.

## 8. Conclusion

Engram treats agent memory as a first-class storage problem: fading, provenance,
time-travel, and coordination-free sharing, on a from-scratch CoW B-tree + WAL.
Agent Causal Consistency gives multi-instance agents read-your-writes, monotonic
reads, and causal memory with `O(|agents|)` metadata and no global
synchronization. The result meets eight performance targets and is 165× faster at
time-travel than a hand-built PostgreSQL schema, while remaining fully
introspectable—every belief can be traced to the events that produced it.
