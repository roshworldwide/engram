//! The four native memory stores (R2), each built on the write-ahead log (R6,
//! durability) and the copy-on-write B-tree (R6, MVCC indexes).
//!
//! Landing order: `episodic` (2a) · `semantic` (2b) · `procedural` (2c) ·
//! `working` (2e). The causal-provenance DAG store (2d) lives alongside them.

pub(crate) mod common;

pub mod causal;
pub mod episodic;
pub mod procedural;
pub mod semantic;
pub mod working;

pub use causal::CausalDag;
pub use episodic::{EpisodicSnapshot, EpisodicStore};
pub use procedural::ProceduralStore;
pub use semantic::{BeliefInput, BeliefView, SemanticStore};
pub use working::{WorkingMemory, DEFAULT_CAPACITY};
