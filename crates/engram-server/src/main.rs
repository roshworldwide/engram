#![forbid(unsafe_code)]
//! The `engram-server` binary: opens an engine over a data directory and serves
//! the REST API.
//!
//! Usage: `engram-server [DATA_DIR]` (default `engram-data`); address via
//! `ENGRAM_ADDR` (default `127.0.0.1:7777`).

use std::sync::Arc;

use engram_query::Engine;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "engram-data".to_string());
    let addr: std::net::SocketAddr = std::env::var("ENGRAM_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:7777".to_string())
        .parse()?;
    let engine = Arc::new(Engine::open(&dir)?);
    println!("engram-server listening on http://{addr} (data dir: {dir})");
    engram_server::serve(engine, addr).await?;
    Ok(())
}
