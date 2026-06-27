"""Knowledge-base agent — beliefs with confidence and source provenance, that
fade unless refreshed.

The agent ingests facts from sources (episodic notes) and forms beliefs that cite
those sources. Each belief decays (power-law forgetting), so stale knowledge loses
confidence over time — and you can always ask which sources a fact came from.

Run:  python examples/knowledge_base.py
"""

import tempfile
import time

import engram

AGENT, SESSION = 1, 3
DAY_MS = 24 * 60 * 60 * 1000


def now_ms() -> int:
    return int(time.time() * 1000)


def main() -> None:
    with tempfile.TemporaryDirectory() as d:
        mem = engram.Engram(d)

        # Two sources support a fresh fact; one source supports a stale one.
        s1 = mem.record_event(AGENT, SESSION, now_ms(), "message", "doc:rfc-9999 §2")
        s2 = mem.record_event(AGENT, SESSION, now_ms(), "message", "doc:wiki/quic")
        fresh = mem.upsert_belief(
            AGENT, "quic", "transport", "udp-based",
            valid_from_ms=now_ms(), confidence=0.9, provenance_ids=[s1, s2],
            decay="power_law", decay_rate=0.3,
        )

        s3 = mem.record_event(AGENT, SESSION, now_ms() - 400 * DAY_MS,
                              "message", "doc:old-notes")
        mem.upsert_belief(
            AGENT, "http2", "status", "cutting-edge",
            valid_from_ms=now_ms() - 400 * DAY_MS, confidence=0.9,
            provenance_ids=[s3], decay="power_law", decay_rate=0.3,
        )

        fresh_obj, fresh_conf = mem.current_belief("quic", "transport")
        stale_obj, stale_conf = mem.current_belief("http2", "status")
        print("knowledge base (confidence reflects power-law forgetting):")
        print(f"   quic.transport = {fresh_obj!r}   confidence {fresh_conf:.3f}  (fresh)")
        print(f"   http2.status   = {stale_obj!r}   confidence {stale_conf:.3f}  (400 days old)")

        print("\nwhere did 'quic.transport' come from?")
        for src in mem.provenance(fresh):
            print(f"   └─ source {src}")

        assert fresh_conf > stale_conf, "older knowledge should have decayed further"
        assert sorted(mem.provenance(fresh)) == sorted([s1, s2])
        print("\n[knowledge_base] done — cited, confidence-weighted, fading knowledge.")


if __name__ == "__main__":
    main()
