//! The write-ahead log (R6).
//!
//! An append-only, crash-safe log of operations. Each entry is framed as
//!
//! ```text
//! ┌───────────────┬──────────────────────────────────────┬────────────┐
//! │ region_len:u32│ region = [meta_len:u32][meta][record] │ crc32:u32  │
//! └───────────────┴──────────────────────────────────────┴────────────┘
//! ```
//!
//! where `meta` is MessagePack of `{lsn, tx_id, op}` and `record` is the raw
//! (already-serialized) payload — kept outside MessagePack so large records are
//! not re-encoded as integer arrays. The CRC32 covers `region`, so any torn or
//! corrupt frame is detected and treated as the end of the log.
//!
//! ## Durability & recovery
//!
//! Writes are buffered and made durable by [`Wal::commit`]/[`Wal::sync`], which
//! flush the buffer and `fsync` the file (batched group-commit: many
//! [`Wal::append`]s, one `fsync`). [`Wal::recover`] replays only entries whose
//! transaction has a durable `Commit` marker — the redo set — so a crash that
//! tore the tail mid-transaction loses exactly that transaction and nothing
//! committed before it.
//!
//! On Unix, [`Wal::create`] and [`Wal::compact`] additionally `fsync` the parent
//! directory so a newly created file or a compaction rename is durably linked (a
//! file `fsync` alone does not persist the directory entry). Recovery is
//! position-aware — a data entry is redone only if its transaction has a `Commit`
//! at a strictly greater LSN — so reusing a `tx_id` after it commits is safe
//! (the reused, uncommitted entries are simply dropped).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use engram_core::{from_msgpack, to_msgpack, RecordKind};

use crate::error::Result;

/// Magic bytes + format version written at the head of every WAL file.
const MAGIC: &[u8; 8] = b"ENGRMWAL";
const FORMAT_VERSION: u32 = 1;
const HEADER_LEN: u64 = 12; // MAGIC (8) + version (4)

/// Reject any frame claiming to be larger than this — protects recovery from
/// allocating gigabytes off a garbage length field. 256 MiB.
const MAX_REGION_LEN: u32 = 256 * 1024 * 1024;

/// The operation an entry records. `Put`/`Delete` carry the [`RecordKind`] so
/// replay can route the bytes to the right store; `Commit`/`Checkpoint` are
/// markers with an empty `record`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WalOp {
    /// Insert or update a record of the given kind.
    Put(RecordKind),
    /// Tombstone / retract a record of the given kind.
    Delete(RecordKind),
    /// Commit marker for a transaction — makes its `Put`/`Delete`s durable.
    Commit,
    /// Checkpoint marker — everything at or below this LSN is safe to compact.
    Checkpoint,
}

impl WalOp {
    /// Whether this op carries data that a store would apply on replay.
    #[must_use]
    pub const fn is_data(self) -> bool {
        matches!(self, WalOp::Put(_) | WalOp::Delete(_))
    }
}

/// A decoded WAL entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalEntry {
    /// Log sequence number (monotonic, gap-free per writer).
    pub lsn: u64,
    /// The transaction this entry belongs to.
    pub tx_id: u64,
    /// The operation.
    pub op: WalOp,
    /// The raw record payload (empty for markers).
    pub record: Vec<u8>,
    /// The CRC32 the frame was stored with (verified on read).
    pub checksum: u32,
}

/// The metadata half of a frame, MessagePack-encoded.
#[derive(Serialize, Deserialize)]
struct WalMeta {
    lsn: u64,
    tx_id: u64,
    op: WalOp,
}

/// The result of replaying a WAL.
#[derive(Debug, Default)]
pub struct Recovered {
    /// Committed data entries (`Put`/`Delete` whose tx has a `Commit`), in LSN order.
    pub entries: Vec<WalEntry>,
    /// The next LSN a writer should assign (one past the highest valid entry).
    pub next_lsn: u64,
    /// Byte length of the valid prefix (a torn tail beyond this is ignored).
    pub valid_len: u64,
    /// The highest `tx_id` seen across *all* valid frames (committed or not). A
    /// writer that resumes the log must assign tx ids strictly above this so it
    /// never reuses a tx id still present on disk (which position-aware redo
    /// would otherwise be vulnerable to).
    pub max_tx_id: u64,
}

/// The outcome of scanning a byte stream for frames.
struct Scan {
    entries: Vec<WalEntry>,
    valid_len: u64,
}

/// The append-only write-ahead log.
pub struct Wal {
    file: BufWriter<File>,
    path: PathBuf,
    next_lsn: u64,
}

impl Wal {
    /// Create a fresh WAL at `path`, truncating any existing file.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(MAGIC)?;
        writer.write_all(&FORMAT_VERSION.to_le_bytes())?;
        writer.flush()?; // make the (empty) WAL a valid file on disk immediately
        sync_parent_dir(&path)?; // durably link the new file into its directory
        Ok(Wal {
            file: writer,
            path,
            next_lsn: 0,
        })
    }

    /// Open an existing WAL for appending, recovering the next LSN and
    /// truncating any torn tail so new appends never follow garbage. Creates the
    /// file if it does not exist.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let len = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e.into()),
        };
        if len == 0 {
            return Self::create(path);
        }

        let scan = scan_reader(BufReader::new(File::open(&path)?))?;
        let next_lsn = scan
            .entries
            .iter()
            .map(|e| e.lsn)
            .max()
            .map_or(0, |m| m + 1);

        let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
        file.set_len(scan.valid_len)?; // drop the torn tail
        file.seek(SeekFrom::End(0))?;
        Ok(Wal {
            file: BufWriter::new(file),
            path,
            next_lsn,
        })
    }

    /// The LSN the next [`append`](Wal::append) will assign.
    #[must_use]
    pub fn next_lsn(&self) -> u64 {
        self.next_lsn
    }

    /// Append an operation, returning its LSN. The write is buffered; call
    /// [`commit`](Wal::commit) or [`sync`](Wal::sync) to make it durable.
    pub fn append(&mut self, tx_id: u64, op: WalOp, record: &[u8]) -> Result<u64> {
        let lsn = self.next_lsn;
        let region = build_region(lsn, tx_id, op, record)?;
        self.file.write_all(&frame_bytes(&region))?;
        self.next_lsn += 1;
        Ok(lsn)
    }

    /// Append a `Commit` marker for `tx_id` and make the transaction durable
    /// (flush + `fsync`). Returns the commit entry's LSN.
    pub fn commit(&mut self, tx_id: u64) -> Result<u64> {
        let lsn = self.append(tx_id, WalOp::Commit, &[])?;
        self.sync()?;
        Ok(lsn)
    }

    /// Append a `Checkpoint` marker and make it durable.
    pub fn checkpoint(&mut self) -> Result<u64> {
        let lsn = self.append(0, WalOp::Checkpoint, &[])?;
        self.sync()?;
        Ok(lsn)
    }

    /// Flush buffered appends to the OS without an `fsync` (not yet durable).
    pub fn flush(&mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }

    /// Flush and `fsync` — all prior appends are now durable.
    pub fn sync(&mut self) -> Result<()> {
        self.file.flush()?;
        self.file.get_ref().sync_data()?;
        Ok(())
    }

    /// Replay the WAL at `path`, returning the committed redo set.
    pub fn recover(path: impl AsRef<Path>) -> Result<Recovered> {
        let scan = scan_reader(BufReader::new(File::open(path.as_ref())?))?;
        let next_lsn = scan
            .entries
            .iter()
            .map(|e| e.lsn)
            .max()
            .map_or(0, |m| m + 1);
        // Position-aware redo: a data entry counts as committed only if a Commit
        // for the same tx_id exists at a strictly greater LSN. This stays correct
        // even if a tx_id is reused after committing — the reused, uncommitted
        // entries have no later Commit and are dropped.
        let mut commit_lsn: HashMap<u64, u64> = HashMap::new();
        for e in &scan.entries {
            if matches!(e.op, WalOp::Commit) {
                let slot = commit_lsn.entry(e.tx_id).or_insert(e.lsn);
                *slot = (*slot).max(e.lsn);
            }
        }
        let max_tx_id = scan.entries.iter().map(|e| e.tx_id).max().unwrap_or(0);
        let entries = scan
            .entries
            .into_iter()
            .filter(|e| e.op.is_data() && commit_lsn.get(&e.tx_id).is_some_and(|&c| c > e.lsn))
            .collect();
        Ok(Recovered {
            entries,
            next_lsn,
            valid_len: scan.valid_len,
            max_tx_id,
        })
    }

    /// Compact the log, discarding every valid frame with `lsn <= up_to_lsn`
    /// (its effects are assumed durable elsewhere — e.g. checkpointed into the
    /// B-tree). Rewrites atomically via a temp file + rename. Returns the number
    /// of frames retained. The writer is left positioned to append.
    pub fn compact(&mut self, up_to_lsn: u64) -> Result<usize> {
        self.sync()?;
        let scan = scan_reader(BufReader::new(File::open(&self.path)?))?;
        let keep: Vec<WalEntry> = scan
            .entries
            .into_iter()
            .filter(|e| e.lsn > up_to_lsn)
            .collect();

        let tmp = self.path.with_extension("compacting");
        {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            let mut writer = BufWriter::new(file);
            writer.write_all(MAGIC)?;
            writer.write_all(&FORMAT_VERSION.to_le_bytes())?;
            for e in &keep {
                let region = build_region(e.lsn, e.tx_id, e.op, &e.record)?;
                writer.write_all(&frame_bytes(&region))?;
            }
            writer.flush()?;
            writer.get_ref().sync_data()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        sync_parent_dir(&self.path)?; // make the rename durable

        let mut file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        file.seek(SeekFrom::End(0))?;
        self.file = BufWriter::new(file);
        Ok(keep.len())
    }
}

impl Drop for Wal {
    fn drop(&mut self) {
        // Best-effort: push buffered bytes to the OS so a forgotten flush does
        // not silently drop a tail. Durability still requires an explicit sync.
        let _ = self.file.flush();
    }
}

/// Fsync the directory containing `path` so a freshly created file or a rename
/// is durably linked. POSIX requires a directory `fsync` for the directory entry
/// to survive a crash — separate from fsyncing the file's contents.
#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(dir)?.sync_all()?;
    Ok(())
}

/// Non-Unix platforms expose no portable directory `fsync`; durability of the
/// directory entry is left to the filesystem.
#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> Result<()> {
    Ok(())
}

/// Encode the CRC-covered region `[meta_len][meta][record]`.
fn build_region(lsn: u64, tx_id: u64, op: WalOp, record: &[u8]) -> Result<Vec<u8>> {
    let meta = WalMeta { lsn, tx_id, op };
    let meta_bytes = to_msgpack(&meta)?;
    let mut region = Vec::with_capacity(4 + meta_bytes.len() + record.len());
    region.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    region.extend_from_slice(&meta_bytes);
    region.extend_from_slice(record);
    Ok(region)
}

/// Wrap a region in its length prefix and trailing CRC32.
fn frame_bytes(region: &[u8]) -> Vec<u8> {
    let crc = crc32fast::hash(region);
    let mut frame = Vec::with_capacity(8 + region.len());
    frame.extend_from_slice(&(region.len() as u32).to_le_bytes());
    frame.extend_from_slice(region);
    frame.extend_from_slice(&crc.to_le_bytes());
    frame
}

/// Read up to `buf.len()` bytes; returns how many were actually read. A short
/// return means EOF (a clean boundary if `0`, otherwise a torn tail).
fn fill(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Scan a byte stream for valid frames, stopping at the first torn/corrupt one.
/// Never panics on arbitrary input (this backs the `wal_reader` fuzz target).
fn scan_reader<R: Read>(mut reader: R) -> Result<Scan> {
    let mut entries = Vec::new();

    // Header. An empty stream is a valid empty WAL; a short/garbled header is
    // treated as "no valid frames".
    let mut header = [0u8; HEADER_LEN as usize];
    let got = fill(&mut reader, &mut header)?;
    if got == 0 {
        return Ok(Scan {
            entries,
            valid_len: 0,
        });
    }
    let version = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if got < header.len() || &header[..8] != MAGIC || version != FORMAT_VERSION {
        return Ok(Scan {
            entries,
            valid_len: 0,
        });
    }

    let mut valid_len = HEADER_LEN;
    let mut region: Vec<u8> = Vec::new();
    loop {
        // region_len
        let mut len_buf = [0u8; 4];
        let n = fill(&mut reader, &mut len_buf)?;
        if n < 4 {
            break; // clean EOF (n == 0) or torn length
        }
        let region_len = u32::from_le_bytes(len_buf);
        if !(4..=MAX_REGION_LEN).contains(&region_len) {
            break; // implausible length ⇒ corrupt/torn
        }

        // region
        region.clear();
        region.resize(region_len as usize, 0);
        let n = fill(&mut reader, &mut region)?;
        if n < region.len() {
            break; // torn region
        }

        // crc
        let mut crc_buf = [0u8; 4];
        let n = fill(&mut reader, &mut crc_buf)?;
        if n < 4 {
            break; // torn crc
        }
        let crc = u32::from_le_bytes(crc_buf);
        if crc32fast::hash(&region) != crc {
            break; // corrupt
        }

        // region = [meta_len:u32][meta][record]
        let meta_len = u32::from_le_bytes([region[0], region[1], region[2], region[3]]) as usize;
        if 4 + meta_len > region.len() {
            break; // corrupt framing
        }
        let meta: WalMeta = match from_msgpack(&region[4..4 + meta_len]) {
            Ok(m) => m,
            Err(_) => break, // corrupt metadata
        };
        let record = region[4 + meta_len..].to_vec();

        entries.push(WalEntry {
            lsn: meta.lsn,
            tx_id: meta.tx_id,
            op: meta.op,
            record,
            checksum: crc,
        });
        valid_len += 4 + u64::from(region_len) + 4;
    }

    Ok(Scan { entries, valid_len })
}

/// Scan an in-memory byte slice for valid frames (used by the `wal_reader`
/// fuzz target and property tests). Returns the entries up to the first
/// torn/corrupt frame; never panics.
pub fn scan_bytes(bytes: &[u8]) -> Vec<WalEntry> {
    scan_reader(bytes).map(|s| s.entries).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn wal_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wal");
        (dir, path)
    }

    const K: RecordKind = RecordKind::Episodic;

    #[test]
    fn committed_entries_round_trip() {
        let (_d, path) = wal_path();
        {
            let mut wal = Wal::create(&path).unwrap();
            for i in 0..100u64 {
                wal.append(7, WalOp::Put(K), format!("rec{i}").as_bytes())
                    .unwrap();
            }
            wal.commit(7).unwrap();
        }
        let recovered = Wal::recover(&path).unwrap();
        assert_eq!(recovered.entries.len(), 100);
        for (i, e) in recovered.entries.iter().enumerate() {
            assert_eq!(e.lsn, i as u64);
            assert_eq!(e.tx_id, 7);
            assert_eq!(e.op, WalOp::Put(K));
            assert_eq!(e.record, format!("rec{i}").into_bytes());
        }
        assert_eq!(recovered.next_lsn, 101); // 100 puts + 1 commit
    }

    #[test]
    fn reused_tx_id_after_commit_is_not_replayed() {
        let (_d, path) = wal_path();
        {
            let mut wal = Wal::create(&path).unwrap();
            wal.append(7, WalOp::Put(K), b"first").unwrap();
            wal.commit(7).unwrap(); // tx 7 committed
                                    // tx_id 7 reused for durable-but-uncommitted writes after its commit.
            wal.append(7, WalOp::Put(K), b"second").unwrap();
            wal.sync().unwrap();
        }
        let recovered = Wal::recover(&path).unwrap();
        let recs: Vec<&[u8]> = recovered
            .entries
            .iter()
            .map(|e| e.record.as_slice())
            .collect();
        assert_eq!(recs, vec![b"first".as_slice()]); // "second" is post-commit, not redone
    }

    #[test]
    fn uncommitted_transaction_is_not_recovered() {
        let (_d, path) = wal_path();
        {
            let mut wal = Wal::create(&path).unwrap();
            wal.append(1, WalOp::Put(K), b"committed").unwrap();
            wal.commit(1).unwrap();
            // tx 2 is appended + made durable, but never gets a Commit marker.
            wal.append(2, WalOp::Put(K), b"orphan").unwrap();
            wal.sync().unwrap();
        }
        let recovered = Wal::recover(&path).unwrap();
        assert_eq!(recovered.entries.len(), 1);
        assert_eq!(recovered.entries[0].record, b"committed");
    }

    #[test]
    fn torn_tail_is_skipped() {
        let (_d, path) = wal_path();
        {
            let mut wal = Wal::create(&path).unwrap();
            wal.append(1, WalOp::Put(K), b"a").unwrap();
            wal.append(1, WalOp::Put(K), b"b").unwrap();
            wal.commit(1).unwrap();
        }
        // Simulate a crash mid-write: a length prefix promising 64 bytes,
        // followed by only a few, then EOF.
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&64u32.to_le_bytes()).unwrap();
            f.write_all(b"partial").unwrap();
        }
        let recovered = Wal::recover(&path).unwrap();
        assert_eq!(recovered.entries.len(), 2);
        assert_eq!(recovered.entries[1].record, b"b");
    }

    #[test]
    fn crc_corruption_truncates_at_the_bad_frame() {
        let (_d, path) = wal_path();
        {
            let mut wal = Wal::create(&path).unwrap();
            wal.append(1, WalOp::Put(K), b"first").unwrap();
            wal.append(1, WalOp::Put(K), b"second").unwrap();
            wal.commit(1).unwrap();
        }
        // Flip a byte inside the first frame's region (just past the header).
        {
            let mut bytes = std::fs::read(&path).unwrap();
            let idx = HEADER_LEN as usize + 6;
            bytes[idx] ^= 0xFF;
            std::fs::write(&path, &bytes).unwrap();
        }
        let recovered = Wal::recover(&path).unwrap();
        // The corrupt first frame ends the scan, so nothing after it survives.
        assert!(recovered.entries.is_empty());
    }

    #[test]
    fn reopen_truncates_torn_tail_and_continues() {
        let (_d, path) = wal_path();
        {
            let mut wal = Wal::create(&path).unwrap();
            wal.append(1, WalOp::Put(K), b"x").unwrap();
            wal.commit(1).unwrap();
        }
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(&999u32.to_le_bytes()).unwrap();
            f.write_all(b"torn").unwrap();
        }
        // open() truncates the torn tail and resumes LSNs after the last good one.
        {
            let mut wal = Wal::open(&path).unwrap();
            assert_eq!(wal.next_lsn(), 2); // put(0) + commit(1)
            wal.append(3, WalOp::Put(K), b"y").unwrap();
            wal.commit(3).unwrap();
        }
        let recovered = Wal::recover(&path).unwrap();
        let records: Vec<&[u8]> = recovered
            .entries
            .iter()
            .map(|e| e.record.as_slice())
            .collect();
        assert_eq!(records, vec![b"x".as_slice(), b"y".as_slice()]);
    }

    #[test]
    fn compaction_drops_old_entries() {
        let (_d, path) = wal_path();
        let mut wal = Wal::create(&path).unwrap();
        let mut last = 0;
        for i in 0..10u64 {
            last = wal
                .append(1, WalOp::Put(K), format!("r{i}").as_bytes())
                .unwrap();
        }
        let commit_lsn = wal.commit(1).unwrap();
        // Keep only entries with lsn > last data lsn − 3 (i.e. the final few + commit).
        let threshold = last - 3;
        let kept = wal.compact(threshold).unwrap();
        // entries 7,8,9 (data) + the commit marker survive.
        assert_eq!(kept, 4);
        assert!(commit_lsn > last);

        let recovered = Wal::recover(&path).unwrap();
        let recs: Vec<Vec<u8>> = recovered.entries.iter().map(|e| e.record.clone()).collect();
        assert_eq!(recs, vec![b"r7".to_vec(), b"r8".to_vec(), b"r9".to_vec()]);

        // The writer still works after compaction and LSNs do not regress.
        let next = wal.next_lsn();
        let new_lsn = wal.append(2, WalOp::Put(K), b"after").unwrap();
        assert_eq!(new_lsn, next);
        wal.commit(2).unwrap();
    }

    #[test]
    fn empty_and_garbage_streams_never_panic() {
        assert!(scan_bytes(&[]).is_empty());
        assert!(scan_bytes(b"not a wal at all").is_empty());
        assert!(scan_bytes(MAGIC).is_empty());
        // Valid header, then a length prefix promising more than exists.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&1_000_000u32.to_le_bytes());
        bytes.extend_from_slice(b"short");
        assert!(scan_bytes(&bytes).is_empty());
    }
}
