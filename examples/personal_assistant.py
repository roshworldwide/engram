"""Personal assistant — beliefs that change over time, with time-travel.

The assistant learns facts about the user (timezone, dietary preference), then
*revises* one of them. Because every upsert appends a new version (it never
overwrites), the assistant can answer "what did you think my timezone was last
month?" — bitemporal time-travel.

Run:  python examples/personal_assistant.py
"""

import tempfile
import time

import engram

AGENT = 1
DAY_MS = 24 * 60 * 60 * 1000


def now_ms() -> int:
    return int(time.time() * 1000)


def main() -> None:
    with tempfile.TemporaryDirectory() as d:
        mem = engram.Engram(d)

        # Learn two stable facts.
        mem.upsert_belief(AGENT, "user", "diet", "vegetarian",
                          valid_from_ms=now_ms() - 60 * DAY_MS, confidence=0.9)
        mem.upsert_belief(AGENT, "user", "timezone", "America/New_York",
                          valid_from_ms=now_ms() - 60 * DAY_MS, confidence=0.95)
        print("learned: user.diet = vegetarian, user.timezone = America/New_York")

        # A month later, the user moves. Capture a checkpoint, then revise.
        time.sleep(0.01)
        last_month_tx = now_ms()
        time.sleep(0.01)
        mem.upsert_belief(AGENT, "user", "timezone", "Europe/Lisbon",
                          valid_from_ms=now_ms(), confidence=0.97)
        print("\nuser moved — revised: user.timezone = Europe/Lisbon")

        # Present vs the past.
        now_tz = mem.current_belief("user", "timezone")
        then_tz = mem.belief_at("user", "timezone", last_month_tx)
        print("\nwhat is the user's timezone?")
        print(f"   now        -> {now_tz[0]}")
        print(f"   last month -> {then_tz[0] if then_tz else '(unknown)'}")

        assert now_tz[0] == "Europe/Lisbon"
        assert then_tz is not None and then_tz[0] == "America/New_York"
        # The stable fact is unchanged.
        assert mem.current_belief("user", "diet")[0] == "vegetarian"
        print("\n[personal_assistant] done — beliefs are versioned and time-travelable.")


if __name__ == "__main__":
    main()
