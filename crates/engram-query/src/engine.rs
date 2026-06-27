//! The query engine: one [`Engine`] bundles the four memory stores and the
//! causal-provenance DAG over a single data directory, and wires writes into the
//! DAG so provenance is queryable end-to-end. This is the integration point the
//! REST/gRPC server (3c) and the Python SDK (3d) sit on top of.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use engram_core::{
    AgentId, Clock, DecayFunction, EdgeType, EpisodicRecord, EventType, MemoryId,
    MemoryIdGenerator, SessionId, SystemClock, SystemRng, Timestamp,
};
use engram_storage::{
    BeliefInput, BeliefView, CausalDag, EpisodicStore, ProceduralStore, Result, SemanticStore,
    WorkingMemory,
};

/// A single-instance Engram engine over a data directory.
pub struct Engine {
    episodic: EpisodicStore,
    semantic: SemanticStore,
    procedural: ProceduralStore,
    causal: CausalDag,
    working: WorkingMemory,
    clock: SystemClock,
    ids: Mutex<MemoryIdGenerator<SystemClock, SystemRng>>,
}

impl Engine {
    /// Open (or create) all stores under `dir`. A store is recovered if its WAL
    /// already exists, else created fresh.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let path = |name: &str| dir.join(name);
        let ep = path("episodic.wal");
        let se = path("semantic.wal");
        let pr = path("procedural.wal");
        let ca = path("causal.wal");
        Ok(Engine {
            episodic: if ep.exists() {
                EpisodicStore::open(&ep)?
            } else {
                EpisodicStore::create(&ep)?
            },
            semantic: if se.exists() {
                SemanticStore::open(&se)?
            } else {
                SemanticStore::create(&se)?
            },
            procedural: if pr.exists() {
                ProceduralStore::open(&pr)?
            } else {
                ProceduralStore::create(&pr)?
            },
            causal: if ca.exists() {
                CausalDag::open(&ca)?
            } else {
                CausalDag::create(&ca)?
            },
            working: WorkingMemory::default(),
            clock: SystemClock,
            ids: Mutex::new(MemoryIdGenerator::new(SystemClock, SystemRng::new())),
        })
    }

    fn next_id(&self) -> MemoryId {
        self.ids
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_id()
    }

    /// Record an immutable episodic event, linking each cause into the causal DAG
    /// (`cause → event`). Returns the new event's id.
    pub fn record_event(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        valid_time: Timestamp,
        event_type: EventType,
        payload: Vec<u8>,
        cause_ids: Vec<MemoryId>,
    ) -> Result<MemoryId> {
        let id = self.next_id();
        let record = EpisodicRecord {
            id,
            agent_id,
            session_id,
            valid_time,
            tx_time: self.clock.now(),
            event_type,
            payload,
            cause_ids: cause_ids.clone(),
        };
        self.episodic.append(record)?;
        self.episodic.commit()?;
        for cause in cause_ids {
            self.causal.add_edge(cause, id, EdgeType::Triggered)?;
        }
        self.causal.commit()?;
        Ok(id)
    }

    /// Fetch an episodic event by id.
    #[must_use]
    pub fn get_event(&self, id: MemoryId) -> Option<Arc<EpisodicRecord>> {
        self.episodic.get(id)
    }

    /// Episodic events in a session, ascending by id.
    #[must_use]
    pub fn session_events(&self, session: SessionId) -> Vec<Arc<EpisodicRecord>> {
        self.episodic.scan_session(session)
    }

    /// Upsert a semantic belief, linking each provenance memory into the causal
    /// DAG (`provenance → belief`). Returns the new belief version's id.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_belief(
        &self,
        agent_id: AgentId,
        subject: String,
        predicate: String,
        object: Vec<u8>,
        valid_from: Timestamp,
        confidence_init: f32,
        decay_fn: DecayFunction,
        provenance_ids: Vec<MemoryId>,
    ) -> Result<MemoryId> {
        let id = self.semantic.upsert_belief(BeliefInput {
            agent_id,
            subject,
            predicate,
            object,
            valid_from,
            confidence_init,
            decay_fn,
            provenance_ids: provenance_ids.clone(),
        })?;
        self.semantic.commit()?;
        for prov in provenance_ids {
            self.causal.add_edge(prov, id, EdgeType::Created)?;
        }
        self.causal.commit()?;
        Ok(id)
    }

    /// The current belief for `(subject, predicate)`, with confidence decayed to now.
    #[must_use]
    pub fn current_belief(&self, subject: &str, predicate: &str) -> Option<BeliefView> {
        self.semantic.current(subject, predicate)
    }

    /// Time-travel: what was believed for `(subject, predicate)` as-of transaction
    /// time `at`.
    #[must_use]
    pub fn belief_at(&self, subject: &str, predicate: &str, at: Timestamp) -> Option<BeliefView> {
        self.semantic.get_at_tx(subject, predicate, at)
    }

    /// Add a causal edge directly.
    pub fn add_edge(&self, from: MemoryId, to: MemoryId, edge_type: EdgeType) -> Result<()> {
        self.causal.add_edge(from, to, edge_type)?;
        self.causal.commit()
    }

    /// All transitive causes of `id` (its provenance chain), breadth-first.
    #[must_use]
    pub fn provenance(&self, id: MemoryId) -> Vec<MemoryId> {
        self.causal.find_provenance_chain(id)
    }

    /// Consolidate a session's episodic events into semantic beliefs (4a). Each
    /// promoted belief is upserted with its evidence as provenance, so it is
    /// causally linked back to the events it was derived from. Returns the new
    /// belief version ids.
    pub fn consolidate_session<E: crate::consolidation::SignalExtractor>(
        &self,
        extractor: &E,
        agent: AgentId,
        session: SessionId,
        consolidator: &crate::consolidation::Consolidator,
    ) -> Result<Vec<MemoryId>> {
        let events = self.episodic.scan_session(session);
        let beliefs = consolidator.consolidate(extractor, &events);
        let mut ids = Vec::with_capacity(beliefs.len());
        for belief in beliefs {
            let id = self.upsert_belief(
                agent,
                belief.subject,
                belief.predicate,
                belief.object.into_bytes(),
                self.clock.now(),
                belief.confidence,
                DecayFunction::None,
                belief.provenance,
            )?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// Record a new version of a procedural skill.
    pub fn put_skill(
        &self,
        agent_id: AgentId,
        name: &str,
        steps: Vec<u8>,
        valid_from: Timestamp,
    ) -> Result<MemoryId> {
        let id = self
            .procedural
            .put_skill(agent_id, name, steps, valid_from)?;
        self.procedural.commit()?;
        Ok(id)
    }

    /// The latest version of a skill.
    #[must_use]
    pub fn latest_skill(
        &self,
        agent_id: AgentId,
        name: &str,
    ) -> Option<Arc<engram_core::ProceduralRecord>> {
        self.procedural.latest(agent_id, name)
    }

    /// Push an entry into working memory, returning any FIFO-evicted entry.
    pub fn working_push(
        &self,
        record: engram_core::WorkingMemoryRecord,
    ) -> Option<engram_core::WorkingMemoryRecord> {
        self.working.push(record)
    }

    /// A reference to the working-memory scratchpad.
    #[must_use]
    pub fn working(&self) -> &WorkingMemory {
        &self.working
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn end_to_end_event_belief_and_provenance() {
        let dir = tempdir().unwrap();
        let engine = Engine::open(dir.path()).unwrap();

        // An observation, then an action it triggered.
        let obs = engine
            .record_event(
                AgentId(1),
                SessionId(1),
                Timestamp::from_millis(1000),
                EventType::Observation,
                b"metric Y = 95% > threshold 80%".to_vec(),
                vec![],
            )
            .unwrap();
        let action = engine
            .record_event(
                AgentId(1),
                SessionId(1),
                Timestamp::from_millis(1001),
                EventType::Action,
                b"restart service X".to_vec(),
                vec![obs],
            )
            .unwrap();

        // A belief derived from the observation.
        let belief = engine
            .upsert_belief(
                AgentId(1),
                "service-x".into(),
                "health".into(),
                b"unhealthy".to_vec(),
                Timestamp::from_millis(1000),
                0.9,
                DecayFunction::None,
                vec![obs],
            )
            .unwrap();

        // Reads.
        assert_eq!(
            engine.get_event(action).unwrap().payload,
            b"restart service X"
        );
        assert_eq!(
            engine
                .current_belief("service-x", "health")
                .unwrap()
                .record
                .object,
            b"unhealthy"
        );

        // Provenance: "why did the agent restart X / believe it unhealthy?" -> obs.
        assert_eq!(engine.provenance(action), vec![obs]);
        assert_eq!(engine.provenance(belief), vec![obs]);

        // Recovery across reopen.
        drop(engine);
        let engine = Engine::open(dir.path()).unwrap();
        assert_eq!(
            engine
                .current_belief("service-x", "health")
                .unwrap()
                .record
                .object,
            b"unhealthy"
        );
        assert_eq!(engine.provenance(action), vec![obs]);
    }

    #[test]
    fn skills_and_working_memory() {
        let dir = tempdir().unwrap();
        let engine = Engine::open(dir.path()).unwrap();
        engine
            .put_skill(AgentId(1), "deploy", b"v1".to_vec(), Timestamp(0))
            .unwrap();
        engine
            .put_skill(AgentId(1), "deploy", b"v2".to_vec(), Timestamp(0))
            .unwrap();
        assert_eq!(
            engine.latest_skill(AgentId(1), "deploy").unwrap().version,
            2
        );

        let rec = engram_core::WorkingMemoryRecord {
            id: MemoryId(1),
            agent_id: AgentId(1),
            session_id: SessionId(1),
            created: Timestamp(0),
            payload: vec![1],
        };
        assert!(engine.working_push(rec).is_none());
        assert_eq!(engine.working().len(), 1);
    }

    #[test]
    fn consolidation_promotes_belief_with_traceable_provenance() {
        use crate::consolidation::{Consolidator, FieldSignalExtractor};

        let dir = tempdir().unwrap();
        let engine = Engine::open(dir.path()).unwrap();

        // 20 "be concise" signals, 3 conflicting "verbose", plus unstructured noise.
        let mut concise = Vec::new();
        for i in 0..20 {
            concise.push(
                engine
                    .record_event(
                        AgentId(1),
                        SessionId(7),
                        Timestamp::from_millis(1000 + i),
                        EventType::Message,
                        b"user\tresponse_length\tconcise".to_vec(),
                        vec![],
                    )
                    .unwrap(),
            );
        }
        for i in 0..3 {
            engine
                .record_event(
                    AgentId(1),
                    SessionId(7),
                    Timestamp::from_millis(2000 + i),
                    EventType::Message,
                    b"user\tresponse_length\tverbose".to_vec(),
                    vec![],
                )
                .unwrap();
        }
        for i in 0..5 {
            engine
                .record_event(
                    AgentId(1),
                    SessionId(7),
                    Timestamp::from_millis(3000 + i),
                    EventType::Observation,
                    b"unstructured noise".to_vec(),
                    vec![],
                )
                .unwrap();
        }

        let ids = engine
            .consolidate_session(
                &FieldSignalExtractor,
                AgentId(1),
                SessionId(7),
                &Consolidator::default(),
            )
            .unwrap();
        assert_eq!(ids.len(), 1);

        let view = engine.current_belief("user", "response_length").unwrap();
        assert_eq!(view.record.object, b"concise"); // dominant value
        assert!((view.confidence - 0.878).abs() < 0.01); // 1 - 0.9^20
        assert_eq!(view.record.provenance_ids.len(), 20);

        // The consolidated belief traces back to exactly its 20 source events.
        let mut prov = engine.provenance(ids[0]);
        prov.sort();
        concise.sort();
        assert_eq!(prov, concise);
    }
}
