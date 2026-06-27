# The agent-memory taxonomy

Engram models four memory types, mirroring cognitive science. Each is a store over
the shared WAL + CoW B-tree foundation, exposed through `engram-query::Engine`.

| Type | Store | Mutability | Temporal model |
|---|---|---|---|
| **Episodic** | `EpisodicStore` | immutable, append-only | valid-time + transaction-time |
| **Semantic** | `SemanticStore` | mutable, versioned | bitemporal |
| **Procedural** | `ProceduralStore` | versioned (`supersedes` chain) | versioned |
| **Working** | `WorkingMemory` | bounded FIFO scratchpad | ephemeral |

## Episodic — what happened

Immutable events: observations, messages, actions. Append-only (a duplicate id is
rejected), indexed four ways — by id, by session, by time, and by cause — so you
can scan a session, scan a time range, or ask "what did this event cause?". These
are the ground truth from which semantic beliefs are consolidated.

## Semantic — what is believed

Mutable, **versioned** beliefs keyed `(subject, predicate)`. Each upsert appends a
new version (it never overwrites), so the full history is queryable and every
belief carries a confidence and a decay function. See
[Time & decay](./time-and-decay.md).

## Procedural — what the agent knows how to do

Skills/runbooks that evolve through explicit versions. Each `put_skill` creates a
new version linked to its predecessor through a `supersedes` chain, so you can ask
for the latest version or walk the lineage.

## Working — the scratchpad

A bounded (default 50) FIFO buffer of recent items. When it overflows, the evicted
item fires a **consolidation hook** — the bridge by which fleeting working memory
becomes durable episodic/semantic memory.

## Consolidation — episodic → semantic

A background task ([`engram-query::consolidation`](./provenance.md)) promotes
beliefs from *repeated* episodic evidence: group events by `(subject, predicate)`,
promote the dominant object once evidence clears a threshold, and assign
confidence `1 − (1 − w)^n` for `n` confirmations. The belief's provenance is
exactly the supporting events — so a consolidated belief is always traceable back
to the events it was derived from.
