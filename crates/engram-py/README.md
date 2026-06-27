# engram (Python)

PyO3 bindings for the [Engram](../../README.md) AI-agent memory engine.

```bash
# from crates/engram-py, with a virtualenv active
maturin develop --features python      # or: maturin build --features python
```

```python
import engram

mem = engram.Engram("./data")          # opens/creates all stores under ./data

obs = mem.record_event(agent=1, session=1, valid_time_ms=1000,
                       event_type="Observation", payload="metric Y = 95% > 80%")
act = mem.record_event(agent=1, session=1, valid_time_ms=1001,
                       event_type="Action", payload="restart X", cause_ids=[obs])

mem.upsert_belief(agent=1, subject="service-x", predicate="health",
                  object="unhealthy", valid_from_ms=1000, confidence=0.9,
                  provenance_ids=[obs])

obj, confidence = mem.current_belief("service-x", "health")   # ("unhealthy", ~0.9)
mem.belief_at("service-x", "health", at_ms=...)               # time-travel
assert mem.provenance(act) == [obs]                           # why did it act?
```

The Rust workspace builds this crate without the `python` feature (an empty
cdylib), so `cargo build` needs no Python toolchain; maturin builds the wheel
with the feature on.
