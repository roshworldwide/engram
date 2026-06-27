# Paper — *Engram: Agent Causal Consistency for AI-Agent Memory Storage*

**The full paper is [`engram.md`](engram.md)** (VLDB-2027-targeted). It contains the formal ACC contract,
the efficiency theorem and its proof sketch, the evaluation table with real numbers, and the SRE case study.
This file is the section outline:

1. **Motivation** — why agent memory is not a database workload (fading, provenance, time-travel, sharing).
2. **Memory-type taxonomy** — episodic / semantic / procedural / working.
3. **Agent Causal Consistency** — formal definition (Prefix Closure, Session Consistency, Monotonic Sessions,
   Causal Memory) and the efficiency theorem: ACC is achievable with `O(|agents|)` metadata per operation and
   no global synchronization, with a proof sketch.
4. **Storage design** — on-disk format, WAL, copy-on-write B-tree / MVCC, bitemporal indexing, lazy decay,
   causal-DAG store.
5. **Evaluation** — the P-metric suite and the ≥ 10× time-travel result vs a hand-built PostgreSQL bitemporal
   schema; all numbers pulled from `../../BENCHMARKS.md`.
6. **Case study** — the SRE provenance trace.
7. **Related work** — bitemporal databases, MVCC, causal consistency, vector clocks, agent-memory systems.

The ACC formal contract that the implementation encodes as runtime checks and proptest oracles is the
authoritative source for §3.
