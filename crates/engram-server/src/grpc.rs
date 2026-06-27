//! Engram gRPC surface (feature `grpc`): the same operations as REST, over
//! [`tonic`]. The Rust types are generated from `proto/engram.proto` at build
//! time (requires `protoc`).
//!
//! `tonic::Status` is a large error type by design, so the `result_large_err`
//! lint is allowed for this whole (gRPC-shaped) module.
#![allow(clippy::result_large_err)]

use std::str::FromStr;
use std::sync::Arc;

use tonic::{Request, Response, Status};

use engram_core::{AgentId, DecayFunction, EventType, MemoryId, SessionId, Timestamp};
use engram_query::Engine;

/// The generated protobuf types and service stubs.
pub mod proto {
    tonic::include_proto!("engram");
}

use proto::engram_server::{Engram, EngramServer};
use proto::{
    BeliefQuery, BeliefResponse, EpisodicRequest, IdResponse, ProvenanceRequest,
    ProvenanceResponse, SemanticRequest,
};

/// The gRPC service over a shared [`Engine`].
pub struct GrpcService {
    engine: Arc<Engine>,
}

impl GrpcService {
    /// Wrap an engine.
    #[must_use]
    pub fn new(engine: Arc<Engine>) -> Self {
        GrpcService { engine }
    }
}

/// Build a tonic server for the Engram gRPC service.
pub fn service(engine: Arc<Engine>) -> EngramServer<GrpcService> {
    EngramServer::new(GrpcService::new(engine))
}

fn parse_event_type(s: &str) -> Result<EventType, Status> {
    match s {
        "ToolCall" => Ok(EventType::ToolCall),
        "Message" => Ok(EventType::Message),
        "Observation" => Ok(EventType::Observation),
        "Action" => Ok(EventType::Action),
        other => Err(Status::invalid_argument(format!(
            "unknown event_type: {other}"
        ))),
    }
}

fn parse_id(s: &str) -> Result<MemoryId, Status> {
    MemoryId::from_str(s).map_err(|e| Status::invalid_argument(format!("invalid id: {e}")))
}

fn parse_ids(ids: &[String]) -> Result<Vec<MemoryId>, Status> {
    ids.iter().map(|s| parse_id(s)).collect()
}

fn internal(e: impl std::fmt::Display) -> Status {
    Status::internal(e.to_string())
}

#[tonic::async_trait]
impl Engram for GrpcService {
    async fn record_episodic(
        &self,
        request: Request<EpisodicRequest>,
    ) -> Result<Response<IdResponse>, Status> {
        let r = request.into_inner();
        let event_type = parse_event_type(&r.event_type)?;
        let causes = parse_ids(&r.cause_ids)?;
        let id = self
            .engine
            .record_event(
                AgentId(r.agent),
                SessionId(r.session),
                Timestamp::from_millis(r.valid_time_ms),
                event_type,
                r.payload,
                causes,
            )
            .map_err(internal)?;
        Ok(Response::new(IdResponse { id: id.to_string() }))
    }

    async fn upsert_semantic(
        &self,
        request: Request<SemanticRequest>,
    ) -> Result<Response<IdResponse>, Status> {
        let r = request.into_inner();
        let provenance = parse_ids(&r.provenance_ids)?;
        let id = self
            .engine
            .upsert_belief(
                AgentId(r.agent),
                r.subject,
                r.predicate,
                r.object,
                Timestamp::from_millis(r.valid_from_ms),
                r.confidence,
                DecayFunction::None,
                provenance,
            )
            .map_err(internal)?;
        Ok(Response::new(IdResponse { id: id.to_string() }))
    }

    async fn get_semantic(
        &self,
        request: Request<BeliefQuery>,
    ) -> Result<Response<BeliefResponse>, Status> {
        let r = request.into_inner();
        let view = match r.at_ms {
            Some(at) => self
                .engine
                .belief_at(&r.subject, &r.predicate, Timestamp::from_millis(at)),
            None => self.engine.current_belief(&r.subject, &r.predicate),
        }
        .ok_or_else(|| Status::not_found("belief not found"))?;
        let rec = &view.record;
        Ok(Response::new(BeliefResponse {
            id: rec.id.to_string(),
            subject: rec.subject.clone(),
            predicate: rec.predicate.clone(),
            object: rec.object.clone(),
            confidence: view.confidence,
            valid_from_ms: rec.valid_from.as_millis(),
            tx_from_ms: rec.tx_from.as_millis(),
        }))
    }

    async fn get_provenance(
        &self,
        request: Request<ProvenanceRequest>,
    ) -> Result<Response<ProvenanceResponse>, Status> {
        let id = parse_id(&request.into_inner().id)?;
        let provenance = self
            .engine
            .provenance(id)
            .iter()
            .map(ToString::to_string)
            .collect();
        Ok(Response::new(ProvenanceResponse { provenance }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn grpc_record_action_and_trace_provenance() {
        let dir = tempdir().unwrap();
        let engine = Arc::new(Engine::open(dir.path()).unwrap());
        let svc = GrpcService::new(engine);

        let obs = svc
            .record_episodic(Request::new(EpisodicRequest {
                agent: 1,
                session: 1,
                valid_time_ms: 1000,
                event_type: "Observation".into(),
                payload: b"metric Y high".to_vec(),
                cause_ids: vec![],
            }))
            .await
            .unwrap()
            .into_inner()
            .id;

        let action = svc
            .record_episodic(Request::new(EpisodicRequest {
                agent: 1,
                session: 1,
                valid_time_ms: 1001,
                event_type: "Action".into(),
                payload: b"restart X".to_vec(),
                cause_ids: vec![obs.clone()],
            }))
            .await
            .unwrap()
            .into_inner()
            .id;

        let prov = svc
            .get_provenance(Request::new(ProvenanceRequest { id: action }))
            .await
            .unwrap()
            .into_inner()
            .provenance;
        assert_eq!(prov, vec![obs]);
    }
}
