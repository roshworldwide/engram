#![forbid(unsafe_code)]
//! # engram-core
//!
//! Shared core types for the Engram engine: identifiers ([`MemoryId`] et al.),
//! [`Timestamp`] and the injectable [`Clock`]/[`Rng`] traits, the [`EventType`]/
//! [`EdgeType`] taxonomies, the lazy [`DecayFunction`] (R4), the four memory
//! [`records`] plus [`CausalEdge`] (R2/R5), the MessagePack [`codec`], and the
//! workspace [`EngramError`].
//!
//! Everything here is deliberately small and dependency-light — it is the
//! contract the storage, query, and consistency layers build on.
//!
//! ```
//! use engram_core::{DecayFunction, EpisodicRecord, EventType, Record, MemoryId,
//!     AgentId, SessionId, Timestamp};
//!
//! let ev = EpisodicRecord {
//!     id: MemoryId::from_parts(1_700_000_000_000, 1),
//!     agent_id: AgentId(1),
//!     session_id: SessionId(1),
//!     valid_time: Timestamp::from_millis(1_700_000_000_000),
//!     tx_time: Timestamp::from_millis(1_700_000_000_001),
//!     event_type: EventType::Observation,
//!     payload: b"metric Y = 95%".to_vec(),
//!     cause_ids: vec![],
//! };
//! let bytes = ev.encode().unwrap();
//! assert_eq!(EpisodicRecord::decode(&bytes).unwrap(), ev);
//! assert!(DecayFunction::Exponential { lambda: 1e-6 }.eval(0.9, 0) == 0.9);
//! ```

pub mod codec;
pub mod decay;
pub mod error;
pub mod event;
pub mod ids;
pub mod records;
pub mod rng;
pub mod time;

pub use codec::{from_msgpack, to_msgpack, Record, RecordKind};
pub use decay::DecayFunction;
pub use error::{EngramError, IdParseError, Result};
pub use event::{EdgeType, EventType};
pub use ids::{AgentId, AgentInstanceId, MemoryId, MemoryIdGenerator, SessionId};
pub use records::{
    CausalEdge, EpisodicRecord, ProceduralRecord, SemanticRecord, WorkingMemoryRecord,
};
pub use rng::{Rng, SplitMix64, SystemRng};
pub use time::{Clock, MockClock, SystemClock, Timestamp};

/// The semantic version of the Engram core crate.
///
/// ```
/// assert!(!engram_core::version().is_empty());
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
