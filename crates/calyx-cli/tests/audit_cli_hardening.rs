use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use calyx_aster::manifest::{
    ImmutableRef, ManifestStore, QuarantineRecord, VaultManifest, is_vault_seq_quarantined,
};
use calyx_aster::sst::write_sst;
use calyx_core::{CxId, FixedClock, LensId};
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, MemoryLedgerStore, SubjectId,
};
use serde_json::{Value, json};

#[test]
#[ignore = "manual aiwonder FSV for issue #349 audit-query quarantine hardening"]
fn issue349_audit_query_quarantine_filter_fsv_writes_readbacks() {
    let root = std::env::var("CALYX_AUDIT_ISSUE349_FSV_DIR")
        .map(PathBuf::from)
        .expect("CALYX_AUDIT_ISSUE349_FSV_DIR is required");
    reset_dir(&root);
    let audit_vault = root.join("audit-vault");
    let cx_vault = root.join("cx-vault");
    let audit_rows = write_issue349_vault(&audit_vault, true);
    let cx_rows = write_issue349_vault(&cx_vault, false);

    let audit_request = json!({"command": "audit", "kind": "ingest"});
    let matching_quarantine_request = json!({"command": "audit", "kind": "measure"});
    let provenance_request = json!({"command": "get-provenance", "cx": cx(1)});
    let audit_ingest = run(["audit", "--vault"], &audit_vault, ["--kind", "ingest"]);
    let audit_measure = run(["audit", "--vault"], &audit_vault, ["--kind", "measure"]);
    let provenance = run(
        ["get-provenance", "--vault"],
        &cx_vault,
        ["--cx", &cx(1).to_string()],
    );

    let manifest =
        serde_json::from_slice::<Value>(&fs::read(audit_vault.join("MANIFEST")).unwrap())
            .expect("manifest json");
    let readback = json!({
        "audit_vault": audit_vault,
        "cx_vault": cx_vault,
        "audit_ledger_rows": audit_rows,
        "cx_ledger_rows": cx_rows,
        "quarantine_manifest": manifest,
        "quarantined_seq_1": is_vault_seq_quarantined(root.join("audit-vault"), 1).unwrap(),
        "audit_request": audit_request,
        "matching_quarantine_request": matching_quarantine_request,
        "provenance_request": provenance_request,
        "audit_ingest_status": audit_ingest.status.code(),
        "audit_ingest_stdout": json_stdout(&audit_ingest),
        "audit_measure_status": audit_measure.status.code(),
        "audit_measure_stderr": stderr(&audit_measure),
        "provenance_status": provenance.status.code(),
        "provenance_stdout": json_stdout(&provenance),
    });
    write_json(&root, "issue349-readback.json", &readback);
    write_json(&root, "audit-filter-ingest.json", &audit_request);
    write_json(
        &root,
        "audit-filter-measure-quarantined.json",
        &matching_quarantine_request,
    );
    write_json(&root, "provenance-cx1-request.json", &provenance_request);
    write_json(
        &root,
        "audit-ingest-result.json",
        &json_stdout(&audit_ingest),
    );
    write_json(
        &root,
        "audit-measure-error.json",
        &json!({"status": audit_measure.status.code(), "stderr": stderr(&audit_measure)}),
    );
    write_json(
        &root,
        "provenance-cx1-result.json",
        &json_stdout(&provenance),
    );

    assert_success(&audit_ingest);
    assert!(!audit_measure.status.success());
    assert_success(&provenance);
    assert_eq!(readback["quarantined_seq_1"], true);
    assert_eq!(readback["audit_ingest_stdout"].as_array().unwrap().len(), 2);
    assert!(stderr(&audit_measure).contains("CALYX_LEDGER_CHAIN_BROKEN"));
    assert_eq!(
        readback["provenance_stdout"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["seq"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 4]
    );

    println!(
        "ISSUE349_AUDIT_FSV root={} ingest={} provenance={} measure_status={:?}",
        root.display(),
        readback["audit_ingest_stdout"].as_array().unwrap().len(),
        readback["provenance_stdout"].as_array().unwrap().len(),
        audit_measure.status.code(),
    );
}

fn write_issue349_vault(vault: &Path, quarantine: bool) -> Value {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(3_490)).unwrap();
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(1)),
        json!({"cx_id": cx(1).to_string()}),
    );
    append_json(
        &mut appender,
        EntryKind::Measure,
        SubjectId::Cx(cx(2)),
        json!({"cx_id": cx(2).to_string(), "surface": "quarantined"}),
    );
    append_json(
        &mut appender,
        EntryKind::Ingest,
        SubjectId::Cx(cx(3)),
        json!({"cx_id": cx(3).to_string()}),
    );
    append_json(
        &mut appender,
        EntryKind::Measure,
        SubjectId::Lens(lens(9)),
        json!({
            "comment": cx(1).to_string(),
            "nested": {"note": cx(1).to_string()},
            "array": [cx(1).to_string()]
        }),
    );
    append_json(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(b"issue349-path".to_vec()),
        json!({"path": [{"from_id": cx(1).to_string(), "to_id": cx(2).to_string()}]}),
    );
    write_ledger_sst(vault, appender.into_store(), quarantine)
}

fn write_ledger_sst(vault: &Path, store: MemoryLedgerStore, quarantine: bool) -> Value {
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
    if quarantine {
        write_quarantine_manifest(vault, 1, 2, rows.len() as u64);
    }
    json!({
        "rows": rows
            .iter()
            .map(|row| json!({"seq": row.seq, "bytes_hex": hex(&row.bytes)}))
            .collect::<Vec<_>>()
    })
}

fn write_quarantine_manifest(vault: &Path, start: u64, end: u64, durable_seq: u64) {
    let panel_bytes = b"issue349-audit-panel";
    let codebook_bytes = b"issue349-audit-codebook";
    write_manifest_asset(vault, "panel/issue349-panel.bin", panel_bytes);
    write_manifest_asset(vault, "codebooks/issue349-codebook.bin", codebook_bytes);
    let mut manifest = VaultManifest::new(
        1,
        durable_seq,
        ImmutableRef::from_bytes("panel/issue349-panel.bin", panel_bytes).unwrap(),
        vec![ImmutableRef::from_bytes("codebooks/issue349-codebook.bin", codebook_bytes).unwrap()],
    )
    .unwrap();
    manifest
        .quarantines
        .push(QuarantineRecord::new(start, end, start, 3_490).expect("quarantine record"));
    ManifestStore::open(vault)
        .write_current(&manifest)
        .expect("write quarantine manifest");
}

fn write_manifest_asset(vault: &Path, logical_path: &str, bytes: &[u8]) {
    let path = vault.join(logical_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
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
            ActorId::Service("issue349-audit-fsv".to_string()),
        )
        .unwrap();
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

fn reset_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
}

fn write_json(root: &Path, name: &str, value: &Value) {
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).expect("serialize json"),
    )
    .expect("write json");
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
