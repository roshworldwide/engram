# Storage engine: WAL + CoW B-tree

Engram embeds **no** third-party storage engine — RocksDB, LMDB, SQLite, sled,
redb, and friends are banned in `deny.toml`. Everything below is written from
scratch in `engram-storage`.

## Write-ahead log

Durability comes first. Every mutation is appended to the WAL before it is
acknowledged.

- **Framing.** Each record is a length prefix + payload + **CRC32**. A torn or
  corrupt frame is detected on read; the scanner stops at the first bad frame.
- **Group commit.** The writer batches `fsync` on commit, so a burst of writes
  pays one flush — the basis of the 330K durable writes/s (P1).
- **Recovery.** `recover` replays only the **committed-transaction redo set**;
  an uncommitted (crashed) transaction's frames are ignored. `open` truncates a
  torn tail. The parent directory is fsync'd so the file's existence is durable.
- **Compaction.** `compact` writes an atomic checkpoint and swaps it in.

Recovering 1,000,000 entries takes **~128 ms** (P8, target < 2 s).

## Copy-on-write B-tree (MVCC)

The index is a **persistent** copy-on-write B-tree.

- **Path-copy writes.** An insert clones only the nodes on the touched
  root→leaf path; everything else is shared via `Arc`. The new root is published
  with a single atomic `arc-swap`.
- **Lock-free reads.** A reader loads the current root and walks an immutable
  tree — no locks, no readers-writer contention. Old roots stay alive as long as
  a snapshot holds them.
- **Snapshots = MVCC.** `snapshot()` pins a root; a snapshot taken before a write
  keeps seeing the old data while the writer advances. This is how every store
  gets multi-version concurrency control for free.
- **`floor(key)`.** Returns the greatest entry `≤ key` in `O(log n)` — the
  primitive behind bitemporal as-of queries (next chapter).

`get` is ~56 ns; 1,000,000 random inserts + a sorted scan complete in ~1.1 s. An
adversarial multi-agent review found the correctness/MVCC dimensions clean and
drove three fixes (WAL directory fsync, position-aware recovery, removal of a
hot-path `Arc` clone).

## How a store is built

Each store layers indexes (CoW B-trees) over the WAL. The episodic store, for
example, keeps four B-trees — primary, by-session, by-time, by-cause — over a
shared `Arc<EpisodicRecord>`, all updated within one transaction so reads are
MVCC-consistent per index. Cross-index atomic snapshots are provided by the ACC
layer.
