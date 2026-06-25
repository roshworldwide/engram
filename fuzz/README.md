# Fuzzing

`cargo-fuzz` (libFuzzer) targets. This is a **separate nightly workspace** (excluded from the root workspace
in `Cargo.toml`) because libFuzzer requires a nightly toolchain.

Planned targets (Q2 — ≥ 10,000,000 iterations each, zero crashes):

- `record_codec` — round-trip + arbitrary-bytes decode of every record type (Phase 1a)
- `wal_reader` — torn/garbage WAL entries must never panic; valid prefixes recover (Phase 1b)
- `btree_ops` — randomized insert/get/range sequences preserve sortedness & snapshots (Phase 1c)
- `dag_decode` — arbitrary adjacency/edge bytes decode safely; cycles rejected (Phase 2d)

Initialize with `cargo fuzz init` when the first target lands. CI builds the targets on every push
(`fuzz-smoke` job) and runs the full 10M-iteration campaign in a nightly job.
