"""Python integration test for the Engram SDK (3d). Run via maturin:

    maturin develop --features python   # from crates/engram-py with a venv active
    python tests/test_engram.py
"""

import tempfile

import engram


def test_sre_flow():
    mem = engram.Engram(tempfile.mkdtemp())

    obs = mem.record_event(
        agent=1, session=1, valid_time_ms=1000,
        event_type="Observation", payload="metric Y = 95% > 80%",
    )
    act = mem.record_event(
        agent=1, session=1, valid_time_ms=1001,
        event_type="Action", payload="restart service X", cause_ids=[obs],
    )
    bel = mem.upsert_belief(
        agent=1, subject="service-x", predicate="health", object="unhealthy",
        valid_from_ms=1000, confidence=0.9, provenance_ids=[obs],
    )

    # Reads.
    obj, conf = mem.current_belief("service-x", "health")
    assert obj == "unhealthy"
    assert 0.0 < conf <= 0.9
    assert mem.current_belief("service-x", "missing") is None

    # Provenance: why did the agent act / hold the belief? -> the observation.
    assert mem.provenance(act) == [obs]
    assert mem.provenance(bel) == [obs]

    # Bad id raises ValueError.
    try:
        mem.provenance("not-an-id")
        raise AssertionError("expected ValueError")
    except ValueError:
        pass


if __name__ == "__main__":
    test_sre_flow()
    print(f"engram {engram.__version__}: python SDK test passed")
