//! MessagePack codec for records.
//!
//! Records serialize with `serde` + `rmp-serde` in **named** form (field names
//! preserved), so the on-disk format can evolve: new fields added with
//! `#[serde(default)]` decode against old data and vice versa. Each record type
//! also carries a stable [`RecordKind`] tag for WAL/B-tree framing (Phase 1b+).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{EngramError, Result};

/// A stable one-byte tag identifying a record type on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecordKind {
    /// [`crate::EpisodicRecord`]
    Episodic,
    /// [`crate::SemanticRecord`]
    Semantic,
    /// [`crate::ProceduralRecord`]
    Procedural,
    /// [`crate::WorkingMemoryRecord`]
    Working,
    /// [`crate::CausalEdge`]
    CausalEdge,
}

impl RecordKind {
    /// The stable wire tag for this kind.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            RecordKind::Episodic => 1,
            RecordKind::Semantic => 2,
            RecordKind::Procedural => 3,
            RecordKind::Working => 4,
            RecordKind::CausalEdge => 5,
        }
    }

    /// Recover a kind from its wire tag.
    #[must_use]
    pub const fn from_u8(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(RecordKind::Episodic),
            2 => Some(RecordKind::Semantic),
            3 => Some(RecordKind::Procedural),
            4 => Some(RecordKind::Working),
            5 => Some(RecordKind::CausalEdge),
            _ => None,
        }
    }
}

/// Serialize a value to a MessagePack byte vector (named fields).
pub fn to_msgpack<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    rmp_serde::to_vec_named(value).map_err(|e| EngramError::Encode(e.to_string()))
}

/// Deserialize a value from MessagePack bytes.
pub fn from_msgpack<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    rmp_serde::from_slice(bytes).map_err(|e| EngramError::Decode(e.to_string()))
}

/// A persisted record: serializable, self-describing via [`RecordKind`], with
/// convenience encode/decode helpers.
pub trait Record: Serialize + DeserializeOwned + Sized {
    /// The on-disk kind tag for this record type.
    const KIND: RecordKind;

    /// Encode to MessagePack bytes.
    fn encode(&self) -> Result<Vec<u8>> {
        to_msgpack(self)
    }

    /// Decode from MessagePack bytes.
    fn decode(bytes: &[u8]) -> Result<Self> {
        from_msgpack(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_kind_tags_round_trip() {
        for kind in [
            RecordKind::Episodic,
            RecordKind::Semantic,
            RecordKind::Procedural,
            RecordKind::Working,
            RecordKind::CausalEdge,
        ] {
            assert_eq!(RecordKind::from_u8(kind.as_u8()), Some(kind));
        }
        assert_eq!(RecordKind::from_u8(0), None);
        assert_eq!(RecordKind::from_u8(99), None);
    }
}
