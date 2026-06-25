#![forbid(unsafe_code)]
//! The `engram` command-line interface.
//!
//! **Phase 0 scaffold.** The command surface is declared so the binary, help
//! text, and exit-code contract exist and are tested; each subcommand is wired
//! to real storage/query calls in later phases.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(run(&args))
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
        "init" | "put" | "get" | "as-of" | "why" | "bench" | "demo" => {
            eprintln!("`engram {cmd}` is not implemented yet (lands in a later phase).");
            2
        }
        other => {
            eprintln!("error: unknown command `{other}`\n");
            print!("{}", help_text());
            2
        }
    }
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
             init     Initialize a new Engram store in the current directory\n    \
             put      Append an episodic event or upsert a semantic belief\n    \
             get      Read a memory by id (optionally as-of a time)\n    \
             as-of    Run a bitemporal time-travel query\n    \
             why      Trace the causal-provenance chain for a memory\n    \
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
    fn unimplemented_subcommand_is_nonzero() {
        assert_eq!(run(&["put".to_string()]), 2);
    }

    #[test]
    fn help_text_mentions_every_command() {
        let help = help_text();
        for cmd in ["init", "put", "get", "as-of", "why", "bench", "demo"] {
            assert!(help.contains(cmd), "help should mention `{cmd}`");
        }
    }
}
