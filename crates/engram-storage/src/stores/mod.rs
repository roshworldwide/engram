//! The four native memory stores (R2), each built on the write-ahead log (R6,
//! durability) and the copy-on-write B-tree (R6, MVCC indexes).
//!
//! Landing order: `episodic` (2a) · `semantic` (2b) · `procedural` (2c) ·
//! `working` (2e). The causal-provenance DAG store (2d) lives alongside them.

pub mod episodic;
pub mod semantic;

pub use episodic::EpisodicStore;
pub use semantic::{BeliefInput, BeliefView, SemanticStore};
