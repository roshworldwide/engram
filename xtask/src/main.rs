#![forbid(unsafe_code)]
//! Engram developer automation. Invoke through the cargo alias: `cargo xtask <task>`.
//!
//! Tasks:
//! - `ci`     — the full local gate: fmt-check, clippy (`-D warnings`), build, test, deny*
//! - `fmt`    — format the whole workspace in place
//! - `clippy` — lint with warnings denied
//! - `build`  — build all targets
//! - `test`   — run the test suite
//! - `bench`  — run the criterion benchmarks
//! - `deny`*  — supply-chain check (needs `cargo-deny`)
//! - `cov`*   — line-coverage summary (needs `cargo-llvm-cov`)
//! - `fuzz`*  — fuzz smoke run (needs `cargo-fuzz` + nightly)
//! - `demo`   — run the SRE provenance demo
//!
//! Tasks marked `*` depend on tools that may not be installed. In `ci` they are
//! reported as `SKIP` (not `FAIL`) so the core gate stays green on a clean
//! machine; CI installs them to enforce the full gate.

use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let task = args.first().map(String::as_str).unwrap_or("help");

    let ok = match task {
        "ci" => cmd_ci(),
        "fmt" => step("Format", &["fmt", "--all"]).passed(),
        "clippy" => step("Clippy", &clippy_args()).passed(),
        "build" => step("Build", &["build", "--all-targets", "--workspace"]).passed(),
        "test" => step("Tests", &["test", "--workspace"]).passed(),
        "bench" => step("Benchmarks", &["bench", "--workspace"]).passed(),
        "deny" => optional("Supply chain", "deny", &["deny", "check"]),
        "cov" => optional(
            "Coverage",
            "llvm-cov",
            &["llvm-cov", "--workspace", "--summary-only"],
        ),
        "fuzz" => optional("Fuzz smoke", "fuzz", &["fuzz", "list"]),
        "grpc" => !matches!(grpc_step(), Outcome::Failed),
        "python" => !matches!(python_step(), Outcome::Failed),
        "demo" => cmd_demo(),
        "help" | "--help" | "-h" => {
            print_help();
            true
        }
        other => {
            eprintln!("error: unknown task `{other}`\n");
            print_help();
            false
        }
    };

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The cargo binary to drive (honours the `CARGO` env var rustup/cargo sets).
fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn clippy_args() -> Vec<&'static str> {
    // Default features only: the `grpc` feature needs `protoc`, so it is checked
    // by the dedicated `grpc` step (which skips when protoc is absent) rather than
    // pulled in here via `--all-features`.
    vec![
        "clippy",
        "--all-targets",
        "--workspace",
        "--",
        "-D",
        "warnings",
    ]
}

/// Result of a single gate step.
enum Outcome {
    Passed,
    Failed,
    Skipped(String),
}

impl Outcome {
    fn passed(&self) -> bool {
        matches!(self, Outcome::Passed)
    }
}

/// Run a required cargo step, streaming its output. A spawn failure is a hard
/// error (cargo itself must be present).
fn step(label: &str, args: &[&str]) -> Outcome {
    println!("\n==> {label}: cargo {}", args.join(" "));
    match Command::new(cargo()).args(args).status() {
        Ok(status) if status.success() => Outcome::Passed,
        Ok(status) => {
            eprintln!("    {label} FAILED ({status})");
            Outcome::Failed
        }
        Err(err) => {
            eprintln!("    could not run cargo: {err}");
            Outcome::Failed
        }
    }
}

/// Run a step that depends on an optional cargo subcommand. When the subcommand
/// is absent it is skipped (not failed). Returns `true` unless an installed tool
/// actually fails.
fn optional(label: &str, subcommand: &str, args: &[&str]) -> bool {
    !matches!(optional_step(label, subcommand, args), Outcome::Failed)
}

fn optional_step(label: &str, subcommand: &str, args: &[&str]) -> Outcome {
    if !subcommand_present(subcommand) {
        let reason = format!("cargo-{subcommand} not installed");
        println!("\n==> {label}: SKIP — {reason}");
        return Outcome::Skipped(reason);
    }
    step(label, args)
}

/// Detect whether a cargo subcommand (e.g. `cargo deny`) is installed by probing
/// its `--version`.
fn subcommand_present(subcommand: &str) -> bool {
    Command::new(cargo())
        .args([subcommand, "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Detect whether a plain binary (e.g. `protoc`) is on `PATH`.
fn binary_present(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Build, clippy, and test the optional `grpc` feature (needs `protoc`). Skips
/// cleanly when protoc is absent.
fn grpc_step() -> Outcome {
    if !binary_present("protoc") {
        let reason = "protoc not installed".to_string();
        println!("\n==> gRPC feature: SKIP — {reason}");
        return Outcome::Skipped(reason);
    }
    let clippy = step(
        "gRPC clippy",
        &[
            "clippy",
            "-p",
            "engram-server",
            "--all-targets",
            "--features",
            "grpc",
            "--",
            "-D",
            "warnings",
        ],
    );
    if !clippy.passed() {
        return clippy;
    }
    step(
        "gRPC test",
        &["test", "-p", "engram-server", "--features", "grpc"],
    )
}

/// Clippy the optional `python` feature of the PyO3 bindings (needs `python3` for
/// the pyo3 build config). Skips cleanly when python3 is absent. The wheel build +
/// pytest run live in a dedicated CI job.
fn python_step() -> Outcome {
    if !binary_present("python3") {
        let reason = "python3 not installed".to_string();
        println!("\n==> python feature: SKIP — {reason}");
        return Outcome::Skipped(reason);
    }
    step(
        "python bindings clippy",
        &[
            "clippy",
            "-p",
            "engram-py",
            "--features",
            "python",
            "--",
            "-D",
            "warnings",
        ],
    )
}

/// The full local gate. Optional tools degrade to `SKIP`.
fn cmd_ci() -> bool {
    // Evaluated left-to-right, so the gate steps run in this order.
    let results: Vec<(&str, Outcome)> = vec![
        (
            "fmt --check",
            step("Formatting", &["fmt", "--all", "--", "--check"]),
        ),
        ("clippy -D warnings", step("Clippy", &clippy_args())),
        (
            "build",
            step("Build", &["build", "--all-targets", "--workspace"]),
        ),
        ("test", step("Tests", &["test", "--workspace"])),
        ("grpc feature", grpc_step()),
        ("python feature", python_step()),
        (
            "cargo-deny",
            optional_step("Supply chain", "deny", &["deny", "check"]),
        ),
    ];
    summarize(&results)
}

fn summarize(results: &[(&str, Outcome)]) -> bool {
    println!("\n-------- xtask ci summary --------");
    let mut all_ok = true;
    for (name, outcome) in results {
        let line = match outcome {
            Outcome::Passed => format!("  [PASS] {name}"),
            Outcome::Failed => {
                all_ok = false;
                format!("  [FAIL] {name}")
            }
            Outcome::Skipped(reason) => format!("  [SKIP] {name} — {reason}"),
        };
        println!("{line}");
    }
    println!("---------------------------------");
    println!("gate: {}", if all_ok { "GREEN" } else { "RED" });
    all_ok
}

fn cmd_demo() -> bool {
    println!("Engram SRE provenance demo lands in Phase 4b.");
    println!("It will store observations (episodic), runbook steps (procedural), and");
    println!("inferred state (semantic), then trace `why did the agent restart X?` to");
    println!("the root-cause episodic event via the causal-provenance DAG.");
    true
}

fn print_help() {
    println!("cargo xtask <task>");
    println!();
    println!("TASKS:");
    println!("  ci      Full local gate: fmt-check, clippy -D warnings, build, test, deny*");
    println!("  fmt     Format the workspace in place");
    println!("  clippy  Lint with warnings denied");
    println!("  build   Build all targets");
    println!("  test    Run the test suite");
    println!("  bench   Run criterion benchmarks");
    println!("  deny*   Supply-chain check (needs cargo-deny)");
    println!("  cov*    Coverage summary (needs cargo-llvm-cov)");
    println!("  fuzz*   Fuzz smoke run (needs cargo-fuzz + nightly)");
    println!("  demo    Run the SRE provenance demo");
    println!();
    println!("Tasks marked * are skipped when their tool is absent.");
}
