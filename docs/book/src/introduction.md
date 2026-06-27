# Engram

**Engram is a from-scratch Rust storage engine for AI-agent memory.** Memory that
**fades** (confidence decay), **knows why** it believes things (a causal-provenance
DAG), can be **rewound** (bitemporal time-travel), and is **safely shared** across
concurrent agent instances (Agent Causal Consistency). No third-party storage
engine is embedded — Engram writes its own on-disk format, write-ahead log, and
B-tree.

This book is the architecture guide. The formal treatment — the ACC contract, its
efficiency theorem, and the full evaluation — lives in the
[paper](../../paper/engram.md). The living build context is in `CLAUDE.md`.

## Why agent memory is not a database

A general-purpose database stores facts that are *true until overwritten*. Agent
memory is different in four ways, and each is a first-class feature here:

| Need | What Engram does | Chapter |
|---|---|---|
| Beliefs should **fade** without evidence | confidence decay evaluated lazily on read (~3 ns) | [Time & decay](./time-and-decay.md) |
| Every belief should **know why** | a queryable causal-provenance DAG | [Provenance](./provenance.md) |
| "What did it believe **last week**?" | bitemporal (valid-time + transaction-time) versioning | [Time & decay](./time-and-decay.md) |
| Many instances **share** memory safely | Agent Causal Consistency, no global sync | [ACC](./acc.md) |

## The shape of the system

```
            ┌──────────────────────────────────────────────┐
   agents → │  engram-server  (REST / gRPC)  ·  engram-py   │
            ├──────────────────────────────────────────────┤
            │  engram-query::Engine  (4 stores + DAG + WM)  │
            │  engram-consistency  (vector clocks + ACC)    │
            ├──────────────────────────────────────────────┤
            │  engram-storage:  WAL · CoW B-tree (MVCC) ·   │
            │     episodic/semantic/procedural/working/DAG  │
            ├──────────────────────────────────────────────┤
            │  engram-core:  ids · decay · records · codec  │
            └──────────────────────────────────────────────┘
```

Read on for each layer, bottom-up.

## Headline results

On the reference machine (Apple M3, 8 cores, 16 GB): **330K** durable
episodic writes/s, bitemporal time-travel in **~162 ns**, 1,000-deep provenance in
**~91 µs**, recover 1M WAL entries in **~128 ms**, and **165.8× faster**
time-travel than a hand-built PostgreSQL bitemporal schema. Full table in
[Evaluation](./evaluation.md).
