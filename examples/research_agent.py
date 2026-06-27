"""Research agent — a multi-hop provenance chain across a reasoning session.

The agent reads sources, forms an intermediate belief, uses it (plus another
observation) to reach a conclusion, then takes an action. Asking "why?" walks the
whole causal chain back to the original sources — provenance is transitive.

Run:  python examples/research_agent.py
"""

import tempfile

import engram

AGENT, SESSION = 1, 5


def main() -> None:
    with tempfile.TemporaryDirectory() as d:
        mem = engram.Engram(d)
        label = {}

        # 1. Read two sources.
        paper = mem.record_event(AGENT, SESSION, 1, "message", "paper: latency up 3x")
        metric = mem.record_event(AGENT, SESSION, 2, "observation", "p99 = 900ms")
        label[paper] = "[episodic] source paper: latency up 3x"
        label[metric] = "[episodic] observed p99 = 900ms"

        # 2. Intermediate belief from the sources.
        diagnosis = mem.upsert_belief(
            AGENT, "service", "bottleneck", "db-connection-pool",
            valid_from_ms=2, confidence=0.8, provenance_ids=[paper, metric],
        )
        label[diagnosis] = "[semantic] service.bottleneck = db-connection-pool"

        # 3. Conclusion built on the intermediate belief + a fresh check.
        check = mem.record_event(AGENT, SESSION, 3, "observation",
                                 "pool exhausted 40% of the time", cause_ids=[diagnosis])
        label[check] = "[episodic] observed pool exhausted 40%"
        plan = mem.upsert_belief(
            AGENT, "service", "fix", "raise-pool-size",
            valid_from_ms=3, confidence=0.85, provenance_ids=[diagnosis, check],
        )
        label[plan] = "[semantic] service.fix = raise-pool-size"

        # 4. Act on the conclusion.
        action = mem.record_event(AGENT, SESSION, 4, "action",
                                  "raise pool size to 64", cause_ids=[plan])

        print("Q: why did the research agent raise the pool size?")
        chain = mem.provenance(action)
        for node in chain:
            print(f"   └─ {label.get(node, node)}")

        # The action transitively depends on the original two sources.
        assert paper in chain and metric in chain, "should trace to original sources"
        assert plan in chain and diagnosis in chain
        print(f"\nthe chain is {len(chain)} hops deep and reaches both original sources.")
        print("\n[research_agent] done — transitive, multi-hop provenance.")


if __name__ == "__main__":
    main()
