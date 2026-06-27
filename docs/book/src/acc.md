# Agent Causal Consistency

When the same agent runs as many concurrent instances sharing memory, each must
see a **causally consistent** view — never a belief without the events that
produced it — *without* forcing all instances through a global lock or consensus.
That guarantee is **Agent Causal Consistency (ACC)**, implemented in
`engram-consistency`. The formal contract and efficiency proof are in the
[paper](../../paper/engram.md); this is the working summary.

## The four guarantees

For every session, ACC ensures:

1. **Prefix Closure** — if a write is visible, all its causal predecessors are too.
2. **Session Consistency (read-your-writes)** — a session sees its own earlier
   writes.
3. **Monotonic Sessions** — reads never go backwards in time.
4. **Causal Memory** — a visible belief's evidence is retrievable in the same
   session.

(1)–(3) are the standard causal+session guarantees; (4) is specific to agent
memory.

## How: vector clocks + causal delivery

Each instance keeps a **vector clock** `vc` over instances. A write increments the
writer's slot and is tagged with the resulting clock; its dependencies are *derived*
from that clock (clock minus the writer's own latest tick), so **no separate
dependency set is stored** — one clock per operation, 16 bytes per slot (Q5).

A remote write `w` is **delivered** to a session exactly when it is causally ready:

```text
deliver(w)  ⟺  w.clock[writer] == vc[writer] + 1        // FIFO from the writer
              ∧  ∀ k ≠ writer : w.clock[k] ≤ vc[k]       // all cross-deps present
```

On delivery, `vc ← max(vc, w.clock)`. Own writes deliver immediately (read-your-
writes). A read returns the causally-maximal **frontier** for a key; the
single-value projection is pinned per session so it never regresses to a concurrent
sibling (monotonic reads).

## No global synchronization

The delivery predicate references only the writer's clock and the local `vc`, so
visibility is decided **locally** at each session — no coordinator, leader,
consensus, or total order is on the read or write path. Metadata is `O(|agents|)`.
That is the efficiency theorem; we validate it with **1,200 randomized
multi-agent histories** (2–5 instances, random interleavings) asserting all four
properties and eventual convergence, and measure **~2.5M ev/s across 10 instances**
with ACC on (P6).
