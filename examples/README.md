# Example agents

Five reference agents built on the Engram Python SDK (Phase 5d). Each demonstrates a different capability:

1. **customer-support** — after 100 conversations, shows the learned-preference DAG, a week-1-vs-week-4
   time-travel diff, a decayed belief, and a consolidation summary (~200 events → ~12 beliefs).
2. **sre-agent** — the SRE provenance demo as a runnable agent (see also `../demos/sre/`).
3. **research-assistant** — semantic beliefs with provenance into source episodic observations.
4. **coding-agent** — procedural skills with versioning / `supersedes` chains.
5. **multi-agent-swarm** — several instances sharing memory under ACC (read-your-writes, monotonic reads,
   causal memory) with no global synchronization.

These land once the Python SDK ships in Phase 3d and are fleshed out in Phase 5d.
