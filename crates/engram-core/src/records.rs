//! The four native memory record types (R2) plus the causal edge (R5).
//!
//! These structs are the contract between the storage, query, and consistency
//! layers. They serialize with the MessagePack codec in [`crate::codec`].

use serde::{Deserialize, Serialize};

use crate::codec::{Record, RecordKind};
use crate::decay::DecayFunction;
use crate::event::{EdgeType, EventType};
use crate::ids::{AgentId, MemoryId, SessionId};
use crate::time::Timestamp;

/// An **episodic** memory: an immutable, append-only record of something that
/// happened (a tool call, message, observation, or action).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EpisodicRecord {
    /// Unique, time-sortable id.
    pub id: MemoryId,
    /// The agent that produced the event.
    pub agent_id: AgentId,
    /// The session the event belongs to.
    pub session_id: SessionId,
    /// When the event is considered to have occurred (valid-time).
    pub valid_time: Timestamp,
    /// When the event was recorded (transaction-time).
    pub tx_time: Timestamp,
    /// The kind of event.
    pub event_type: EventType,
    /// Opaque application payload.
    pub payload: Vec<u8>,
    /// Ids of the memories/events that caused this one (provenance).
    pub cause_ids: Vec<MemoryId>,
}

impl Record for EpisodicRecord {
    const KIND: RecordKind = RecordKind::Episodic;
}

/// A **semantic** memory: a mutable, bitemporal, versioned belief of the form
/// `subject predicate object`, carrying confidence and a decay curve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticRecord {
    /// Unique id of this belief version.
    pub id: MemoryId,
    /// The agent that holds the belief.
    pub agent_id: AgentId,
    /// Subject of the belief triple.
    pub subject: String,
    /// Predicate of the belief triple.
    pub predicate: String,
    /// Object of the belief triple (opaque bytes).
    pub object: Vec<u8>,
    /// Start of the belief's validity interval (valid-time).
    pub valid_from: Timestamp,
    /// End of the belief's validity interval, if closed.
    pub valid_until: Option<Timestamp>,
    /// When this version was recorded (transaction-time).
    pub tx_from: Timestamp,
    /// When this version was superseded, if it has been.
    pub tx_until: Option<Timestamp>,
    /// Confidence at `valid_from`, before decay.
    pub confidence_init: f32,
    /// How confidence decays after `valid_from`.
    pub decay_fn: DecayFunction,
    /// Ids of memories that justify this belief (provenance).
    pub provenance_ids: Vec<MemoryId>,
}

impl Record for SemanticRecord {
    const KIND: RecordKind = RecordKind::Semantic;
}

/// A **procedural** memory: a versioned skill. Newer versions `supersede` older
/// ones, forming a chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProceduralRecord {
    /// Unique id of this skill version.
    pub id: MemoryId,
    /// The agent that owns the skill.
    pub agent_id: AgentId,
    /// Human-readable skill name.
    pub name: String,
    /// Monotonically increasing version number.
    pub version: u32,
    /// Opaque encoded steps of the procedure.
    pub steps: Vec<u8>,
    /// The previous version this one replaces, if any.
    pub supersedes: Option<MemoryId>,
    /// Start of validity (valid-time).
    pub valid_from: Timestamp,
    /// When this version was recorded (transaction-time).
    pub tx_from: Timestamp,
}

impl Record for ProceduralRecord {
    const KIND: RecordKind = RecordKind::Procedural;
}

/// A **working** memory: a bounded scratchpad entry. The store keeps a bounded
/// FIFO (default cap 50); overflow triggers consolidation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkingMemoryRecord {
    /// Unique id.
    pub id: MemoryId,
    /// The agent the scratchpad belongs to.
    pub agent_id: AgentId,
    /// The session the entry belongs to.
    pub session_id: SessionId,
    /// When the entry was created.
    pub created: Timestamp,
    /// Opaque payload.
    pub payload: Vec<u8>,
}

impl Record for WorkingMemoryRecord {
    const KIND: RecordKind = RecordKind::Working;
}

/// An edge in the causal-provenance DAG (R5): `from_id` caused `to_id` with the
/// given relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CausalEdge {
    /// The cause.
    pub from_id: MemoryId,
    /// The effect.
    pub to_id: MemoryId,
    /// The relationship type.
    pub edge_type: EdgeType,
}

impl Record for CausalEdge {
    const KIND: RecordKind = RecordKind::CausalEdge;
}
