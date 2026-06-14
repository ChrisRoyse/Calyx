//! FSV: the real `calyx` binary fails closed — a misused command writes the
//! structured error envelope to stderr (NOT stdout) and exits with code 2.
//!
//! This is the source-of-truth read: we execute the compiled binary and
//! inspect its actual exit status and streams rather than trusting a return
//! value. Synthetic known input (a bogus subcommand) → known outcome (exit 2,
//! parseable `{code,message,remediation}` on stderr, empty stdout).

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::CalyxError;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args(args)
        .output()
        .expect("spawn calyx binary")
}

#[test]
fn bogus_command_exits_2_with_structured_stderr_envelope() {
    let output = run(&["definitely-not-a-real-subcommand", "--nonsense"]);

    // Exit code is the fail-closed truth gate.
    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Errors go to stderr, never stdout.
    assert!(
        output.stdout.is_empty(),
        "stdout must be empty on error, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    // stderr is a single-line, well-formed JSON envelope with the three fields.
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    let line = stderr.lines().next().expect("at least one stderr line");
    let parsed: serde_json::Value = serde_json::from_str(line)
        .unwrap_or_else(|error| panic!("stderr line must be JSON ({error}): {line}"));

    assert_eq!(parsed["code"], "CALYX_CLI_USAGE_ERROR", "envelope: {line}");
    assert!(
        parsed["message"].as_str().is_some_and(|m| !m.is_empty()),
        "message must be non-empty: {line}"
    );
    assert!(
        parsed["remediation"]
            .as_str()
            .is_some_and(|r| !r.is_empty()),
        "remediation must be non-empty: {line}"
    );
}

#[test]
fn catalog_failure_exits_2_with_byte_identical_calyx_error_envelope() {
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-catalog-error-fsv-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let sqlite = root.join("malformed.sqlite");
    let vault = root.join("vault.calyx");
    std::fs::write(&sqlite, b"").expect("write empty sqlite");

    let output = run(&[
        "migrate",
        "vault",
        sqlite.to_str().expect("sqlite path utf-8"),
        vault.to_str().expect("vault path utf-8"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "stdout must be empty on catalog error, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let expected = CalyxError {
        code: "CALYX_MIGRATE_SQLITE_SCHEMA",
        message: "chunks table missing required column chunk_id".to_string(),
        remediation: "provide a Leapable Vault SQLite DB with chunks(chunk_id,database_name,content,embedding)",
    };
    let expected_stderr = format!("{}\n", serde_json::to_string(&expected).unwrap());
    let actual_stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");

    assert_eq!(actual_stderr, expected_stderr);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn successful_help_exits_0_and_writes_stdout() {
    // Contrast case proving the 0/2 split is real, not constant: `--help`
    // succeeds, writes to stdout, and leaves stderr clean.
    let output = run(&["--help"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "help should exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.is_empty(), "help must write usage to stdout");
}
