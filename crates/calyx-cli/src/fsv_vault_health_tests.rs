use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::fsv_vault_health::{
    REMEDIATION, REPORT_SCHEMA, SOURCE_OF_TRUTH, VaultHealthReport, failed, parse_args,
};
use crate::fsv_vault_health_marker::{REBUILD_REQUIRED_CODE, check_search_rebuild_marker};
use crate::fsv_vault_health_quarantine::{
    QUARANTINE_FILE, QUARANTINE_INVALID_CODE, QUARANTINE_SCHEMA, check_marker, sha256_hex,
    write_marker,
};

#[test]
fn parse_requires_vault() {
    let error = parse_args(&[]).expect_err("missing vault must fail closed");
    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn parse_rejects_unknown_argument() {
    let args = vec![
        "--vault".to_string(),
        "demo".to_string(),
        "--bad".to_string(),
    ];
    let error = parse_args(&args).expect_err("unknown flag must fail closed");
    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("--bad"), "{}", error.message());
}

#[test]
fn quarantine_marker_round_trips_from_physical_file() {
    let root = temp_root("quarantine-roundtrip");
    fs::create_dir_all(&root).expect("create temp root");
    let marker = root.join(QUARANTINE_FILE);
    let report = VaultHealthReport {
        schema: REPORT_SCHEMA,
        source_of_truth: SOURCE_OF_TRUTH,
        vault_ref: "demo".to_string(),
        vault_dir: root.display().to_string(),
        vault_id: "01KTEST0000000000000000000".to_string(),
        vault_name: "demo".to_string(),
        fsv_ready: false,
        quarantine_required: true,
        quarantine_marker_path: marker.display().to_string(),
        quarantine_marker_sha256: None,
        checks: vec![failed(
            "registry_snapshot_ref",
            "CALYX_ASTER_CORRUPT_SHARD",
            "vault manifest has no persisted registry snapshot ref".to_string(),
            REMEDIATION,
            json!({"manifest_seq": 1}),
        )],
        repair_actions: Vec::new(),
    };

    let write = write_marker(&report).expect("write marker");
    let bytes = fs::read(&marker).expect("read marker source of truth");
    assert_eq!(write.sha256_hex, sha256_hex(&bytes));
    let decoded: Value = serde_json::from_slice(&bytes).expect("decode marker");
    assert_eq!(decoded["schema"], QUARANTINE_SCHEMA);
    assert_eq!(decoded["vault_id"], report.vault_id);
    assert_eq!(
        decoded["failed_checks"][0]["code"],
        "CALYX_ASTER_CORRUPT_SHARD"
    );

    fs::remove_dir_all(root).expect("cleanup temp root");
}

#[test]
fn quarantine_marker_check_fails_closed_on_invalid_schema() {
    let root = temp_root("invalid-quarantine");
    fs::create_dir_all(&root).expect("create temp root");
    let marker = root.join(QUARANTINE_FILE);
    fs::write(&marker, br#"{"schema":"wrong"}"#).expect("write invalid marker");

    let check = check_marker(&marker);
    assert_eq!(check.status, "failed");
    assert_eq!(check.code.as_deref(), Some(QUARANTINE_INVALID_CODE));

    fs::remove_dir_all(root).expect("cleanup temp root");
}

#[test]
fn rebuild_marker_check_reports_recorded_commit_context() {
    let root = temp_root("rebuild-marker");
    fs::create_dir_all(&root).expect("create temp root");

    let clean = check_search_rebuild_marker(&root);
    let mut marker = calyx_search::RebuildRequiredMarker::new(
        "batch_ingest",
        "interrupted ingest left derived indexes unproven",
    )
    .expect("build marker");
    marker.required_base_seq = Some(36982);
    marker.session_id = Some("issue1089-session".to_string());
    calyx_search::write_rebuild_required_marker(&root, &marker).expect("write marker");
    let flagged = check_search_rebuild_marker(&root);

    assert_eq!(clean.status, "ok");
    assert_eq!(flagged.status, "failed");
    assert_eq!(flagged.code.as_deref(), Some(REBUILD_REQUIRED_CODE));
    assert!(flagged.message.contains("required_base_seq=36982"));
    assert!(flagged.message.contains("session_id=issue1089-session"));
    assert_eq!(flagged.details["required_base_seq"], json!(36982));
    assert!(
        flagged
            .remediation
            .as_deref()
            .expect("remediation")
            .contains("rebuild-search-index")
    );

    fs::remove_dir_all(root).expect("cleanup temp root");
}

#[test]
fn rebuild_marker_check_fails_closed_on_corrupt_marker() {
    let root = temp_root("rebuild-marker-corrupt");
    let marker_path = calyx_search::rebuild_required_marker_path(&root);
    fs::create_dir_all(marker_path.parent().expect("marker parent")).expect("create idx/search");
    fs::write(&marker_path, b"{ not json").expect("write corrupt marker");

    let check = check_search_rebuild_marker(&root);

    assert_eq!(check.status, "failed");
    assert_eq!(check.code.as_deref(), Some("CALYX_STALE_DERIVED"));
    assert!(check.message.contains("not valid JSON"));

    fs::remove_dir_all(root).expect("cleanup temp root");
}

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-fsv-vault-health-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ))
}
