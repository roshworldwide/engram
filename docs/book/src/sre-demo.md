# The SRE provenance demo

The flagship demo ties every memory type together and shows off the "causal
memory" guarantee. Run it:

```bash
cargo xtask demo          # or: cargo run -p sre-demo
```

## The scenario

An SRE agent:

1. learns a **procedural** runbook — "restart-on-high-latency";
2. records **episodic** observations — a calm baseline, then
   `metric Y = 95% > threshold Z = 80%`;
3. infers a **semantic** belief — `service-x.health = unhealthy` (confidence 0.92),
   derived from that observation;
4. takes an **action** — `restart service X`, caused by the belief.

## The payoff: "why did the agent restart service X?"

Engram answers by tracing the causal-provenance DAG backward from the action:

```text
restart action
  └─ [semantic] service-x.health = unhealthy
      └─ [episodic Observation] metric Y = 95% > threshold Z = 80%
```

The action is explained by the belief; the belief by the root-cause observation.
No log grep, no guesswork — the provenance chain *is* the explanation. This is ACC
property P4 (causal memory) made operational: a visible belief always carries you
back to the evidence it was built from.

The same flow is available from Python (`import engram`) — see the customer-support
example in `examples/`.
