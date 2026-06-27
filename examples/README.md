# Example agents

Five reference agents built on the Engram Python SDK. Each is self-contained
(uses a temporary store) and asserts its own invariants, so the whole set doubles
as an end-to-end SDK smoke test.

| Example | Capability shown |
|---|---|
| [`customer_support.py`](customer_support.py) | **consolidation + provenance + time-travel + decay** — 100 conversations → one preference belief (confidence from evidence), traced to its 75 source events, with a week-1-vs-week-4 diff |
| [`sre_agent.py`](sre_agent.py) | **provenance "why?"** — `action → belief → observation` (the Python twin of `cargo xtask demo`) |
| [`personal_assistant.py`](personal_assistant.py) | **bitemporal versioning** — a revised belief; "what did you think my timezone was last month?" |
| [`knowledge_base.py`](knowledge_base.py) | **confidence decay + sources** — power-law forgetting; fresh vs 400-day-old facts; cited provenance |
| [`research_agent.py`](research_agent.py) | **transitive provenance** — a 5-hop chain from an action back to the original sources |

## Run

Build and install the SDK into a virtualenv, then run any example:

```bash
python3 -m venv .venv && source .venv/bin/activate
pip install maturin
(cd crates/engram-py && maturin develop --release --features python)

python examples/customer_support.py
```

All five run in well under a second. The SDK surface they use:

```python
import engram
mem = engram.Engram(path)
eid    = mem.record_event(agent, session, valid_ms, kind, payload, cause_ids=None)
bid    = mem.upsert_belief(agent, subject, predicate, obj, valid_from_ms, confidence,
                           provenance_ids=None, decay=None, decay_rate=None)
obj, c = mem.current_belief(subject, predicate)        # confidence decayed to now
past   = mem.belief_at(subject, predicate, at_ms)      # bitemporal time-travel
chain  = mem.provenance(memory_id)                     # transitive causes
```
