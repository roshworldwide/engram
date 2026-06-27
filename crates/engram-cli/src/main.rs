#![forbid(unsafe_code)]
//! The `engram` command-line interface — a thin shell over
//! [`engram_query::Engine`].
//!
//! ```text
//! engram init  <dir>
//! engram put   event  <dir> <session> <type> <payload>
//! engram put   belief <dir> <subject> <predicate> <object> [confidence]
//! engram get   event  <dir> <id>
//! engram get   belief <dir> <subject> <predicate>
//! engram as-of <dir> <subject> <predicate> <ms>
//! engram why   <dir> <id>
//! engram demo
//! engram bench
//! ```

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use engram_core::{AgentId, DecayFunction, EventType, MemoryId, SessionId, Timestamp};
use engram_query::Engine;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(run(&args))
}

/// A CLI failure, carrying the intended exit code (2 = usage, 1 = runtime).
enum CliError {
    Usage(String),
    Runtime(anyhow::Error),
}

impl From<engram_storage::StorageError> for CliError {
    fn from(e: engram_storage::StorageError) -> Self {
        CliError::Runtime(e.into())
    }
}

/// Dispatch a CLI invocation, returning the process exit code.
///
/// Split out from `main` so the exit-code contract is unit-testable.
fn run(args: &[String]) -> u8 {
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "version" | "--version" | "-V" => {
            println!("engram {}", env!("CARGO_PKG_VERSION"));
            0
        }
        "help" | "--help" | "-h" => {
            print!("{}", help_text());
            0
        }
        "bench" => {
            eprintln!("run `cargo xtask bench` to execute the criterion benchmark suite.");
            0
        }
        "init" | "put" | "get" | "as-of" | "why" | "demo" => match dispatch(cmd, &args[1..]) {
            Ok(()) => 0,
            Err(CliError::Usage(msg)) => {
                eprintln!("usage error: {msg}\n");
                print!("{}", help_text());
                2
            }
            Err(CliError::Runtime(e)) => {
                eprintln!("error: {e:#}");
                1
            }
        },
        other => {
            eprintln!("error: unknown command `{other}`\n");
            print!("{}", help_text());
            2
        }
    }
}

fn dispatch(cmd: &str, a: &[String]) -> Result<(), CliError> {
    match cmd {
        "init" => {
            let dir = arg(a, 0, "dir")?;
            Engine::open(dir)?;
            println!("initialized engram store at {dir}");
            Ok(())
        }
        "put" => put(a),
        "get" => get(a),
        "as-of" => {
            let (dir, subject, predicate) = (
                arg(a, 0, "dir")?,
                arg(a, 1, "subject")?,
                arg(a, 2, "predicate")?,
            );
            let ms = parse(arg(a, 3, "ms")?, "ms")?;
            let engine = Engine::open(dir)?;
            match engine.belief_at(subject, predicate, Timestamp::from_millis(ms)) {
                Some(v) => println!(
                    "{} (confidence {:.3})",
                    String::from_utf8_lossy(&v.record.object),
                    v.confidence
                ),
                None => println!("(no belief as-of {ms})"),
            }
            Ok(())
        }
        "why" => {
            let dir = arg(a, 0, "dir")?;
            let id: MemoryId = parse(arg(a, 1, "id")?, "id")?;
            let engine = Engine::open(dir)?;
            let chain = engine.provenance(id);
            if chain.is_empty() {
                println!("{id} has no recorded provenance");
            }
            for node in chain {
                println!("  └─ {}", describe(&engine, node));
            }
            Ok(())
        }
        "demo" => demo(),
        _ => unreachable!("dispatch only called for known subcommands"),
    }
}

fn put(a: &[String]) -> Result<(), CliError> {
    match arg(a, 0, "event|belief")? {
        "event" => {
            let dir = arg(a, 1, "dir")?;
            let session: u64 = parse(arg(a, 2, "session")?, "session")?;
            let event_type = parse_event_type(arg(a, 3, "type")?)?;
            let payload = arg(a, 4, "payload")?.as_bytes().to_vec();
            let engine = Engine::open(dir)?;
            let id = engine.record_event(
                AgentId(1),
                SessionId(session),
                Timestamp::from_millis(now_ms()),
                event_type,
                payload,
                vec![],
            )?;
            println!("{id}");
            Ok(())
        }
        "belief" => {
            let dir = arg(a, 1, "dir")?;
            let (subject, predicate, object) = (
                arg(a, 2, "subject")?,
                arg(a, 3, "predicate")?,
                arg(a, 4, "object")?,
            );
            let confidence = match a.get(5) {
                Some(s) => parse(s, "confidence")?,
                None => 1.0,
            };
            let engine = Engine::open(dir)?;
            let id = engine.upsert_belief(
                AgentId(1),
                subject.to_string(),
                predicate.to_string(),
                object.as_bytes().to_vec(),
                Timestamp::from_millis(now_ms()),
                confidence,
                DecayFunction::None,
                vec![],
            )?;
            println!("{id}");
            Ok(())
        }
        other => Err(CliError::Usage(format!(
            "`put {other}` — expected `event` or `belief`"
        ))),
    }
}

fn get(a: &[String]) -> Result<(), CliError> {
    match arg(a, 0, "event|belief")? {
        "event" => {
            let dir = arg(a, 1, "dir")?;
            let id: MemoryId = parse(arg(a, 2, "id")?, "id")?;
            let engine = Engine::open(dir)?;
            match engine.get_event(id) {
                Some(e) => println!(
                    "{id} [{:?}] session={} {}",
                    e.event_type,
                    e.session_id.0,
                    String::from_utf8_lossy(&e.payload)
                ),
                None => println!("(no event {id})"),
            }
            Ok(())
        }
        "belief" => {
            let (dir, subject, predicate) = (
                arg(a, 1, "dir")?,
                arg(a, 2, "subject")?,
                arg(a, 3, "predicate")?,
            );
            let engine = Engine::open(dir)?;
            match engine.current_belief(subject, predicate) {
                Some(v) => println!(
                    "{} (confidence {:.3})",
                    String::from_utf8_lossy(&v.record.object),
                    v.confidence
                ),
                None => println!("(no belief for {subject}.{predicate})"),
            }
            Ok(())
        }
        other => Err(CliError::Usage(format!(
            "`get {other}` — expected `event` or `belief`"
        ))),
    }
}

/// The self-contained SRE provenance demo (mirrors `cargo xtask demo`).
fn demo() -> Result<(), CliError> {
    let dir = std::env::temp_dir().join(format!("engram-cli-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let engine = Engine::open(&dir)?;
    let (agent, session) = (AgentId(1), SessionId(7));

    engine.put_skill(
        agent,
        "restart-on-high-latency",
        b"if Y > Z, restart X".to_vec(),
        Timestamp::from_millis(0),
    )?;
    let e42 = engine.record_event(
        agent,
        session,
        Timestamp::from_millis(100),
        EventType::Observation,
        b"metric Y = 95% > threshold Z = 80%".to_vec(),
        vec![],
    )?;
    let belief = engine.upsert_belief(
        agent,
        "service-x".into(),
        "health".into(),
        b"unhealthy".to_vec(),
        Timestamp::from_millis(100),
        0.92,
        DecayFunction::None,
        vec![e42],
    )?;
    let action = engine.record_event(
        agent,
        session,
        Timestamp::from_millis(101),
        EventType::Action,
        b"restart service X".to_vec(),
        vec![belief],
    )?;

    println!("Q: why did the agent restart service X? (action {action})");
    for node in engine.provenance(action) {
        println!("  └─ {}", describe(&engine, node));
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Render a provenance node, resolving it as an episodic event or a belief.
fn describe(engine: &Engine, id: MemoryId) -> String {
    if let Some(e) = engine.get_event(id) {
        format!(
            "{id} [episodic {:?}] {}",
            e.event_type,
            String::from_utf8_lossy(&e.payload)
        )
    } else if let Some(b) = engine.get_belief(id) {
        format!(
            "{id} [semantic] {}.{} = {}",
            b.subject,
            b.predicate,
            String::from_utf8_lossy(&b.object)
        )
    } else {
        format!("{id} (unknown)")
    }
}

fn arg<'a>(a: &'a [String], i: usize, what: &str) -> Result<&'a str, CliError> {
    a.get(i)
        .map(String::as_str)
        .ok_or_else(|| CliError::Usage(format!("missing <{what}>")))
}

fn parse<T: std::str::FromStr>(s: &str, what: &str) -> Result<T, CliError>
where
    T::Err: std::fmt::Display,
{
    s.parse::<T>()
        .map_err(|e| CliError::Usage(format!("bad <{what}> `{s}`: {e}")))
}

fn parse_event_type(s: &str) -> Result<EventType, CliError> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "tool_call" | "toolcall" => EventType::ToolCall,
        "message" | "msg" => EventType::Message,
        "observation" | "obs" => EventType::Observation,
        "action" => EventType::Action,
        other => {
            return Err(CliError::Usage(format!(
                "unknown event type `{other}` (tool_call|message|observation|action)"
            )))
        }
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The CLI usage text. Returned (not just printed) so tests can assert on it.
fn help_text() -> String {
    format!(
        "engram {version} — purpose-built storage engine for AI-agent memory\n\
         \n\
         USAGE:\n    \
             engram <COMMAND> [ARGS]\n\
         \n\
         COMMANDS:\n    \
             init     Initialize a new Engram store:  init <dir>\n    \
             put      Append an event / upsert a belief:  put event|belief <dir> ...\n    \
             get      Read a memory:  get event|belief <dir> ...\n    \
             as-of    Time-travel a belief:  as-of <dir> <subject> <predicate> <ms>\n    \
             why      Trace provenance:  why <dir> <id>\n    \
             bench    Run the criterion benchmark suite\n    \
             demo     Run the SRE provenance demo\n    \
             version  Print the version\n    \
             help     Print this help\n",
        version = env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
    use super::{help_text, run};

    #[test]
    fn version_command_succeeds() {
        assert_eq!(run(&["version".to_string()]), 0);
    }

    #[test]
    fn no_args_shows_help_and_succeeds() {
        assert_eq!(run(&[]), 0);
    }

    #[test]
    fn unknown_command_is_usage_error() {
        assert_eq!(run(&["frobnicate".to_string()]), 2);
    }

    #[test]
    fn missing_args_is_usage_error() {
        assert_eq!(run(&["put".to_string()]), 2);
        assert_eq!(run(&["why".to_string()]), 2);
    }

    #[test]
    fn help_text_mentions_every_command() {
        let help = help_text();
        for cmd in ["init", "put", "get", "as-of", "why", "bench", "demo"] {
            assert!(help.contains(cmd), "help should mention `{cmd}`");
        }
    }

    #[test]
    fn demo_runs_end_to_end() {
        assert_eq!(run(&["demo".to_string()]), 0);
    }
}
