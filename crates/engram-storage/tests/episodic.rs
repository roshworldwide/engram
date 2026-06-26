//! Integration test for the episodic store (2a): 10k events, exact time-range
//! slices, session isolation, and full recovery after reopen.

use engram_core::{AgentId, EpisodicRecord, EventType, MemoryId, SessionId, Timestamp};
use engram_storage::EpisodicStore;
use tempfile::tempdir;

const BASE_MS: i64 = 1_000_000;

fn event(i: u128) -> EpisodicRecord {
    EpisodicRecord {
        id: MemoryId(i),
        agent_id: AgentId(1),
        session_id: SessionId((i % 16) as u64),
        valid_time: Timestamp::from_millis(BASE_MS + i as i64),
        tx_time: Timestamp(0),
        event_type: EventType::Observation,
        payload: format!("event-{i}").into_bytes(),
        cause_ids: vec![],
    }
}

#[test]
fn ten_thousand_events_slices_sessions_and_recovery() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("episodic.wal");
    let n: u128 = 10_000;

    {
        let store = EpisodicStore::create(&path).unwrap();
        for i in 0..n {
            store.append(event(i)).unwrap();
            if i % 1000 == 999 {
                store.commit().unwrap(); // group commit every 1000
            }
        }
        store.commit().unwrap();
        assert_eq!(store.len(), n as usize);

        // Exact half-open time slice: valid_ms in [BASE+2500, BASE+7500) -> i in [2500, 7500).
        let slice = store.scan_time_range(
            Timestamp::from_millis(BASE_MS + 2500),
            Timestamp::from_millis(BASE_MS + 7500),
        );
        assert_eq!(slice.len(), 5000);
        assert_eq!(slice.first().unwrap().id.0, 2500);
        assert_eq!(slice.last().unwrap().id.0, 7499);
        assert!(slice.windows(2).all(|w| w[0].valid_time <= w[1].valid_time));

        // Session isolation: 10000 / 16 = 625 events in each of 16 sessions.
        let s3 = store.scan_session(SessionId(3));
        assert_eq!(s3.len(), 625);
        assert!(s3.iter().all(|r| r.session_id == SessionId(3)));
    }

    // Reopen: every committed event is recovered and queryable.
    let store = EpisodicStore::open(&path).unwrap();
    assert_eq!(store.len(), n as usize);
    assert_eq!(
        store.get(MemoryId(4242)).unwrap().valid_time,
        Timestamp::from_millis(BASE_MS + 4242)
    );
    let head = store.scan_time_range(
        Timestamp::from_millis(BASE_MS),
        Timestamp::from_millis(BASE_MS + 100),
    );
    assert_eq!(head.len(), 100);
    assert_eq!(head[0].id.0, 0);
}
