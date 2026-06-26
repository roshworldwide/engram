//! Event and causal-edge taxonomies.

use serde::{Deserialize, Serialize};

/// The kind of thing an episodic memory records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    /// The agent invoked a tool.
    ToolCall,
    /// A message was sent or received.
    Message,
    /// The agent observed something about the world.
    Observation,
    /// The agent took an action.
    Action,
}

/// The relationship a causal-provenance edge represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeType {
    /// The target memory was created from the source.
    Created,
    /// The target updated a belief held by the source.
    Updated,
    /// The source triggered the target (e.g. an observation triggered an action).
    Triggered,
    /// The target contradicts the source.
    Contradicted,
}
