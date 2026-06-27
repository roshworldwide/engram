# Bitemporal time-travel & lazy decay

## Two timelines

Every belief carries **two** times:

- **valid-time** — when the fact is true *in the world* ("the user preferred
  concise answers starting Monday").
- **transaction-time** — when Engram *recorded* it ("we learned this Tuesday").

Keeping both lets you answer *"what did the agent believe **as of** last Tuesday?"*
— the transaction-time as-of query — independently of when those facts were valid.

## How versioning works

The `SemanticStore` keys each version `(subject, predicate, tx_from)` and keeps
transaction-time strictly monotonic. An upsert **appends** a new version; it never
overwrites. A retraction writes a tombstone with a `tx_until`, hiding the belief
from the present while keeping it in history.

An as-of query is then a single B-tree **`floor((subject, predicate, T))`** plus an
open-interval check:

```text
get_at_tx(subject, predicate, T)
  = floor((subject, predicate, T))     // greatest version recorded at or before T
    if its [tx_from, tx_until) contains T
```

`O(log n)`, **~162 ns even at 200 versions** (P4) — and **165.8× faster** than the
equivalent indexed SQL on PostgreSQL (it pays a socket round-trip; Engram does
not).

## Confidence decay as a storage primitive

A belief's confidence is **never stored decayed**. Each belief carries a
`DecayFunction`; reads evaluate it at the query timestamp:

- **Exponential** — `c₀ · e^(−λ·Δt)`
- **Power-law** — `c₀ · (1 + Δt)^(−β)` (regularized so `Δt = 0 ⇒ c₀`)
- **Step** — `c₀` until a cutoff, then a floor
- **None** — constant

Evaluation is **~3 ns** and happens only on read, so there is **zero background
CPU** spent aging beliefs — a million dormant beliefs cost nothing until queried
(P7). Decay is monotonic in `Δt` (a proptest invariant): re-reading a belief later
never *raises* its confidence.
