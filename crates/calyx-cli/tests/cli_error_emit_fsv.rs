//! FSV: the real `calyx` binary fails closed — a misused command writes the
//! structured error envelope to stderr (NOT stdout) and exits with code 2.
//!
//! This is the source-of-truth read: we execute the compiled binary and
//! inspect its actual exit status and streams rather than trusting a return
//! value. Synthetic known input (a bogus subcommand) → known outcome (exit 2,
//! parseable `{code,message,remediation}` on stderr, empty stdout).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
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
fn missing_turso_database_exits_2_with_byte_identical_calyx_error_envelope() {
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-catalog-error-fsv-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let database = root.join("missing.turso");

    let output = run(&[
        "relational",
        "status",
        database.to_str().expect("Turso path utf-8"),
        "missing-dataset",
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
        code: "CALYX_TURSO_OPEN",
        message: format!(
            "Turso database {} does not exist; initialize it explicitly",
            database.display()
        ),
        remediation: "inspect the database path, permissions, and emitted Turso engine detail",
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

#[test]
fn panel_registry_commands_resolve_real_vault_names_ids_and_paths() {
    let home = temp_home("registry-resolver");
    fs::create_dir_all(&home).expect("create registry resolver home");
    let created = run_home(
        &home,
        &[
            "create-vault",
            "registry-resolver-real-vault",
            "--panel-template",
            "text-default",
        ],
    );
    assert_success(&created, "create real durable vault");
    let created_json: serde_json::Value =
        serde_json::from_slice(&created.stdout).expect("parse create-vault JSON");
    let vault_id = created_json["vault_id"].as_str().expect("vault id");
    let vault = home.join("vaults").join(vault_id);
    let canonical_vault = vault.canonicalize().expect("canonical real vault");
    let before = tree_hash(&home);

    for reference in [
        "registry-resolver-real-vault".to_string(),
        vault_id.to_string(),
        canonical_vault.display().to_string(),
    ] {
        let audit = run_home(&home, &["panel", "registry-audit", "--vault", &reference]);
        assert_success(&audit, &format!("audit vault reference {reference}"));
        let report: serde_json::Value =
            serde_json::from_slice(&audit.stdout).expect("parse registry audit JSON");
        assert_eq!(report["status"], "registry_contracts_valid");
        assert_eq!(report["valid"], true);
        assert_eq!(
            report["vault"].as_str(),
            Some(canonical_vault.to_str().expect("canonical vault UTF-8"))
        );
    }

    let repair = run_home(
        &home,
        &[
            "panel",
            "registry-repair",
            "--vault",
            "registry-resolver-real-vault",
            "--all",
        ],
    );
    assert_success(&repair, "no-op repair by vault name");
    let repair_report: serde_json::Value =
        serde_json::from_slice(&repair.stdout).expect("parse registry repair JSON");
    assert_eq!(repair_report["status"], "registry_contract_repair_noop");
    assert_eq!(repair_report["wrote_manifest"], false);
    assert_eq!(
        repair_report["vault"].as_str(),
        Some(canonical_vault.to_str().expect("canonical vault UTF-8"))
    );

    let after = tree_hash(&home);
    println!(
        "REGISTRY_RESOLVER_HAPPY source_of_truth={} before={} after={} audit_forms=3 repair_noop=true",
        canonical_vault.display(),
        before,
        after
    );
    assert_eq!(
        before, after,
        "read-only audits and no-op repair mutated disk"
    );
    fs::remove_dir_all(home).expect("cleanup registry resolver home");
}

#[test]
fn panel_registry_invalid_references_fail_before_storage_and_never_mutate() {
    let home = temp_home("registry-resolver-edges");
    fs::create_dir_all(&home).expect("create registry edge home");
    let before_unknown = tree_hash(&home);
    let unknown = run_home(
        &home,
        &[
            "panel",
            "registry-audit",
            "--vault",
            "definitely-unknown-vault",
        ],
    );
    assert_typed_failure(&unknown, "CALYX_VAULT_ACCESS_DENIED");
    let after_unknown = tree_hash(&home);
    println!("REGISTRY_RESOLVER_EDGE_UNKNOWN before={before_unknown} after={after_unknown}");
    assert_eq!(before_unknown, after_unknown);

    let missing_path = home.join("vaults").join("01AAAAAAAAAAAAAAAAAAAAAAAA");
    let before_missing_path = tree_hash(&home);
    let explicit = missing_path.display().to_string();
    let missing = run_home(
        &home,
        &["panel", "registry-repair", "--vault", &explicit, "--all"],
    );
    assert_typed_failure(&missing, "CALYX_VAULT_ACCESS_DENIED");
    let after_missing_path = tree_hash(&home);
    println!(
        "REGISTRY_RESOLVER_EDGE_MISSING_PATH before={before_missing_path} after={after_missing_path}"
    );
    assert_eq!(before_missing_path, after_missing_path);

    let created = run_home(
        &home,
        &[
            "create-vault",
            "registry-missing-current",
            "--panel-template",
            "text-default",
        ],
    );
    assert_success(&created, "create vault for missing CURRENT edge");
    let created_json: serde_json::Value =
        serde_json::from_slice(&created.stdout).expect("parse edge create JSON");
    let vault_id = created_json["vault_id"].as_str().expect("edge vault id");
    let current = home.join("vaults").join(vault_id).join("CURRENT");
    fs::remove_file(&current).expect("remove CURRENT to create known invalid physical state");
    let before_missing_current = tree_hash(&home);
    let missing_current = run_home(&home, &["panel", "registry-audit", "--vault", vault_id]);
    assert_typed_failure(&missing_current, "CALYX_ASTER_MANIFEST_MISSING");
    let after_missing_current = tree_hash(&home);
    println!(
        "REGISTRY_RESOLVER_EDGE_MISSING_CURRENT before={before_missing_current} after={after_missing_current} current_exists={} ",
        current.exists()
    );
    assert_eq!(before_missing_current, after_missing_current);
    assert!(
        !current.exists(),
        "audit must not synthesize missing CURRENT"
    );
    fs::remove_dir_all(home).expect("cleanup registry edge home");
}

fn run_home(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .env("CALYX_HOME", home)
        .args(args)
        .output()
        .expect("spawn calyx binary with isolated home")
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context}: status={:?} stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_typed_failure(output: &Output, expected_code: &str) {
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let envelope: serde_json::Value = serde_json::from_str(
        stderr
            .lines()
            .last()
            .expect("typed failure must emit a stderr envelope"),
    )
    .expect("parse typed stderr envelope");
    assert_eq!(envelope["code"], expected_code, "stderr={stderr}");
    assert!(
        envelope["message"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "stderr={stderr}"
    );
    assert!(
        envelope["remediation"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "stderr={stderr}"
    );
}

fn temp_home(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-cli-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ))
}

fn tree_hash(root: &Path) -> String {
    fn walk(root: &Path, path: &Path, rows: &mut Vec<(String, Vec<u8>)>) {
        let mut entries = fs::read_dir(path)
            .expect("read tree directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("read tree entries");
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let child = entry.path();
            let relative = child
                .strip_prefix(root)
                .expect("tree member under root")
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type().expect("read member type").is_dir() {
                rows.push((format!("d:{relative}"), Vec::new()));
                walk(root, &child, rows);
            } else {
                rows.push((
                    format!("f:{relative}"),
                    fs::read(child).expect("read tree file"),
                ));
            }
        }
    }
    let mut rows = Vec::new();
    walk(root, root, &mut rows);
    let mut hasher = blake3::Hasher::new();
    for (name, bytes) in rows {
        hasher.update(name.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    hasher.finalize().to_hex().to_string()
}
