// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=fsv_support visibility=private
use crate::__calyx_shared_fsv_support_rs as fsv_support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::FixedClock;
use calyx_ledger::{
    DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore, LedgerEntry, decode,
};
use calyx_poly::{
    LocalOnlyPolicy, POLICY_AUDIT_SCHEMA_VERSION, PolyAction, PolyError,
    write_policy_config_snapshot, write_policy_guarded_artifact,
};
use serde_json::{Value, json};

use fsv_support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const TEST_TS: u64 = 1_786_400_162;

#[test]
fn issue162_no_trade_runtime_policy_fsv() {
    let (root, _env_supplied) =
        named_fsv_root("POLY_ISSUE162_FSV_ROOT", "issue162-no-trade-policy-fsv");
    reset_dir(&root);

    let policy = LocalOnlyPolicy::default();
    let config_path = write_policy_config_snapshot(&root, &policy).expect("write policy config");
    let config_readback: Value =
        serde_json::from_slice(&fs::read(&config_path).expect("read policy config"))
            .expect("decode policy config");
    assert_eq!(
        config_readback["schema_version"],
        POLICY_AUDIT_SCHEMA_VERSION
    );
    assert_eq!(
        config_readback["forbidden_trading_actions"]
            .as_array()
            .expect("forbidden action array")
            .len(),
        PolyAction::FORBIDDEN_TRADING_ACTIONS.len()
    );

    let ledger_dir = root.join("ledger");
    let artifact_root = root.join("artifacts");
    let trading_root = root.join("trading-attempts");
    fs::create_dir_all(&artifact_root).expect("create artifact root");
    fs::create_dir_all(&trading_root).expect("create trading root");
    let mut ledger = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).expect("open policy ledger"),
        FixedClock::new(TEST_TS),
    )
    .expect("open ledger appender");

    let happy_relative = Path::new("happy/read-public-observation.json");
    let happy_path = artifact_root.join(happy_relative);
    let happy_before = state_snapshot(&ledger_dir, &happy_path);
    let happy_enforcement = write_policy_guarded_artifact(
        &artifact_root,
        &mut ledger,
        &policy,
        PolyAction::ReadPublicData,
        happy_relative,
        br#"{"source":"synthetic-known-truth","expected":"local-read-only-observation"}"#,
    )
    .expect("read-only action should be allowed and logged");
    let happy_after = state_snapshot(&ledger_dir, &happy_path);
    assert!(happy_after["artifact_exists"].as_bool().unwrap());
    assert_eq!(happy_after["ledger_rows"].as_u64(), Some(1));
    assert!(happy_enforcement.decision.allowed);
    assert_eq!(happy_enforcement.ledger_ref.seq, 0);

    let happy_entries = read_ledger_entries(&ledger_dir);
    let happy_payload = policy_payload(happy_entries.last().expect("happy ledger row"));
    assert_eq!(happy_payload["action"], "read_public_data");
    assert_eq!(happy_payload["allowed"], true);
    assert_eq!(payload_code(&happy_payload), "CALYX_POLY_POLICY_ALLOWED");

    let mut edges = Vec::new();
    for action in PolyAction::FORBIDDEN_TRADING_ACTIONS {
        let expected = policy.enforce(action);
        let relative = PathBuf::from(format!("{}.json", action.as_str()));
        let artifact_path = trading_root.join(&relative);
        let before = state_snapshot(&ledger_dir, &artifact_path);
        let err = write_policy_guarded_artifact(
            &trading_root,
            &mut ledger,
            &policy,
            action,
            &relative,
            br#"{"should_not_exist":true}"#,
        )
        .expect_err("forbidden trading action must fail closed");
        let (code, message) = policy_error(err);
        assert_eq!(code, expected.code);
        let after = state_snapshot(&ledger_dir, &artifact_path);
        assert!(!after["artifact_exists"].as_bool().unwrap());
        assert_eq!(
            after["ledger_rows"].as_u64().unwrap(),
            before["ledger_rows"].as_u64().unwrap() + 1
        );

        let entries = read_ledger_entries(&ledger_dir);
        let entry = entries.last().expect("policy ledger row");
        let payload = policy_payload(entry);
        assert_eq!(entry.kind, EntryKind::Policy);
        assert_eq!(payload["action"], action.as_str());
        assert_eq!(payload["allowed"], false);
        assert_eq!(payload_code(&payload), expected.code);
        assert_eq!(payload["reason"], expected.reason);

        edges.push(json!({
            "action": action.as_str(),
            "expected_code": expected.code,
            "error_code": code,
            "error_message": message,
            "before": before,
            "after": after,
            "ledger_seq": entry.seq,
            "artifact_absent_after_refusal": !artifact_path.exists()
        }));
    }

    let final_entries = read_ledger_entries(&ledger_dir);
    assert_eq!(
        final_entries.len(),
        1 + PolyAction::FORBIDDEN_TRADING_ACTIONS.len()
    );
    assert!(
        final_entries
            .iter()
            .all(|entry| entry.kind == EntryKind::Policy)
    );
    assert!(trading_artifact_files(&trading_root).is_empty());

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "source_of_truth": {
            "policy_config": config_path.display().to_string(),
            "ledger_dir": ledger_dir.display().to_string(),
            "artifact_root": artifact_root.display().to_string(),
            "trading_attempt_root": trading_root.display().to_string()
        },
        "config_readback": config_readback,
        "happy_path": {
            "action": "read_public_data",
            "before": happy_before,
            "after": happy_after,
            "ledger_ref_seq": happy_enforcement.ledger_ref.seq,
            "ledger_payload": happy_payload,
            "artifact_readback": serde_json::from_slice::<Value>(
                &fs::read(&happy_path).expect("read happy artifact")
            ).expect("decode happy artifact")
        },
        "edge_cases": edges,
        "final": {
            "ledger_rows": final_entries.len(),
            "last_ledger_hash": hex(&final_entries.last().expect("last ledger row").entry_hash),
            "trading_artifact_files": trading_artifact_files(&trading_root),
            "files": files
        }
    });
    write_json(
        &root.join("issue162-no-trade-policy-fsv-summary.json"),
        &summary,
    );
    write_blake3sums(&root);
}

fn state_snapshot(ledger_dir: &Path, artifact_path: &Path) -> Value {
    let metadata = fs::metadata(artifact_path).ok();
    json!({
        "ledger_rows": read_ledger_entries(ledger_dir).len(),
        "artifact_path": artifact_path.display().to_string(),
        "artifact_exists": metadata.is_some(),
        "artifact_bytes": metadata.map(|meta| meta.len())
    })
}

fn read_ledger_entries(ledger_dir: &Path) -> Vec<LedgerEntry> {
    let store = DirectoryLedgerStore::open(ledger_dir).expect("open ledger for readback");
    store
        .scan()
        .expect("scan physical ledger rows")
        .into_iter()
        .map(|row| decode(&row.bytes).expect("decode physical ledger row"))
        .collect()
}

fn policy_payload(entry: &LedgerEntry) -> Value {
    serde_json::from_slice(&entry.payload).expect("decode policy payload")
}

fn payload_code(payload: &Value) -> String {
    payload["code_parts"]
        .as_array()
        .expect("code parts")
        .iter()
        .map(|part| part.as_str().expect("code part string"))
        .collect::<Vec<_>>()
        .join("_")
}

fn policy_error(err: PolyError) -> (String, String) {
    match err {
        PolyError::Policy { code, message } => (code, message),
        other => panic!("expected policy error, got {other:?}"),
    }
}

fn trading_artifact_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_trading_files(root, &mut files);
    files.sort();
    files
}

fn collect_trading_files(path: &Path, out: &mut Vec<String>) {
    if !path.exists() {
        return;
    }
    for entry in fs::read_dir(path).expect("read trading artifact dir") {
        let path = entry.expect("read trading artifact entry").path();
        if path.is_dir() {
            collect_trading_files(&path, out);
        } else {
            out.push(path.display().to_string());
        }
    }
}
