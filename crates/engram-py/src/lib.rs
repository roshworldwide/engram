//! # engram-py
//!
//! PyO3 bindings exposing Engram to Python as the `engram` module (R8).
//!
//! Build the wheel with `maturin build --features python` (or `maturin develop`
//! into a virtualenv). Without the `python` feature this crate is an empty
//! cdylib/rlib, so the default workspace build needs no Python toolchain.
//!
//! ```python
//! import engram
//! mem = engram.Engram("./data")
//! obs = mem.record_event(agent=1, session=1, valid_time_ms=1000,
//!                        event_type="Observation", payload="metric Y high")
//! act = mem.record_event(agent=1, session=1, valid_time_ms=1001,
//!                        event_type="Action", payload="restart X", cause_ids=[obs])
//! assert mem.provenance(act) == [obs]
//! ```

#[cfg(feature = "python")]
mod bindings {
    use std::str::FromStr;

    use pyo3::exceptions::{PyRuntimeError, PyValueError};
    use pyo3::prelude::*;

    use engram_core::{AgentId, DecayFunction, EventType, MemoryId, SessionId, Timestamp};
    use engram_query::Engine;

    fn parse_event_type(s: &str) -> PyResult<EventType> {
        match s.to_ascii_lowercase().as_str() {
            "toolcall" | "tool_call" => Ok(EventType::ToolCall),
            "message" | "msg" => Ok(EventType::Message),
            "observation" | "obs" => Ok(EventType::Observation),
            "action" => Ok(EventType::Action),
            other => Err(PyValueError::new_err(format!(
                "unknown event_type: {other} (tool_call|message|observation|action)"
            ))),
        }
    }

    /// Parse a `(kind, rate)` decay spec from Python. `kind` is one of
    /// `none|exponential|power_law`; `rate` is `lambda` (per second) for
    /// exponential or `beta` for power-law.
    fn parse_decay(kind: Option<&str>, rate: Option<f32>) -> PyResult<DecayFunction> {
        match kind.map(str::to_ascii_lowercase).as_deref() {
            None | Some("none") => Ok(DecayFunction::None),
            Some("exponential" | "exp") => Ok(DecayFunction::Exponential {
                lambda: rate.unwrap_or(0.0),
            }),
            Some("power_law" | "powerlaw" | "power") => Ok(DecayFunction::PowerLaw {
                beta: rate.unwrap_or(0.0),
            }),
            Some(other) => Err(PyValueError::new_err(format!(
                "unknown decay: {other} (none|exponential|power_law)"
            ))),
        }
    }

    fn parse_id(s: &str) -> PyResult<MemoryId> {
        MemoryId::from_str(s).map_err(|e| PyValueError::new_err(format!("invalid id: {e}")))
    }

    fn parse_ids(ids: Option<Vec<String>>) -> PyResult<Vec<MemoryId>> {
        ids.unwrap_or_default()
            .iter()
            .map(|s| parse_id(s))
            .collect()
    }

    fn runtime_err(e: impl std::fmt::Display) -> PyErr {
        PyRuntimeError::new_err(e.to_string())
    }

    /// A single-instance Engram memory engine over a data directory.
    #[pyclass]
    struct Engram {
        inner: Engine,
    }

    #[pymethods]
    impl Engram {
        /// Open (or create) an engine at `path`.
        #[new]
        fn new(path: String) -> PyResult<Self> {
            Engine::open(&path)
                .map(|inner| Engram { inner })
                .map_err(runtime_err)
        }

        /// Record an immutable episodic event; returns its id. Each cause id is
        /// linked into the provenance DAG.
        #[pyo3(signature = (agent, session, valid_time_ms, event_type, payload, cause_ids=None))]
        fn record_event(
            &self,
            agent: u64,
            session: u64,
            valid_time_ms: i64,
            event_type: &str,
            payload: String,
            cause_ids: Option<Vec<String>>,
        ) -> PyResult<String> {
            let id = self
                .inner
                .record_event(
                    AgentId(agent),
                    SessionId(session),
                    Timestamp::from_millis(valid_time_ms),
                    parse_event_type(event_type)?,
                    payload.into_bytes(),
                    parse_ids(cause_ids)?,
                )
                .map_err(runtime_err)?;
            Ok(id.to_string())
        }

        /// Upsert a semantic belief; returns the new version's id. Each provenance
        /// id is linked into the provenance DAG.
        #[allow(clippy::too_many_arguments)]
        #[pyo3(signature = (agent, subject, predicate, object, valid_from_ms, confidence, provenance_ids=None, decay=None, decay_rate=None))]
        fn upsert_belief(
            &self,
            agent: u64,
            subject: String,
            predicate: String,
            object: String,
            valid_from_ms: i64,
            confidence: f32,
            provenance_ids: Option<Vec<String>>,
            decay: Option<&str>,
            decay_rate: Option<f32>,
        ) -> PyResult<String> {
            let id = self
                .inner
                .upsert_belief(
                    AgentId(agent),
                    subject,
                    predicate,
                    object.into_bytes(),
                    Timestamp::from_millis(valid_from_ms),
                    confidence,
                    parse_decay(decay, decay_rate)?,
                    parse_ids(provenance_ids)?,
                )
                .map_err(runtime_err)?;
            Ok(id.to_string())
        }

        /// The current belief for `(subject, predicate)` as `(object, confidence)`,
        /// or `None`.
        fn current_belief(&self, subject: &str, predicate: &str) -> Option<(String, f32)> {
            self.inner.current_belief(subject, predicate).map(|v| {
                (
                    String::from_utf8_lossy(&v.record.object).into_owned(),
                    v.confidence,
                )
            })
        }

        /// Time-travel: the belief as-of transaction time `at_ms`.
        fn belief_at(&self, subject: &str, predicate: &str, at_ms: i64) -> Option<(String, f32)> {
            self.inner
                .belief_at(subject, predicate, Timestamp::from_millis(at_ms))
                .map(|v| {
                    (
                        String::from_utf8_lossy(&v.record.object).into_owned(),
                        v.confidence,
                    )
                })
        }

        /// The causal-provenance chain (transitive causes) of a memory id.
        fn provenance(&self, id: &str) -> PyResult<Vec<String>> {
            let id = parse_id(id)?;
            Ok(self
                .inner
                .provenance(id)
                .iter()
                .map(ToString::to_string)
                .collect())
        }
    }

    #[pymodule]
    fn engram(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<Engram>()?;
        m.add("__version__", env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }
}
