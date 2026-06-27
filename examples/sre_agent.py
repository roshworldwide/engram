"""SRE agent — the provenance "why?" demo, in Python.

Stores observations (episodic), an inferred state (semantic), and an action,
then answers *"why did the agent restart service X?"* by tracing the
causal-provenance DAG. This is the Python twin of `cargo xtask demo`.

Run:  python examples/sre_agent.py
"""

import tempfile

import engram

AGENT, SESSION = 1, 7


def main() -> None:
    with tempfile.TemporaryDirectory() as d:
        mem = engram.Engram(d)
        labels = {}

        # Episodic observations: a calm baseline, then the alert.
        mem.record_event(AGENT, SESSION, 10, "observation", "metric Y = 40% (ok)")
        alert = mem.record_event(
            AGENT, SESSION, 100, "observation", "metric Y = 95% > threshold Z = 80%"
        )
        labels[alert] = "[episodic] observed: metric Y = 95% > threshold Z = 80%"

        # Semantic inference, derived from the alert.
        belief = mem.upsert_belief(
            AGENT, "service-x", "health", "unhealthy",
            valid_from_ms=100, confidence=0.92, provenance_ids=[alert],
        )
        labels[belief] = "[semantic] service-x.health = unhealthy"

        # The action, caused by the belief.
        action = mem.record_event(
            AGENT, SESSION, 101, "action", "restart service X", cause_ids=[belief]
        )

        print("Q: why did the agent restart service X?")
        for node in mem.provenance(action):
            print(f"   └─ {labels.get(node, node)}")
        print("\nA: it observed metric Y exceed threshold Z — full chain above.")

        chain = mem.provenance(action)
        assert belief in chain and alert in chain, "action must trace to belief and alert"


if __name__ == "__main__":
    main()
