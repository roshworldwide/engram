//! Fuzz target (Q2): the WAL scanner must never panic on arbitrary bytes —
//! torn, truncated, or adversarial input yields entries up to the first bad
//! frame, but never crashes, hangs, or reads out of bounds.
//!
//! Run (nightly): `cargo +nightly fuzz run wal_reader`.
#![no_main]

use engram_storage::scan_bytes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = scan_bytes(data);
});
