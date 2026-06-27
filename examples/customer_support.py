"""Customer-support agent — the flagship Engram example.

Over 100 conversations the agent observes how the user reacts to its answers,
*consolidates* a durable preference belief from the repeated evidence (with full
provenance), then shows the four things that make Engram different from a database:

  1. consolidation   — repeated episodic evidence -> one semantic belief
  2. provenance       — the belief traces back to the exact events
  3. time-travel      — what did we believe in week 1 vs week 4?
  4. confidence decay — a belief fades without fresh evidence

Run:  python examples/customer_support.py
"""

import tempfile
import time

import engram

AGENT = 1
SESSION = 42
DAY_MS = 24 * 60 * 60 * 1000


def now_ms() -> int:
    return int(time.time() * 1000)


def main() -> None:
    with tempfile.TemporaryDirectory() as d:
        mem = engram.Engram(d)

        # --- 1. 100 conversations of episodic evidence over the last ~33 days
        # The user thumbs-up "concise" replies far more than "verbose" ones.
        start = now_ms() - 33 * DAY_MS
        concise_events = []
        for i in range(100):
            liked = "concise" if i % 4 != 0 else "verbose"  # 75% concise
            ts = start + i * (DAY_MS // 3)  # ~3 conversations/day
            eid = mem.record_event(
                AGENT, SESSION, ts, "observation",
                f"user_feedback\tpreferred_length\t{liked}",
            )
            if liked == "concise":
                concise_events.append(eid)
        print(f"recorded 100 conversations ({len(concise_events)} liked concise replies)")

        # --- 2. Consolidate the dominant signal into a belief --------------
        # Confidence grows with evidence: 1 - (1 - 0.1)^n, capped at 0.99 — the
        # same curve the Rust Consolidator uses. The belief decays slowly so it
        # fades if the evidence stops.
        n = len(concise_events)
        confidence = min(1.0 - (1.0 - 0.1) ** n, 0.99)
        belief = mem.upsert_belief(
            AGENT, "user", "preferred_length", "concise",
            valid_from_ms=start, confidence=confidence,
            provenance_ids=concise_events,
            decay="exponential", decay_rate=1e-7,
        )
        obj, conf = mem.current_belief("user", "preferred_length")
        print(f"\nconsolidated belief: user.preferred_length = {obj!r} "
              f"(confidence {conf:.3f}, decayed from c0={confidence:.3f} over ~33 days)")
        assert obj == "concise"

        # --- 3. Provenance: WHY do we believe this? -----------------------
        chain = mem.provenance(belief)
        assert sorted(chain) == sorted(concise_events), "belief must trace to its evidence"
        print(f"provenance: the belief traces back to exactly its {len(chain)} source events")
        print(f"   e.g. {chain[0]} … {chain[-1]}")

        # --- 4. Time-travel: belief in week 1 vs week 4 -------------------
        time.sleep(0.01)
        week4_tx = now_ms()           # a transaction-time checkpoint
        time.sleep(0.01)
        # Re-learn at week 4 with refreshed (higher) confidence; the old version
        # stays in history, queryable as-of `week4_tx`.
        mem.upsert_belief(
            AGENT, "user", "preferred_length", "concise",
            valid_from_ms=now_ms() - 5 * DAY_MS, confidence=0.95,
            decay="exponential", decay_rate=1e-7,
        )
        _, conf_now = mem.current_belief("user", "preferred_length")
        past = mem.belief_at("user", "preferred_length", week4_tx)
        print("\ntime-travel (transaction-time as-of):")
        print(f"   now          -> confidence {conf_now:.3f}  (refreshed version)")
        if past:
            print(f"   as-of week 4 -> confidence {past[1]:.3f}  (the earlier version)")

        # --- 5. Decay summary --------------------------------------------
        print("\nThe confidence you see is computed lazily on read: it reflects "
              "exponential decay\nsince the evidence was gathered, at zero background CPU cost.")
        print("\n[customer_support] done — consolidation · provenance · time-travel · decay.")


if __name__ == "__main__":
    main()
