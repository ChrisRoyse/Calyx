mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use calyx_aster::manifest::{
    ImmutableRef, ManifestStore, QuarantineRecord, VaultManifest, is_vault_seq_quarantined,
};
use calyx_aster::sst::write_sst;
use calyx_core::{CxId, FixedClock, LensId, SlotId};
use calyx_ledger::{
    ActorId, EntryKind, FusionMode, FusionWeights, LedgerAppender, LedgerCfStore,
    MemoryLedgerStore, SlotWeight, SubjectId,
};
use serde_json::{Value, json};
use support::fsv_io::{
    fsv_root, list_files, numbered_temp_root, reset_dir, write_json, write_manifest_asset,
};

#[test]
fn audit_cli_commands_print_expected_json() {
    let root = test_dir("audit-cli");
    reset_dir(&root);
    let vault = root.join("vault");
    write_vault_ledger(&vault);

    let provenance = run(
        ["get-provenance", "--vault"],
        &vault,
        ["--cx", &cx(1).to_string()],
    );
    let trace = run(
        ["get-answer-trace", "--vault"],
        &vault,
        ["--answer", "audit-answer"],
    );
    let audit = run(["audit", "--vault"], &vault, ["--kind", "ingest"]);

    assert_success(&provenance);
    assert_success(&trace);
    assert_success(&audit);
    assert_eq!(json_stdout(&provenance).as_array().unwrap().len(), 5);
    assert_eq!(json_stdout(&trace)["complete"], true);
    assert_eq!(json_stdout(&trace)["path"].as_array().unwrap().len(), 2);
    assert_eq!(json_stdout(&audit).as_array().unwrap().len(), 3);
    cleanup(root);
}

#[test]
fn audit_cli_fails_closed_on_manifest_quarantine() {
    let root = test_dir("audit-cli-quarantine");
    reset_dir(&root);
    let vault = root.join("vault");
    write_vault_ledger(&vault);
    write_quarantine_manifest(&vault, 8, 9, 8);

    let trace = run(
        ["get-answer-trace", "--vault"],
        &vault,
        ["--answer", "audit-answer"],
    );

    assert!(!trace.status.success());
    assert!(stderr(&trace).contains("CALYX_LEDGER_CHAIN_BROKEN"));
    assert!(is_vault_seq_quarantined(&vault, 8).unwrap());
    cleanup(root);
}

#[test]
#[ignore = "manual FSV for PH36 audit query CLI surface"]
fn ph36_audit_query_cli_manual_fsv() {
    let root = fsv_root("CALYX_FSV_ROOT", "calyx-ph36-audit-query-fsv").join("audit-query-surface");
    reset_dir(&root);
    let vault = root.join("vault");
    let partial_vault = root.join("partial-vault");
    let quarantined_vault = root.join("quarantined-vault");
    write_vault_ledger(&vault);
    write_partial_vault_ledger(&partial_vault);
    write_vault_ledger(&quarantined_vault);
    write_quarantine_manifest(&quarantined_vault, 8, 9, 8);

    let provenance = run(
        ["get-provenance", "--vault"],
        &vault,
        ["--cx", &cx(1).to_string()],
    );
    let trace = run(
        ["get-answer-trace", "--vault"],
        &vault,
        ["--answer", "audit-answer"],
    );
    let audit = run(["audit", "--vault"], &vault, ["--kind", "ingest"]);
    let partial = run(
        ["get-answer-trace", "--vault"],
        &partial_vault,
        ["--answer", &hex(cx(40).as_bytes())],
    );
    let bad_kind = run(["audit", "--vault"], &vault, ["--kind", "bogus"]);
    let quarantined_trace = run(
        ["get-answer-trace", "--vault"],
        &quarantined_vault,
        ["--answer", "audit-answer"],
    );

    let readback = json!({
        "vault": vault,
        "partial_vault": partial_vault,
        "quarantined_vault": quarantined_vault,
        "ledger_sst_files": list_files(&root.join("vault").join("cf").join("ledger")),
        "quarantined_ledger_sst_files": list_files(&root.join("quarantined-vault").join("cf").join("ledger")),
        "quarantined_current": fs::read_to_string(root.join("quarantined-vault").join("CURRENT")).unwrap(),
        "quarantined_manifest": serde_json::from_slice::<Value>(
            &fs::read(root.join("quarantined-vault").join("MANIFEST")).unwrap()
        ).unwrap(),
        "quarantined_seq_8": is_vault_seq_quarantined(root.join("quarantined-vault"), 8).unwrap(),
        "provenance_stdout": json_stdout(&provenance),
        "trace_stdout": json_stdout(&trace),
        "audit_stdout": json_stdout(&audit),
        "partial_trace_stdout": json_stdout(&partial),
        "bad_kind_status": bad_kind.status.code(),
        "bad_kind_stderr": stderr(&bad_kind),
        "quarantined_trace_status": quarantined_trace.status.code(),
        "quarantined_trace_stderr": stderr(&quarantined_trace),
    });
    let readback_path = root.join("audit-query-readback.json");
    write_json(&readback_path, &readback);

    println!("PH36_AUDIT_QUERY_FSV_ROOT={}", root.display());
    println!("PH36_AUDIT_QUERY_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_success(&provenance);
    assert_success(&trace);
    assert_success(&audit);
    assert_success(&partial);
    assert_eq!(readback["provenance_stdout"].as_array().unwrap().len(), 5);
    assert_eq!(readback["trace_stdout"]["complete"], true);
    assert_eq!(
        readback["trace_stdout"]["path"].as_array().unwrap().len(),
        2
    );
    assert_eq!(readback["audit_stdout"].as_array().unwrap().len(), 3);
    assert_eq!(readback["partial_trace_stdout"]["complete"], false);
    assert!(stderr(&bad_kind).contains("invalid --kind"));
    assert_eq!(readback["quarantined_seq_8"], true);
    assert!(!quarantined_trace.status.success());
    assert!(stderr(&quarantined_trace).contains("CALYX_LEDGER_CHAIN_BROKEN"));
}

fn write_vault_ledger(vault: &Path) {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(1_000)).unwrap();
    for seed in 1..=3 {
        append_json(
            &mut appender,
            EntryKind::Ingest,
            SubjectId::Cx(cx(seed)),
            json!({"cx_id": cx(seed).to_string()}),
        );
    }
    for kind in [EntryKind::Measure, EntryKind::Assay, EntryKind::Guard] {
        append_json(
            &mut appender,
            kind,
            SubjectId::Cx(cx(1)),
            json!({"cx_id": cx(1).to_string(), "surface": kind.as_str()}),
        );
    }
    append_json(
        &mut appender,
        EntryKind::Kernel,
        SubjectId::Kernel(cx(88).as_bytes().to_vec()),
        json!({"kernel_id": cx(88).to_string(), "recall_ratio": 0.99}),
    );
    append_json(
        &mut appender,
        EntryKind::Guard,
        SubjectId::Guard(b"audit-guard".to_vec()),
        json!({"guard_id": "audit-guard", "pass": true, "tau": 0.8}),
    );
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(b"audit-answer".to_vec()),
        json!({
            "complete": true,
            "expected_hops": 2,
            "kernel_id": cx(88).to_string(),
            "guard_id": "audit-guard",
            "path": [
                {"from_id": cx(1).to_string(), "cx_id": cx(2).to_string(), "hop": 0, "score": 0.9, "lens_id": lens(1).to_string()},
                {"from_id": cx(2).to_string(), "cx_id": cx(3).to_string(), "hop": 1, "score": 0.7, "lens_id": lens(2).to_string()}
            ],
            "fusion_weights": fusion_weights(),
            "guard_result": {"pass": true},
            "freshness_ts": 1234
        }),
    );
    write_ledger_sst(vault, appender.into_store());
}

fn write_partial_vault_ledger(vault: &Path) {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(2_000)).unwrap();
    let answer_id = cx(40).as_bytes().to_vec();
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(answer_id.clone()),
        json!({"from_id": cx(10).to_string(), "to_id": cx(11).to_string(), "hop_index": 0, "hop_score": 0.8}),
    );
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(answer_id),
        json!({"from_id": cx(11).to_string(), "to_id": cx(12).to_string(), "hop_index": 1, "hop_score": 0.6}),
    );
    write_ledger_sst(vault, appender.into_store());
}

fn write_ledger_sst(vault: &Path, store: MemoryLedgerStore) {
    let ledger_dir = vault.join("cf").join("ledger");
    fs::create_dir_all(&ledger_dir).unwrap();
    let rows = store.scan().unwrap();
    let entries = rows
        .iter()
        .map(|row| (row.seq.to_be_bytes().to_vec(), row.bytes.clone()))
        .collect::<Vec<_>>();
    write_sst(
        ledger_dir.join("00000000000000000001.sst"),
        entries
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice())),
    )
    .unwrap();
}

fn write_quarantine_manifest(vault: &Path, start: u64, end: u64, broken: u64) {
    let panel_bytes = b"ph36-audit-query-panel";
    let codebook_bytes = b"ph36-audit-query-codebook";
    write_manifest_asset(vault, "panel/audit-panel.bin", panel_bytes);
    write_manifest_asset(vault, "codebooks/audit-codebook.bin", codebook_bytes);
    let mut manifest = VaultManifest::new(
        1,
        end,
        ImmutableRef::from_bytes("panel/audit-panel.bin", panel_bytes).unwrap(),
        vec![ImmutableRef::from_bytes("codebooks/audit-codebook.bin", codebook_bytes).unwrap()],
    )
    .unwrap();
    manifest
        .quarantines
        .push(QuarantineRecord::new(start, end, broken, 3_000).expect("quarantine record"));
    ManifestStore::open(vault)
        .write_current(&manifest)
        .expect("write quarantine manifest");
}

fn append_json<S, C>(
    appender: &mut LedgerAppender<S, C>,
    kind: EntryKind,
    subject: SubjectId,
    value: Value,
) where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    appender
        .append(
            kind,
            subject,
            serde_json::to_vec(&value).unwrap(),
            ActorId::Service("audit-cli-test".to_string()),
        )
        .unwrap();
}

fn fusion_weights() -> FusionWeights {
    FusionWeights {
        mode: FusionMode::WeightedRrf,
        k: 2,
        candidates: vec![cx(1), cx(2)],
        weights: vec![SlotWeight {
            slot_id: SlotId::new(0),
            weight: 1.0,
        }],
        single_slot: None,
    }
}

fn run<const A: usize, const B: usize>(
    prefix: [&str; A],
    path: &Path,
    suffix: [&str; B],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_calyx"));
    for arg in prefix {
        command.arg(arg);
    }
    command.arg(path);
    for arg in suffix {
        command.arg(arg);
    }
    command.output().expect("run calyx")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
}

fn json_stdout(output: &Output) -> Value {
    serde_json::from_str(&stdout(output)).expect("stdout json")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn test_dir(name: &str) -> PathBuf {
    numbered_temp_root("calyx", name)
}

fn cleanup(path: PathBuf) {
    if path.starts_with(std::env::temp_dir()) {
        let _ = fs::remove_dir_all(path);
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn lens(seed: u8) -> LensId {
    LensId::from_bytes([seed; 16])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
