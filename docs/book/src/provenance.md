# Causal-provenance DAG

Every belief should know **why** it is held. Engram stores causal edges in a
directed acyclic graph and makes provenance a query.

## Dual adjacency

The `CausalDag` store keeps each edge **twice** — forward `(from → to)` and
reverse `(to → from)` adjacency, each in a CoW B-tree. That makes both directions
a prefix scan:

- `get_effects(x)` — "what did `x` cause?" (forward)
- `get_causes(x)` — "what caused `x`?" (reverse)

`find_provenance_chain` then walks the reverse index breadth-first to return the
full ancestry of a node. Tracing a **1,000-deep** chain takes **~91 µs** (P5,
target < 2 ms).

## Acyclic by construction

`add_edge` **rejects** any edge that would close a cycle — checked before the edge
is committed, so the graph is acyclic at all times. This is validated by a
Kahn's-algorithm topological-sort proptest and a differential fuzz target
(`dag_ops`).

## Consolidation links beliefs to evidence

When the consolidation engine promotes a belief from repeated episodic evidence,
it records a provenance edge from each supporting event to the new belief. So:

```text
20 × observe "user / response_length / concise"
        │  consolidate (dominant object, ≥ threshold)
        ▼
   belief: user.response_length = concise   (confidence 0.878 = 1 − 0.9²⁰)
        │  provenance = the 20 event ids
        ▼
   engine.provenance(belief)  ==  those 20 events
```

This is the "causal memory" guarantee (ACC property P4) made operational — and the
backbone of the [SRE demo](./sre-demo.md).
