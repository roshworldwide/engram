# Engram Playground

A single, self-contained HTML page that makes Engram's three signature ideas
tangible — no build step, no server, no dependencies.

```bash
open playground/index.html        # macOS
xdg-open playground/index.html    # Linux
```

Three interactive panels:

- **Causal-provenance DAG** — click any node (try *restart service X*) to trace its
  provenance, exactly as `engine.provenance(id)` does — a reverse BFS over the
  causal edges.
- **Confidence decay** — pick exponential / power-law / step and drag the sliders
  to watch a belief fade; this is the lazy, read-time decay primitive.
- **Bitemporal time-travel** — drag the transaction-time slider over a versioned
  belief; the highlighted version is the `floor`-based "as-of T" answer.

The data is illustrative. The identical operations run on the real engine via the
Rust SDK, the `engram` CLI, REST/gRPC, or the Python wheel — see [`../docs/book/`](../docs/book/)
and [`../docs/paper/engram.md`](../docs/paper/engram.md).
