//! Memory consolidation (4a): promote long-term semantic beliefs from repeated
//! episodic evidence, **conservatively** and with full provenance.
//!
//! The pipeline is: extract a `(subject, predicate, object)` [`Signal`] from each
//! episodic event (via a pluggable [`SignalExtractor`] — a rule-based one here, a
//! local-LLM one could be substituted), group by `(subject, predicate)`, and for
//! each group promote the *dominant* object to a belief — but only when the
//! evidence count clears `min_evidence`. Confidence grows with evidence and the
//! belief's provenance is exactly the supporting event ids, so every consolidated
//! belief can be traced back to the events it was derived from.

use std::collections::HashMap;
use std::sync::Arc;

use engram_core::{EpisodicRecord, MemoryId};

/// A `(subject, predicate, object)` signal extracted from one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signal {
    /// Subject of the belief triple.
    pub subject: String,
    /// Predicate of the belief triple.
    pub predicate: String,
    /// Object of the belief triple.
    pub object: String,
}

/// Maps an episodic event to a belief signal, if any. Rule-based by default; an
/// LLM-backed implementation could return signals at lower confidence.
pub trait SignalExtractor {
    /// Extract a signal from `event`, or `None` if it carries none.
    fn extract(&self, event: &EpisodicRecord) -> Option<Signal>;
}

/// A conservative rule-based extractor: the payload must be UTF-8
/// `subject\tpredicate\tobject` (tab-separated). Anything else yields no signal.
#[derive(Clone, Copy, Debug, Default)]
pub struct FieldSignalExtractor;

impl SignalExtractor for FieldSignalExtractor {
    fn extract(&self, event: &EpisodicRecord) -> Option<Signal> {
        let text = std::str::from_utf8(&event.payload).ok()?;
        let mut parts = text.split('\t');
        let subject = parts.next()?.trim();
        let predicate = parts.next()?.trim();
        let object = parts.next()?.trim();
        if parts.next().is_some() || subject.is_empty() || predicate.is_empty() || object.is_empty()
        {
            return None;
        }
        Some(Signal {
            subject: subject.to_owned(),
            predicate: predicate.to_owned(),
            object: object.to_owned(),
        })
    }
}

/// A belief promoted from episodic evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct ConsolidatedBelief {
    /// Subject of the belief.
    pub subject: String,
    /// Predicate of the belief.
    pub predicate: String,
    /// The dominant object value.
    pub object: String,
    /// Confidence in `[0, 1]`, increasing with evidence.
    pub confidence: f32,
    /// The ids of the events that support this belief (its provenance).
    pub provenance: Vec<MemoryId>,
}

/// The consolidation policy.
#[derive(Clone, Copy, Debug)]
pub struct Consolidator {
    /// Minimum number of supporting events before a belief is promoted.
    pub min_evidence: usize,
    /// Per-evidence confidence weight: combined confidence is
    /// `1 − (1 − weight)^evidence`, capped at `MAX_CONFIDENCE`.
    pub evidence_weight: f32,
}

/// Beliefs never consolidate to certainty.
const MAX_CONFIDENCE: f32 = 0.99;

impl Default for Consolidator {
    fn default() -> Self {
        Consolidator {
            min_evidence: 3,
            evidence_weight: 0.1,
        }
    }
}

impl Consolidator {
    /// Confidence for `evidence` independent confirmations:
    /// `1 − (1 − weight)^evidence`, capped.
    #[must_use]
    pub fn confidence(&self, evidence: usize) -> f32 {
        let p = 1.0 - (1.0 - self.evidence_weight).powi(evidence as i32);
        p.min(MAX_CONFIDENCE)
    }

    /// Consolidate a batch of episodic events into beliefs. For each
    /// `(subject, predicate)` the dominant object (most supporting events) is
    /// promoted when its evidence count is at least `min_evidence`. Output is
    /// sorted by `(subject, predicate)` for determinism.
    #[must_use]
    pub fn consolidate<E: SignalExtractor>(
        &self,
        extractor: &E,
        events: &[Arc<EpisodicRecord>],
    ) -> Vec<ConsolidatedBelief> {
        // (subject, predicate) -> object -> supporting event ids (insertion order).
        let mut groups: HashMap<(String, String), HashMap<String, Vec<MemoryId>>> = HashMap::new();
        for event in events {
            if let Some(sig) = extractor.extract(event) {
                groups
                    .entry((sig.subject, sig.predicate))
                    .or_default()
                    .entry(sig.object)
                    .or_default()
                    .push(event.id);
            }
        }

        let mut beliefs = Vec::new();
        for ((subject, predicate), objects) in groups {
            // Dominant object = most evidence; tie-break by object value for
            // determinism.
            let best = objects
                .into_iter()
                .max_by(|(oa, ia), (ob, ib)| ia.len().cmp(&ib.len()).then_with(|| ob.cmp(oa)));
            if let Some((object, ids)) = best {
                if ids.len() >= self.min_evidence {
                    beliefs.push(ConsolidatedBelief {
                        subject,
                        predicate,
                        object,
                        confidence: self.confidence(ids.len()),
                        provenance: ids,
                    });
                }
            }
        }
        beliefs.sort_by(|a, b| (&a.subject, &a.predicate).cmp(&(&b.subject, &b.predicate)));
        beliefs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_core::{AgentId, EventType, SessionId, Timestamp};

    fn event(id: u128, payload: &str) -> Arc<EpisodicRecord> {
        Arc::new(EpisodicRecord {
            id: MemoryId(id),
            agent_id: AgentId(1),
            session_id: SessionId(1),
            valid_time: Timestamp(id as i64),
            tx_time: Timestamp(0),
            event_type: EventType::Message,
            payload: payload.as_bytes().to_vec(),
            cause_ids: vec![],
        })
    }

    #[test]
    fn confidence_grows_with_evidence() {
        let c = Consolidator::default();
        assert!((c.confidence(1) - 0.1).abs() < 1e-6);
        assert!((c.confidence(20) - 0.878).abs() < 0.01); // 1 - 0.9^20
        assert!(c.confidence(1000) <= MAX_CONFIDENCE);
    }

    #[test]
    fn dominant_object_wins_with_full_provenance() {
        let mut events = Vec::new();
        for i in 0..20 {
            events.push(event(i, "user\tresponse_length\tconcise"));
        }
        for i in 20..23 {
            events.push(event(i, "user\tresponse_length\tverbose"));
        }
        for i in 23..28 {
            events.push(event(i, "unstructured noise")); // no signal
        }
        let beliefs = Consolidator::default().consolidate(&FieldSignalExtractor, &events);
        assert_eq!(beliefs.len(), 1);
        let b = &beliefs[0];
        assert_eq!(
            (b.subject.as_str(), b.predicate.as_str(), b.object.as_str()),
            ("user", "response_length", "concise")
        );
        assert_eq!(b.provenance.len(), 20); // only the supporting events
        assert!((b.confidence - 0.878).abs() < 0.01);
    }

    #[test]
    fn below_threshold_is_not_promoted() {
        let events = vec![event(1, "a\tb\tc"), event(2, "a\tb\tc")];
        // default min_evidence = 3
        assert!(Consolidator::default()
            .consolidate(&FieldSignalExtractor, &events)
            .is_empty());
    }
}
