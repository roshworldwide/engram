#![forbid(unsafe_code)]
//! The Engram SRE killer demo (4b): store observations (episodic), a runbook
//! (procedural), and inferred state (semantic), take an action, then answer
//! *"why did the agent restart service X?"* by tracing the causal-provenance DAG
//! back to the root-cause observation.
//!
//! Run with `cargo run -p sre-demo` or `cargo xtask demo`.

use std::fmt::Write as _;
use std::path::Path;

use engram_core::{AgentId, DecayFunction, EventType, SessionId, Timestamp};
use engram_query::Engine;

fn main() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    print!("{}", run(dir.path())?);
    Ok(())
}

/// Run the demo against a data directory, returning the narrative (so it is
/// testable).
fn run(dir: &Path) -> anyhow::Result<String> {
    let engine = Engine::open(dir)?;
    let agent = AgentId(1);
    let session = SessionId(7);
    let ts = |ms: i64| Timestamp::from_millis(1_700_000_000_000 + ms);

    let mut out = String::new();
    writeln!(out, "== Engram SRE provenance demo ==\n")?;

    // Procedural: a learned runbook.
    engine.put_skill(
        agent,
        "restart-on-high-latency",
        b"1. read metric Y\n2. if Y > threshold Z, restart service X".to_vec(),
        ts(0),
    )?;
    writeln!(
        out,
        "[procedural] learned runbook 'restart-on-high-latency'"
    )?;

    // Episodic: observations (a calm baseline, then the alert).
    let _baseline = engine.record_event(
        agent,
        session,
        ts(10),
        EventType::Observation,
        b"metric Y = 40% (ok)".to_vec(),
        vec![],
    )?;
    let e42 = engine.record_event(
        agent,
        session,
        ts(100),
        EventType::Observation,
        b"metric Y = 95% > threshold Z = 80%".to_vec(),
        vec![],
    )?;
    writeln!(
        out,
        "[episodic]   observed {e42}: metric Y = 95% > threshold Z = 80%"
    )?;

    // Semantic: inferred state derived from the observation.
    let belief = engine.upsert_belief(
        agent,
        "service-x".into(),
        "health".into(),
        b"unhealthy".to_vec(),
        ts(100),
        0.92,
        DecayFunction::None,
        vec![e42],
    )?;
    writeln!(
        out,
        "[semantic]   inferred {belief}: service-x.health = unhealthy (conf 0.92), from {e42}"
    )?;

    // Action: restart, caused by the belief (which was derived from the observation).
    let action = engine.record_event(
        agent,
        session,
        ts(101),
        EventType::Action,
        b"restart service X".to_vec(),
        vec![belief],
    )?;
    writeln!(out, "[action]     {action}: restart service X\n")?;

    // The killer question: trace provenance back to the root cause.
    writeln!(out, "Q: why did the agent restart service X?")?;
    writeln!(out, "   tracing the provenance of action {action}:")?;
    for (step, id) in engine.provenance(action).into_iter().enumerate() {
        let indent = "   ".repeat(step + 1);
        if let Some(event) = engine.get_event(id) {
            writeln!(
                out,
                "{indent}└─ {id}  [episodic {:?}] {}",
                event.event_type,
                String::from_utf8_lossy(&event.payload)
            )?;
        } else if let Some(b) = engine.get_belief(id) {
            writeln!(
                out,
                "{indent}└─ {id}  [semantic] {}.{} = {}",
                b.subject,
                b.predicate,
                String::from_utf8_lossy(&b.object)
            )?;
        }
    }
    writeln!(
        out,
        "\nA: it observed metric Y exceed threshold Z (episodic {e42})."
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn demo_traces_action_to_root_cause_observation() {
        let dir = tempfile::tempdir().unwrap();
        let out = run(dir.path()).unwrap();
        assert!(out.contains("why did the agent restart service X?"));
        assert!(out.contains("metric Y = 95% > threshold Z = 80%"));
        assert!(out.contains("service-x.health = unhealthy"));
        assert!(out.contains("it observed metric Y exceed threshold Z"));
    }
}
