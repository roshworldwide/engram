//! Property tests for WAL durability & recovery (1b):
//! - all-committed sequences round-trip in order,
//! - only committed transactions are recovered,
//! - truncating the file at any offset yields a safe prefix (never a corrupt or
//!   partial entry, never a panic),
//! - arbitrary bytes never panic the scanner / recovery.

use std::collections::HashSet;
use std::path::Path;

use engram_core::RecordKind;
use engram_storage::{scan_bytes, Wal, WalOp};
use proptest::prelude::*;
use tempfile::tempdir;

const PUT: WalOp = WalOp::Put(RecordKind::Episodic);

/// Write `entries` as `Put`s, then a `Commit` for every tx id in `commit`.
/// Uses buffered flush (not fsync) for speed — recovery reads the same bytes.
fn write_wal(path: &Path, entries: &[(u64, Vec<u8>)], commit: &HashSet<u64>) {
    let mut wal = Wal::create(path).unwrap();
    for (tx, rec) in entries {
        wal.append(*tx, PUT, rec).unwrap();
    }
    for tx in commit {
        wal.append(*tx, WalOp::Commit, &[]).unwrap();
    }
    wal.flush().unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Every entry round-trips, in append order, when its transaction commits.
    #[test]
    fn all_committed_round_trips(records in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..40), 0..40)) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("w.wal");
        let entries: Vec<(u64, Vec<u8>)> = records.iter().cloned().map(|r| (1u64, r)).collect();
        let mut commit = HashSet::new();
        commit.insert(1u64);
        write_wal(&path, &entries, &commit);

        let recovered = Wal::recover(&path).unwrap();
        prop_assert_eq!(recovered.entries.len(), records.len());
        for (got, want) in recovered.entries.iter().zip(records.iter()) {
            prop_assert_eq!(&got.record, want);
            prop_assert_eq!(got.op, PUT);
        }
    }

    /// Only entries whose transaction has a Commit marker are recovered.
    #[test]
    fn only_committed_transactions_recover(
        items in prop::collection::vec((0u64..4, prop::collection::vec(any::<u8>(), 0..16)), 0..40),
        flags in any::<[bool; 4]>(),
    ) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("w.wal");
        let committed: HashSet<u64> = (0u64..4).filter(|t| flags[*t as usize]).collect();
        write_wal(&path, &items, &committed);

        let recovered = Wal::recover(&path).unwrap();
        let expected: Vec<&Vec<u8>> = items
            .iter()
            .filter(|(tx, _)| committed.contains(tx))
            .map(|(_, r)| r)
            .collect();
        let got: Vec<&Vec<u8>> = recovered.entries.iter().map(|e| &e.record).collect();
        prop_assert_eq!(got, expected);
    }

    /// Truncating the log at any byte offset recovers a clean prefix and never
    /// panics or yields a corrupt entry. Each entry is its own committed tx, so
    /// the committed redo set is a prefix of the inputs.
    #[test]
    fn truncation_is_safe(
        records in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..24), 1..16),
        cut in 0usize..4096,
    ) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("full.wal");
        {
            let mut wal = Wal::create(&path).unwrap();
            for (i, r) in records.iter().enumerate() {
                wal.append(i as u64, PUT, r).unwrap();
                wal.append(i as u64, WalOp::Commit, &[]).unwrap();
            }
            wal.flush().unwrap();
        }
        let full = std::fs::read(&path).unwrap();
        let keep = cut.min(full.len());
        let cut_path = dir.path().join("cut.wal");
        std::fs::write(&cut_path, &full[..keep]).unwrap();

        let recovered = Wal::recover(&cut_path).unwrap();
        // Whatever survived must be an in-order prefix of the original records.
        prop_assert!(recovered.entries.len() <= records.len());
        for (got, want) in recovered.entries.iter().zip(records.iter()) {
            prop_assert_eq!(&got.record, want);
        }
    }

    /// The scanner never panics on arbitrary bytes (the in-process mirror of the
    /// `wal_reader` fuzz target).
    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = scan_bytes(&bytes);
        let dir = tempdir().unwrap();
        let path = dir.path().join("arb.wal");
        std::fs::write(&path, &bytes).unwrap();
        // recover() must return Ok/Err, never panic.
        let _ = Wal::recover(&path);
    }
}
