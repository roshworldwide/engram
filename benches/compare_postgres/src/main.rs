//! Q4 — Engram bitemporal time-travel vs. a hand-built PostgreSQL bitemporal
//! schema. Both answer the SAME query: *"what was believed for (subject,
//! predicate) as-of transaction time T"* against a belief with 200 versions.
//!
//! Engram runs in-process (a `floor` over the version index); PostgreSQL runs the
//! equivalent indexed SQL over its client/server protocol — which is the honest,
//! realistic comparison (an agent's memory is in-process; a Postgres-backed
//! memory is over a socket). Set `DATABASE_URL` (libpq form) or rely on the
//! default `host=localhost user=postgres dbname=postgres`.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use engram_core::{AgentId, DecayFunction, MockClock, Timestamp};
use engram_storage::{BeliefInput, SemanticStore};

const VERSIONS: i64 = 200;
const DEEP: i64 = 2; // the 3rd version — deep historical point
const ENGRAM_ITERS: u32 = 2_000_000;
const PG_ITERS: u32 = 20_000;

fn main() -> Result<()> {
    let engram_ns = bench_engram()?;
    let pg_ns = bench_postgres()?;

    println!("\n──────── Q4: bitemporal time-travel, 200 versions ────────");
    println!("  engram   get_at_tx        : {engram_ns:>10.1} ns/query  (in-process)");
    println!("  postgres bitemporal SELECT: {pg_ns:>10.1} ns/query  (client/server)");
    println!("  Engram is {:.1}x faster", pg_ns / engram_ns);
    Ok(())
}

/// Build 200 versions in Engram and time the as-of query.
fn bench_engram() -> Result<f64> {
    let dir = tempfile::tempdir()?;
    let clock = Arc::new(MockClock::new(Timestamp(1_000_000_000)));
    let dyn_clock: Arc<dyn engram_core::Clock> = clock.clone();
    let store = SemanticStore::create_with_clock(dir.path().join("s.wal"), dyn_clock)?;

    let mut tx_points = Vec::new();
    for v in 0..VERSIONS {
        let id = store.upsert_belief(BeliefInput {
            agent_id: AgentId(1),
            subject: "user".into(),
            predicate: "pref".into(),
            object: format!("v{v}").into_bytes(),
            valid_from: Timestamp(0),
            confidence_init: 1.0,
            decay_fn: DecayFunction::None,
            provenance_ids: vec![],
        })?;
        tx_points.push(store.get_by_id(id).unwrap().tx_from);
        clock.advance(1_000_000); // +1 ms between versions
    }
    store.commit()?;
    let deep_tx = tx_points[DEEP as usize];

    // Warm, then time.
    assert!(store.get_at_tx("user", "pref", deep_tx).is_some());
    let start = Instant::now();
    let mut hits = 0u64;
    for _ in 0..ENGRAM_ITERS {
        if store.get_at_tx("user", "pref", deep_tx).is_some() {
            hits += 1;
        }
    }
    let ns = start.elapsed().as_nanos() as f64 / f64::from(ENGRAM_ITERS);
    println!("engram:   {ns:.1} ns/query over {ENGRAM_ITERS} iters ({hits} hits)");
    Ok(ns)
}

/// Build the equivalent 200-version bitemporal table in PostgreSQL and time the
/// same as-of query (indexed).
fn bench_postgres() -> Result<f64> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "host=localhost user=postgres dbname=postgres".to_string());
    let mut client = postgres::Client::connect(&url, postgres::NoTls)?;

    client.batch_execute(
        "DROP TABLE IF EXISTS beliefs;
         CREATE TABLE beliefs (
             subject text, predicate text, object text,
             tx_from bigint, tx_until bigint);",
    )?;
    for v in 0..VERSIONS {
        let tx_until: Option<i64> = if v == VERSIONS - 1 { None } else { Some(v + 1) };
        client.execute(
            "INSERT INTO beliefs VALUES ('user','pref',$1,$2,$3)",
            &[&format!("v{v}"), &v, &tx_until],
        )?;
    }
    client.batch_execute(
        "CREATE INDEX idx_beliefs ON beliefs (subject, predicate, tx_from);
         ANALYZE beliefs;",
    )?;

    let stmt = client.prepare(
        "SELECT object FROM beliefs
         WHERE subject = $1 AND predicate = $2 AND tx_from <= $3
           AND (tx_until IS NULL OR tx_until > $3)
         ORDER BY tx_from DESC LIMIT 1",
    )?;

    // Warm, then time.
    let _: Option<String> = client
        .query_opt(&stmt, &[&"user", &"pref", &DEEP])?
        .map(|r| r.get(0));
    let start = Instant::now();
    let mut hits = 0u64;
    for _ in 0..PG_ITERS {
        if client.query_opt(&stmt, &[&"user", &"pref", &DEEP])?.is_some() {
            hits += 1;
        }
    }
    let ns = start.elapsed().as_nanos() as f64 / f64::from(PG_ITERS);
    println!("postgres: {ns:.1} ns/query over {PG_ITERS} iters ({hits} hits)");
    Ok(ns)
}
