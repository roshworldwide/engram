# Using Engram

Engram is reachable four ways, all over the same `engram-query::Engine`: the Rust
library, the `engram` CLI, a REST/gRPC server, and a Python SDK.

## The CLI

```bash
cargo run -p engram-cli -- <command>      # or install the `engram` binary
```

```text
engram init  <dir>
engram put   event  <dir> <session> <type> <payload>      # type: tool_call|message|observation|action
engram put   belief <dir> <subject> <predicate> <object> [confidence]
engram get   event  <dir> <id>
engram get   belief <dir> <subject> <predicate>
engram as-of <dir> <subject> <predicate> <ms>             # bitemporal time-travel
engram why   <dir> <id>                                   # trace provenance
engram demo                                               # the SRE provenance demo
engram bench                                              # → cargo xtask bench
```

Example session:

```console
$ engram init /tmp/mem
initialized engram store at /tmp/mem
$ engram put event /tmp/mem 7 observation "metric Y = 95%"
01KW4D2164JYJHC3WGRCCFSY89
$ engram put belief /tmp/mem service-x health unhealthy 0.92
01KW4D21N40000000000000001
$ engram get belief /tmp/mem service-x health
unhealthy (confidence 0.920)
```

Exit codes: `0` success, `2` usage error, `1` runtime error.

## The Rust library

```rust
use engram_query::Engine;
use engram_core::{AgentId, SessionId, Timestamp, EventType, DecayFunction};

let engine = Engine::open("/tmp/mem")?;
let obs = engine.record_event(AgentId(1), SessionId(7), Timestamp::from_millis(100),
    EventType::Observation, b"metric Y = 95%".to_vec(), vec![])?;
let belief = engine.upsert_belief(AgentId(1), "service-x".into(), "health".into(),
    b"unhealthy".to_vec(), Timestamp::from_millis(100), 0.92, DecayFunction::None, vec![obs])?;
let action = engine.record_event(AgentId(1), SessionId(7), Timestamp::from_millis(101),
    EventType::Action, b"restart service X".to_vec(), vec![belief])?;
assert_eq!(engine.provenance(action), vec![belief, obs]);   // why?
```

## REST & gRPC server

`engram-server` exposes the engine over HTTP (axum):

| Method & path | Action |
|---|---|
| `GET  /health` | liveness |
| `POST /memories/episodic` | record an event |
| `GET  /memories/episodic/{id}` | fetch an event |
| `POST /memories/semantic` | upsert a belief |
| `GET  /memories/semantic?subject=&predicate=` | current belief |
| `GET  /memories/provenance/{id}` | provenance chain |

A tonic **gRPC** surface is available behind the `grpc` feature (needs `protoc`).

## Python SDK

The `engram` wheel (built with maturin from `engram-py`, PyO3) mirrors the engine:

```python
import engram
mem = engram.Engram("/tmp/mem")
obs = mem.record_event(1, 7, 100, "observation", "metric Y = 95%")
belief = mem.upsert_belief(1, "service-x", "health", "unhealthy",
                           valid_from_ms=100, confidence=0.92, provenance_ids=[obs],
                           decay="exponential", decay_rate=1e-7)
print(mem.current_belief("service-x", "health"))   # ('unhealthy', ~0.92)
print(mem.provenance(belief))                        # [obs]
```

See `examples/` for five end-to-end example agents, including the customer-support
agent that learns preferences over 100 conversations and replays its memory across
time.
