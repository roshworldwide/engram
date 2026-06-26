//! Fuzz target (Q2): decoding arbitrary bytes as any record type must never
//! panic — it must return `Ok` or `Err`, but never crash, hang, or read OOB.
//!
//! Run (nightly): `cargo +nightly fuzz run record_codec`.
#![no_main]

use engram_core::{
    from_msgpack, CausalEdge, EpisodicRecord, ProceduralRecord, SemanticRecord, WorkingMemoryRecord,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Attempt every record type against the same arbitrary bytes.
    let _ = from_msgpack::<EpisodicRecord>(data);
    let _ = from_msgpack::<SemanticRecord>(data);
    let _ = from_msgpack::<ProceduralRecord>(data);
    let _ = from_msgpack::<WorkingMemoryRecord>(data);
    let _ = from_msgpack::<CausalEdge>(data);
});
