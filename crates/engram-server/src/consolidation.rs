//! Background consolidation service (4a): a `tokio` task that periodically
//! promotes beliefs from recent episodic evidence for a set of sessions. The
//! extraction logic lives in [`engram_query::Consolidator`]; this just schedules it.

use std::sync::Arc;
use std::time::Duration;

use engram_core::{AgentId, SessionId};
use engram_query::{Consolidator, Engine, SignalExtractor};

/// Spawn a background task that consolidates `sessions` every `period`, returning
/// its [`tokio::task::JoinHandle`] (abort it to stop).
pub fn spawn_consolidation<E>(
    engine: Arc<Engine>,
    extractor: Arc<E>,
    agent: AgentId,
    sessions: Vec<SessionId>,
    consolidator: Consolidator,
    period: Duration,
) -> tokio::task::JoinHandle<()>
where
    E: SignalExtractor + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(period);
        loop {
            ticker.tick().await;
            for &session in &sessions {
                // A failed pass (e.g. transient I/O) is logged by the caller's
                // tracing; the loop keeps running.
                let _ =
                    engine.consolidate_session(extractor.as_ref(), agent, session, &consolidator);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_core::{EventType, Timestamp};
    use engram_query::FieldSignalExtractor;
    use tempfile::tempdir;

    #[tokio::test]
    async fn background_task_promotes_a_belief() {
        let dir = tempdir().unwrap();
        let engine = Arc::new(Engine::open(dir.path()).unwrap());
        for i in 0..5 {
            engine
                .record_event(
                    AgentId(1),
                    SessionId(1),
                    Timestamp::from_millis(1000 + i),
                    EventType::Message,
                    b"user\ttone\tformal".to_vec(),
                    vec![],
                )
                .unwrap();
        }

        let handle = spawn_consolidation(
            Arc::clone(&engine),
            Arc::new(FieldSignalExtractor),
            AgentId(1),
            vec![SessionId(1)],
            Consolidator::default(),
            Duration::from_millis(5),
        );

        // Give the background task a few ticks to run.
        tokio::time::sleep(Duration::from_millis(60)).await;
        handle.abort();

        let view = engine
            .current_belief("user", "tone")
            .expect("belief consolidated");
        assert_eq!(view.record.object, b"formal");
    }
}
