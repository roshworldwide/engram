//! Property tests proving (1a):
//! - every record type round-trips through the MessagePack codec, and
//! - decoding arbitrary bytes as any record type never panics (it errors).

use engram_core::{
    from_msgpack, AgentId, CausalEdge, DecayFunction, EdgeType, EpisodicRecord, EventType,
    MemoryId, ProceduralRecord, Record, SemanticRecord, SessionId, Timestamp, WorkingMemoryRecord,
};
use proptest::prelude::*;

fn arb_memory_id() -> impl Strategy<Value = MemoryId> {
    any::<u128>().prop_map(MemoryId)
}

fn arb_timestamp() -> impl Strategy<Value = Timestamp> {
    any::<i64>().prop_map(Timestamp)
}

fn arb_event_type() -> impl Strategy<Value = EventType> {
    prop_oneof![
        Just(EventType::ToolCall),
        Just(EventType::Message),
        Just(EventType::Observation),
        Just(EventType::Action),
    ]
}

fn arb_edge_type() -> impl Strategy<Value = EdgeType> {
    prop_oneof![
        Just(EdgeType::Created),
        Just(EdgeType::Updated),
        Just(EdgeType::Triggered),
        Just(EdgeType::Contradicted),
    ]
}

fn arb_decay() -> impl Strategy<Value = DecayFunction> {
    prop_oneof![
        (-5.0f32..5.0).prop_map(|lambda| DecayFunction::Exponential { lambda }),
        (0.0f32..5.0).prop_map(|beta| DecayFunction::PowerLaw { beta }),
        (any::<i64>(), 0.0f32..1.0).prop_map(|(d, c_low)| DecayFunction::Step {
            drop_at: Timestamp(d),
            c_low
        }),
        Just(DecayFunction::None),
    ]
}

fn arb_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..64)
}

fn arb_ids() -> impl Strategy<Value = Vec<MemoryId>> {
    prop::collection::vec(arb_memory_id(), 0..6)
}

fn arb_text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 _:./-]{0,24}"
}

prop_compose! {
    fn arb_episodic()(
        id in arb_memory_id(), agent in any::<u64>(), session in any::<u64>(),
        valid_time in arb_timestamp(), tx_time in arb_timestamp(),
        event_type in arb_event_type(), payload in arb_bytes(), cause_ids in arb_ids(),
    ) -> EpisodicRecord {
        EpisodicRecord {
            id, agent_id: AgentId(agent), session_id: SessionId(session),
            valid_time, tx_time, event_type, payload, cause_ids,
        }
    }
}

prop_compose! {
    fn arb_semantic()(
        id in arb_memory_id(), agent in any::<u64>(),
        subject in arb_text(), predicate in arb_text(), object in arb_bytes(),
        valid_from in arb_timestamp(), valid_until in prop::option::of(arb_timestamp()),
        tx_from in arb_timestamp(), tx_until in prop::option::of(arb_timestamp()),
        confidence_init in 0.0f32..1.0, decay_fn in arb_decay(), provenance_ids in arb_ids(),
    ) -> SemanticRecord {
        SemanticRecord {
            id, agent_id: AgentId(agent), subject, predicate, object,
            valid_from, valid_until, tx_from, tx_until, confidence_init, decay_fn, provenance_ids,
        }
    }
}

prop_compose! {
    fn arb_procedural()(
        id in arb_memory_id(), agent in any::<u64>(), name in arb_text(),
        version in any::<u32>(), steps in arb_bytes(),
        supersedes in prop::option::of(arb_memory_id()),
        valid_from in arb_timestamp(), tx_from in arb_timestamp(),
    ) -> ProceduralRecord {
        ProceduralRecord {
            id, agent_id: AgentId(agent), name, version, steps, supersedes, valid_from, tx_from,
        }
    }
}

prop_compose! {
    fn arb_working()(
        id in arb_memory_id(), agent in any::<u64>(), session in any::<u64>(),
        created in arb_timestamp(), payload in arb_bytes(),
    ) -> WorkingMemoryRecord {
        WorkingMemoryRecord {
            id, agent_id: AgentId(agent), session_id: SessionId(session), created, payload,
        }
    }
}

prop_compose! {
    fn arb_edge()(
        from_id in arb_memory_id(), to_id in arb_memory_id(), edge_type in arb_edge_type(),
    ) -> CausalEdge {
        CausalEdge { from_id, to_id, edge_type }
    }
}

/// Encode → decode must be the identity, via both the free functions and the
/// `Record` trait helpers.
fn assert_roundtrips<T>(value: &T)
where
    T: Record + PartialEq + std::fmt::Debug,
{
    let bytes = value.encode().expect("encode");
    let back = T::decode(&bytes).expect("decode");
    assert_eq!(value, &back);
    let back2: T = from_msgpack(&bytes).expect("free-fn decode");
    assert_eq!(value, &back2);
}

proptest! {
    #[test]
    fn episodic_round_trips(r in arb_episodic()) {
        assert_roundtrips(&r);
    }

    #[test]
    fn semantic_round_trips(r in arb_semantic()) {
        assert_roundtrips(&r);
    }

    #[test]
    fn procedural_round_trips(r in arb_procedural()) {
        assert_roundtrips(&r);
    }

    #[test]
    fn working_round_trips(r in arb_working()) {
        assert_roundtrips(&r);
    }

    #[test]
    fn edge_round_trips(r in arb_edge()) {
        assert_roundtrips(&r);
    }

    #[test]
    fn memory_id_string_round_trips(v in any::<u128>()) {
        let id = MemoryId(v);
        prop_assert_eq!(id, id.to_string().parse::<MemoryId>().unwrap());
    }

    /// Decoding arbitrary bytes must error, never panic. This is the in-process
    /// mirror of the `record_codec` fuzz target.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let _ = from_msgpack::<EpisodicRecord>(&bytes);
        let _ = from_msgpack::<SemanticRecord>(&bytes);
        let _ = from_msgpack::<ProceduralRecord>(&bytes);
        let _ = from_msgpack::<WorkingMemoryRecord>(&bytes);
        let _ = from_msgpack::<CausalEdge>(&bytes);
    }
}
