use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, FixedClock, SlotId};
use calyx_ledger::{
    ActorId, AuditFilter, DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore,
    LedgerEntry, LedgerRow, MemoryLedgerStore, QuarantineSet, SubjectId, audit, decode,
    get_provenance,
};
use calyx_ward::{
    CalibrationInput, GuardId, GuardPolicy, GuardProfile, NoveltyAction, ProducedSlots, SlotKind,
    WardLedgerError, calibrate_with_ledger, guard_with_ledger,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn ward_ledger_wrappers_append_guard_rows_and_preserve_audit_quarantine_contract() {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(10_000)).unwrap();
    let (profile, calibration_ref) = calibrate_with_ledger(
        &mut appender,
        profile_template(),
        vec![calibration_input(slot(1), SlotKind::Identity, 0.01)],
        0.05,
        &FixedClock::new(20_000),
    )
    .expect("calibrate with ledger");
    appender
        .append(
            EntryKind::Measure,
            SubjectId::Cx(cx(9)),
            serde_json::to_vec(&json!({"cx_id": cx(9).to_string()})).unwrap(),
            ActorId::Service("ward-ledger-test".to_string()),
        )
        .unwrap();
    let (verdict, verdict_ref) = guard_with_ledger(
        &mut appender,
        cx(1),
        &profile,
        &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
        &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
        true,
    )
    .expect("guard with ledger");
    let store = appender.into_store();
    let entries = store
        .scan()
        .unwrap()
        .into_iter()
        .map(|row| decode(&row.bytes).unwrap())
        .collect::<Vec<_>>();
    let calibration_payload = payload_json(&entries[0]);
    let verdict_payload = payload_json(&entries[2]);
    let unrelated_quarantine = QuarantineSet::from_ranges(std::iter::once(1..2)).unwrap();
    let guard_quarantine = QuarantineSet::from_ranges(std::iter::once(2..3)).unwrap();

    let guard_audit = audit(
        &store,
        &unrelated_quarantine,
        AuditFilter {
            kind: Some(EntryKind::Guard),
            ..AuditFilter::default()
        },
    )
    .unwrap();
    let measure_error = audit(
        &store,
        &unrelated_quarantine,
        AuditFilter {
            kind: Some(EntryKind::Measure),
            ..AuditFilter::default()
        },
    )
    .unwrap_err();
    let guard_error = audit(
        &store,
        &guard_quarantine,
        AuditFilter {
            kind: Some(EntryKind::Guard),
            ..AuditFilter::default()
        },
    )
    .unwrap_err();
    let provenance = get_provenance(&store, &QuarantineSet::default(), cx(1)).unwrap();

    assert!(profile.is_calibrated());
    assert!(verdict.overall_pass);
    assert_eq!(calibration_ref.seq, 0);
    assert_eq!(verdict_ref.seq, 2);
    assert_eq!(
        entries.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
        vec![EntryKind::Guard, EntryKind::Measure, EntryKind::Guard]
    );
    assert_eq!(
        calibration_payload["ward_provenance"],
        "ward_calibration_v1"
    );
    assert_eq!(calibration_payload["guard_id"], GUARD_UUID);
    assert_eq!(
        calibration_payload["calibration"]["estimator"],
        "conformal_quantile_v1"
    );
    assert_eq!(verdict_payload["ward_provenance"], "ward_guard_verdict_v1");
    assert_eq!(verdict_payload["cx_id"], cx(1).to_string());
    assert_eq!(verdict_payload["per_slot"][0]["pass"], true);
    assert_eq!(
        guard_audit
            .iter()
            .map(|entry| entry.seq)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert_eq!(measure_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(guard_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(
        provenance.iter().map(|entry| entry.seq).collect::<Vec<_>>(),
        vec![2]
    );
}

#[test]
#[ignore = "manual FSV for issue #279 Ward Ledger provenance"]
fn issue279_ward_ledger_provenance_fsv_writes_readbacks() {
    let root = std::env::var("CALYX_WARD_LEDGER_ISSUE279_FSV_DIR")
        .map(PathBuf::from)
        .expect("CALYX_WARD_LEDGER_ISSUE279_FSV_DIR is required");
    reset_dir(&root);
    let ledger_dir = root.join("ledger-cf");
    fs::create_dir_all(&ledger_dir).unwrap();
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap();
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(27_900),
    )
    .unwrap();
    let (profile, calibration_ref) = calibrate_with_ledger(
        &mut appender,
        profile_template(),
        vec![calibration_input(slot(1), SlotKind::Identity, 0.01)],
        0.05,
        &FixedClock::new(28_000),
    )
    .expect("calibrate with ledger");
    appender
        .append(
            EntryKind::Measure,
            SubjectId::Cx(cx(9)),
            serde_json::to_vec(&json!({"cx_id": cx(9).to_string(), "surface": "unrelated"}))
                .unwrap(),
            ActorId::Service("ward-ledger-fsv".to_string()),
        )
        .unwrap();
    let (verdict, verdict_ref) = guard_with_ledger(
        &mut appender,
        cx(1),
        &profile,
        &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
        &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
        true,
    )
    .expect("guard with ledger");
    let store = appender.into_store();
    let rows = store.scan().unwrap();
    let entries = rows
        .iter()
        .map(|row| decode(&row.bytes).unwrap())
        .collect::<Vec<_>>();
    let unrelated_quarantine = QuarantineSet::from_ranges(std::iter::once(1..2)).unwrap();
    let guard_quarantine = QuarantineSet::from_ranges(std::iter::once(2..3)).unwrap();
    let audit_guard = audit(
        &store,
        &unrelated_quarantine,
        AuditFilter {
            kind: Some(EntryKind::Guard),
            ..AuditFilter::default()
        },
    )
    .unwrap();
    let audit_guard_quarantined = audit(
        &store,
        &guard_quarantine,
        AuditFilter {
            kind: Some(EntryKind::Guard),
            ..AuditFilter::default()
        },
    )
    .unwrap_err();
    let provenance = get_provenance(&store, &QuarantineSet::default(), cx(1)).unwrap();

    write_json(
        &root,
        "issue279-readback.json",
        &json!({
            "ledger_dir": ledger_dir,
            "before_rows": row_readback(&before_rows),
            "after_rows": row_readback(&rows),
            "entries": entry_readback(&entries),
            "calibration_ref": {"seq": calibration_ref.seq, "hash": hex(&calibration_ref.hash)},
            "verdict_ref": {"seq": verdict_ref.seq, "hash": hex(&verdict_ref.hash)},
            "verdict": verdict,
            "quarantine_ranges": {
                "unrelated_measure": "1..2",
                "matching_guard": "2..3",
            },
            "audit_guard_seqs": seqs(&audit_guard),
            "audit_guard_quarantined_error": {
                "code": audit_guard_quarantined.code,
                "message": audit_guard_quarantined.message,
            },
            "provenance_cx1_seqs": seqs(&provenance),
        }),
    );
    write_json(
        &root,
        "audit-kind-guard-request.json",
        &json!({"kind": "guard", "quarantine": "1..2"}),
    );
    write_json(&root, "audit-kind-guard-result.json", &json!(audit_guard));
    write_json(
        &root,
        "audit-kind-guard-quarantined-error.json",
        &json!({
            "code": audit_guard_quarantined.code,
            "message": audit_guard_quarantined.message,
        }),
    );
    write_json(&root, "provenance-cx1-result.json", &json!(provenance));

    assert!(before_rows.is_empty());
    assert!(profile.is_calibrated());
    assert!(verdict.overall_pass);
    assert_eq!(
        rows.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(seqs(&audit_guard), vec![0, 2]);
    assert_eq!(audit_guard_quarantined.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(seqs(&provenance), vec![2]);

    println!(
        "ISSUE279_WARD_LEDGER_FSV root={} rows={} audit_guard={:?} provenance={:?} guard_error={}",
        root.display(),
        rows.len(),
        seqs(&audit_guard),
        seqs(&provenance),
        audit_guard_quarantined.code
    );
}

#[test]
#[ignore = "manual FSV for issue #649 high-stakes slot provenance"]
fn issue649_high_stakes_slot_provenance_fsv_writes_readbacks() {
    let root = std::env::var("CALYX_WARD_ISSUE649_FSV_DIR")
        .map(PathBuf::from)
        .expect("CALYX_WARD_ISSUE649_FSV_DIR is required");
    reset_dir(&root);
    let ledger_dir = root.join("ledger-cf");
    fs::create_dir_all(&ledger_dir).unwrap();
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap();
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(64_900),
    )
    .unwrap();
    let (profile, calibration_ref) = calibrate_with_ledger(
        &mut appender,
        profile_template(),
        vec![calibration_input(slot(1), SlotKind::Identity, 0.01)],
        0.05,
        &FixedClock::new(64_901),
    )
    .expect("calibrate with ledger");
    let (verdict, verdict_ref) = guard_with_ledger(
        &mut appender,
        cx(1),
        &profile,
        &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
        &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
        true,
    )
    .expect("high-stakes guard with ledger");
    let mut missing_slot_meta = profile.clone();
    missing_slot_meta
        .calibration
        .as_mut()
        .unwrap()
        .per_slot
        .clear();
    let missing_slot_error = guard_with_ledger(
        &mut appender,
        cx(2),
        &missing_slot_meta,
        &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
        &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
        true,
    )
    .expect_err("missing per-slot provenance");
    let store = appender.into_store();
    let rows = store.scan().unwrap();
    let entries = rows
        .iter()
        .map(|row| decode(&row.bytes).unwrap())
        .collect::<Vec<_>>();

    assert!(before_rows.is_empty());
    assert!(verdict.overall_pass);
    assert_eq!(seqs(&entries), vec![0, 1]);
    assert_eq!(calibration_ref.seq, 0);
    assert_eq!(verdict_ref.seq, 1);
    assert!(missing_slot_error.to_string().contains("required slot 1"));
    assert!(
        profile
            .calibration
            .as_ref()
            .unwrap()
            .per_slot
            .contains_key(&slot(1))
    );

    write_json(
        &root,
        "issue649-readback.json",
        &json!({
            "ledger_dir": ledger_dir,
            "before_rows": row_readback(&before_rows),
            "after_rows": row_readback(&rows),
            "entries": entry_readback(&entries),
            "calibration_ref": {"seq": calibration_ref.seq, "hash": hex(&calibration_ref.hash)},
            "verdict_ref": {"seq": verdict_ref.seq, "hash": hex(&verdict_ref.hash)},
            "verdict": verdict,
            "calibration_per_slot_count": profile.calibration.as_ref().unwrap().per_slot.len(),
            "required_slots": profile.required_slots.iter().map(|slot| slot.get()).collect::<Vec<_>>(),
            "missing_slot_error": ward_ledger_error_json(&missing_slot_error),
        }),
    );
    write_sha_manifest(&root);

    println!(
        "ISSUE649_WARD_PROVENANCE_FSV root={} rows={} calibration_seq={} verdict_seq={} missing_code={}",
        root.display(),
        rows.len(),
        calibration_ref.seq,
        verdict_ref.seq,
        ward_ledger_error_code(&missing_slot_error),
    );
}

fn payload_json(entry: &calyx_ledger::LedgerEntry) -> Value {
    serde_json::from_slice(&entry.payload).unwrap()
}

fn row_readback(rows: &[LedgerRow]) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            json!({
                "seq": row.seq,
                "file": format!("{:016x}.ledger", row.seq),
                "bytes_hex": hex(&row.bytes),
            })
        })
        .collect()
}

fn entry_readback(entries: &[LedgerEntry]) -> Vec<Value> {
    entries
        .iter()
        .map(|entry| {
            json!({
                "seq": entry.seq,
                "kind": entry.kind.as_str(),
                "subject": entry.subject,
                "ts": entry.ts,
                "entry_hash": hex(&entry.entry_hash),
                "payload_json": payload_json(entry),
            })
        })
        .collect()
}

fn seqs(entries: &[LedgerEntry]) -> Vec<u64> {
    entries.iter().map(|entry| entry.seq).collect()
}

fn ward_ledger_error_json(error: &WardLedgerError) -> Value {
    match error {
        WardLedgerError::Ward(error) => json!({
            "source": "ward",
            "code": error.code(),
            "message": error.to_string(),
        }),
        WardLedgerError::Ledger(error) => json!({
            "source": "ledger",
            "code": error.code,
            "message": error.message,
        }),
    }
}

fn ward_ledger_error_code(error: &WardLedgerError) -> &'static str {
    match error {
        WardLedgerError::Ward(error) => error.code(),
        WardLedgerError::Ledger(_) => "CALYX_LEDGER_ERROR",
    }
}

fn profile_template() -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic".to_string(),
        tau: BTreeMap::new(),
        required_slots: Vec::new(),
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn calibration_input(slot: SlotId, slot_kind: SlotKind, target_far: f32) -> CalibrationInput {
    CalibrationInput {
        slot,
        good_scores: (0..100).map(|index| 0.80 + index as f32 * 0.001).collect(),
        bad_scores: (0..100).map(|index| 0.30 + index as f32 * 0.003).collect(),
        slot_kind,
        target_far,
    }
}

fn slot_vectors(values: &[(SlotId, Vec<f32>)]) -> ProducedSlots {
    values.iter().cloned().collect()
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
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

fn write_sha_manifest(root: &Path) {
    let mut lines = Vec::new();
    collect_sha_lines(root, root, &mut lines);
    lines.sort();
    fs::write(root.join("sha256-manifest.txt"), lines.concat()).expect("write sha manifest");
}

fn collect_sha_lines(root: &Path, dir: &Path, lines: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read fsv root") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_sha_lines(root, &path, lines);
            continue;
        }
        if !path.is_file() || path.file_name().unwrap() == "sha256-manifest.txt" {
            continue;
        }
        let bytes = fs::read(&path).expect("read fsv file");
        let rel_path = path.strip_prefix(root).expect("relative fsv path");
        lines.push(format!(
            "{:x}  {}\n",
            Sha256::digest(bytes),
            rel_path.display()
        ));
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
