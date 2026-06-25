# SRE provenance demo (the killer demo)

The flagship demonstration of causal provenance (Phase 4b). An SRE agent:

- stores **observations** as `episodic` memories (e.g. `E42: metric Y = 95% > threshold Z = 80%`),
- stores **runbook steps** as `procedural` memories,
- stores **inferred state** as `semantic` beliefs (e.g. `service X is unhealthy`),

then answers:

> **"Why did the agent restart service X?"**

by tracing the causal-provenance DAG from the restart action back to its root cause:

> *"It observed metric Y exceed threshold Z (episodic event E42)."*

Run with `cargo xtask demo` (currently prints the planned flow; implemented in Phase 4b).
