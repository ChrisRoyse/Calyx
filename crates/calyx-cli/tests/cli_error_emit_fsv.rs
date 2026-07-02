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

/// Regression test for issue #1145: a storage-layer typed `CalyxError`
/// propagating out of a command must reach stderr verbatim — code, message,
/// and remediation — instead of being stringified and re-wrapped as
/// `CALYX_CLI_USAGE_ERROR` with the generic "run --help" remediation.
///
/// Synthetic known input: a CF directory whose file names trip the
/// seq-domain order gate (legacy flush ordinal 2 > commit-domain seq 1, the
/// exact layout from issue #1138). The gate is name-based, so the files need
/// no valid SST content. Known outcome: exit 2, stderr envelope carrying
/// `CALYX_ASTER_SST_ORDER_AMBIGUOUS` and its typed remediation.
#[test]
fn storage_error_envelope_preserves_typed_code_and_remediation() {
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-issue1145-fsv-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    let base = root.join("cf").join("base");
    std::fs::create_dir_all(&base).expect("create cf/base");
    std::fs::write(base.join("00000000000000000002.sst"), b"legacy").expect("legacy flush");
    std::fs::write(base.join("compacted-00000000000000000001.sst"), b"commit")
        .expect("commit-domain sst");

    // Source of truth for the expected typed error: the gate itself.
    let expected = calyx_aster::storage_names::ensure_unambiguous_sst_order(
        [
            base.join("00000000000000000002.sst"),
            base.join("compacted-00000000000000000001.sst"),
        ]
        .iter()
        .map(|path| path.as_path()),
    )
    .expect_err("synthetic layout must trip the order gate");
    assert_eq!(expected.code, "CALYX_ASTER_SST_ORDER_AMBIGUOUS");

    let output = run(&[
        "readback",
        "--cf",
        "base",
        "--vault",
        root.to_str().expect("vault path utf-8"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "stdout must be empty on error, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    let line = stderr.lines().next().expect("at least one stderr line");
    let parsed: serde_json::Value = serde_json::from_str(line)
        .unwrap_or_else(|error| panic!("stderr line must be JSON ({error}): {line}"));

    // The typed code must survive as the envelope `code`, not be demoted
    // into the message of a usage error.
    assert_eq!(parsed["code"], expected.code, "envelope: {line}");
    assert_eq!(parsed["message"], expected.message, "envelope: {line}");
    assert_eq!(
        parsed["remediation"], expected.remediation,
        "envelope: {line}"
    );
    std::fs::remove_dir_all(root).ok();
}

/// Companion to the #1145 regression: no non-parse failure may ever surface
/// as `CALYX_CLI_USAGE_ERROR`. A corrupt shard behind a canonical SST name is
/// a storage failure; its typed catalog code must reach the envelope.
#[test]
fn corrupt_shard_is_never_reported_as_usage_error() {
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-corrupt-shard-fsv-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    let base = root.join("cf").join("base");
    std::fs::create_dir_all(&base).expect("create cf/base");
    let shard = base.join("00000000000000000001.sst");
    std::fs::write(&shard, b"garbage bytes, not an sst").expect("write corrupt shard");

    // Source of truth: opening the shard in-process yields the typed error.
    let expected = calyx_aster::sst::SstReader::open(&shard)
        .map(|_| ())
        .expect_err("corrupt shard must fail to open");

    let output = run(&[
        "readback",
        "--cf",
        "base",
        "--vault",
        root.to_str().expect("vault path utf-8"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    let line = stderr.lines().next().expect("at least one stderr line");
    let parsed: serde_json::Value = serde_json::from_str(line)
        .unwrap_or_else(|error| panic!("stderr line must be JSON ({error}): {line}"));

    assert_ne!(
        parsed["code"], "CALYX_CLI_USAGE_ERROR",
        "storage failure must not claim command misuse: {line}"
    );
    assert_eq!(parsed["code"], expected.code, "envelope: {line}");
    assert_eq!(
        parsed["remediation"], expected.remediation,
        "envelope: {line}"
    );
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
