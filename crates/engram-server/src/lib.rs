#![forbid(unsafe_code)]
//! Engram REST surface (Phase 3c): an [`axum`] HTTP API over the
//! [`Engine`](engram_query::Engine).
//!
//! Endpoints:
//! - `GET  /health`
//! - `POST /memories/episodic` — record an event
//! - `GET  /memories/episodic/{id}` — fetch an event
//! - `POST /memories/semantic` — upsert a belief
//! - `GET  /memories/semantic?subject=…&predicate=…[&at_ms=…]` — current or
//!   time-travel belief
//! - `GET  /memories/provenance/{id}` — the causal-provenance chain of a memory
//!
//! Payloads/objects are treated as UTF-8 text at this surface (the engine stores
//! arbitrary bytes); ids are Crockford-base32 [`MemoryId`] strings.

use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use engram_core::{AgentId, DecayFunction, EventType, MemoryId, SessionId, Timestamp};
use engram_query::Engine;

/// Build the router over a shared engine.
pub fn router(engine: Arc<Engine>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/memories/episodic", post(post_episodic))
        .route("/memories/episodic/:id", get(get_episodic))
        .route("/memories/semantic", post(post_semantic))
        .route("/memories/semantic", get(get_semantic))
        .route("/memories/provenance/:id", get(get_provenance))
        .with_state(engine)
}

/// Serve the REST API on `addr` until shutdown.
///
/// # Errors
/// Returns an error if the listener cannot bind or the server loop fails.
pub async fn serve(engine: Arc<Engine>, addr: std::net::SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(engine)).await
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// REST error mapped to an HTTP status.
pub enum ApiError {
    /// 404
    NotFound,
    /// 400
    BadRequest(String),
    /// 500
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

impl From<engram_storage::StorageError> for ApiError {
    fn from(e: engram_storage::StorageError) -> Self {
        ApiError::Internal(e.to_string())
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

fn parse_id(s: &str) -> Result<MemoryId, ApiError> {
    MemoryId::from_str(s).map_err(|e| ApiError::BadRequest(format!("invalid id: {e}")))
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct IdResponse {
    id: MemoryId,
}

#[derive(Deserialize)]
struct EpisodicRequest {
    agent: u64,
    session: u64,
    valid_time_ms: i64,
    event_type: EventType,
    payload: String,
    #[serde(default)]
    cause_ids: Vec<MemoryId>,
}

#[derive(Serialize)]
struct EpisodicResponse {
    id: MemoryId,
    agent: u64,
    session: u64,
    valid_time_ms: i64,
    event_type: EventType,
    payload: String,
    cause_ids: Vec<MemoryId>,
}

fn decay_none() -> DecayFunction {
    DecayFunction::None
}

#[derive(Deserialize)]
struct SemanticRequest {
    agent: u64,
    subject: String,
    predicate: String,
    object: String,
    valid_from_ms: i64,
    confidence: f32,
    #[serde(default = "decay_none")]
    decay: DecayFunction,
    #[serde(default)]
    provenance_ids: Vec<MemoryId>,
}

#[derive(Deserialize)]
struct BeliefQuery {
    subject: String,
    predicate: String,
    at_ms: Option<i64>,
}

#[derive(Serialize)]
struct BeliefResponse {
    id: MemoryId,
    subject: String,
    predicate: String,
    object: String,
    confidence: f32,
    valid_from_ms: i64,
    tx_from_ms: i64,
}

#[derive(Serialize)]
struct ProvenanceResponse {
    id: MemoryId,
    provenance: Vec<MemoryId>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn post_episodic(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<EpisodicRequest>,
) -> Result<Json<IdResponse>, ApiError> {
    let id = engine.record_event(
        AgentId(req.agent),
        SessionId(req.session),
        Timestamp::from_millis(req.valid_time_ms),
        req.event_type,
        req.payload.into_bytes(),
        req.cause_ids,
    )?;
    Ok(Json(IdResponse { id }))
}

async fn get_episodic(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<EpisodicResponse>, ApiError> {
    let id = parse_id(&id)?;
    let rec = engine.get_event(id).ok_or(ApiError::NotFound)?;
    Ok(Json(EpisodicResponse {
        id: rec.id,
        agent: rec.agent_id.0,
        session: rec.session_id.0,
        valid_time_ms: rec.valid_time.as_millis(),
        event_type: rec.event_type,
        payload: String::from_utf8_lossy(&rec.payload).into_owned(),
        cause_ids: rec.cause_ids.clone(),
    }))
}

async fn post_semantic(
    State(engine): State<Arc<Engine>>,
    Json(req): Json<SemanticRequest>,
) -> Result<Json<IdResponse>, ApiError> {
    let id = engine.upsert_belief(
        AgentId(req.agent),
        req.subject,
        req.predicate,
        req.object.into_bytes(),
        Timestamp::from_millis(req.valid_from_ms),
        req.confidence,
        req.decay,
        req.provenance_ids,
    )?;
    Ok(Json(IdResponse { id }))
}

async fn get_semantic(
    State(engine): State<Arc<Engine>>,
    Query(q): Query<BeliefQuery>,
) -> Result<Json<BeliefResponse>, ApiError> {
    let view = match q.at_ms {
        Some(at) => engine.belief_at(&q.subject, &q.predicate, Timestamp::from_millis(at)),
        None => engine.current_belief(&q.subject, &q.predicate),
    }
    .ok_or(ApiError::NotFound)?;
    let rec = &view.record;
    Ok(Json(BeliefResponse {
        id: rec.id,
        subject: rec.subject.clone(),
        predicate: rec.predicate.clone(),
        object: String::from_utf8_lossy(&rec.object).into_owned(),
        confidence: view.confidence,
        valid_from_ms: rec.valid_from.as_millis(),
        tx_from_ms: rec.tx_from.as_millis(),
    }))
}

async fn get_provenance(
    State(engine): State<Arc<Engine>>,
    Path(id): Path<String>,
) -> Result<Json<ProvenanceResponse>, ApiError> {
    let id = parse_id(&id)?;
    Ok(Json(ProvenanceResponse {
        id,
        provenance: engine.provenance(id),
    }))
}
