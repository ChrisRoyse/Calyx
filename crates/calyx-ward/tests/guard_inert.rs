use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_core::{CxId, FixedClock, SlotId};
use calyx_ledger::{
    DirectoryLedgerStore, LedgerAppender, LedgerCfStore, LedgerRow, MemoryLedgerStore,
};
use calyx_ward::{
    CalibrationMeta, GuardId, GuardPolicy, GuardProfile, MatchedSlots, NoveltyAction,
    ProducedSlots, SlotCalibrationMeta, TrustedRegion, WardError, WardLedgerError, guard,
    guard_query, guard_result_with_stakes, guard_with_ledger,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn empty_required_profile_fails_closed_for_high_stakes_and_query() {
    let profile = calibrated_profile(GuardPolicy::AllRequired, &[]);

    let guard_error =
        guard(&profile, &ProducedSlots::new(), &MatchedSlots::new(), true).expect_err("inert");
    let query_error =
        guard_query(&profile, &ProducedSlots::new(), &[region(cx(1))]).expect_err("query inert");
    let no_regions_error =
        guard_query(&profile, &ProducedSlots::new(), &[]).expect_err("no-region query inert");

    assert_eq!(
        guard_error,
        WardError::InertProfile {
            guard_id: guard_id(),
            reason: "empty_required_slots",
        }
    );
    assert_eq!(query_error, guard_error);
    assert_eq!(no_regions_error, guard_error);
    assert_eq!(guard_error.code(), "CALYX_GUARD_INERT_PROFILE");
}

#[test]
fn calibrated_kofn_zero_fails_before_slot_scores_and_ledger_append() {
    let profile = calibrated_profile(GuardPolicy::KofN { k: 0 }, &[slot(1)]);
    let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);
    let matched = slot_vectors(&[(slot(1), cos_vector(0.1))]);
    let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(65_000))
        .expect("appender");

    let direct = guard(&profile, &produced, &matched, true).expect_err("direct inert");
    let result =
        guard_result_with_stakes(&profile, &produced, &matched, true).expect_err("result inert");
    let ledger = guard_with_ledger(&mut appender, cx(2), &profile, &produced, &matched, true)
        .expect_err("ledger inert");
    let rows = appender.into_store().scan().expect("ledger scan");

    assert_eq!(
        direct,
        WardError::InertProfile {
            guard_id: guard_id(),
            reason: "kofn_zero",
        }
    );
    assert_eq!(result, direct);
    assert!(matches!(ledger, WardLedgerError::Ward(error) if error == direct));
    assert!(rows.is_empty());
}

#[test]
#[ignore = "manual FSV for issue #650 inert GuardProfile surfaces"]
fn issue650_inert_guard_fsv_writes_readbacks() {
    let root = std::env::var("CALYX_WARD_ISSUE650_FSV_DIR")
        .map(std::path::PathBuf::from)
        .expect("CALYX_WARD_ISSUE650_FSV_DIR is required");
    reset_dir(&root);
    let ledger_dir = root.join("ledger-cf");
    fs::create_dir_all(&ledger_dir).unwrap();
    let empty_profile = calibrated_profile(GuardPolicy::AllRequired, &[]);
    let k0_profile = calibrated_profile(GuardPolicy::KofN { k: 0 }, &[slot(1)]);
    let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);
    let matched = slot_vectors(&[(slot(1), cos_vector(0.1))]);

    let empty_high_stakes = guard(
        &empty_profile,
        &ProducedSlots::new(),
        &MatchedSlots::new(),
        true,
    )
    .expect_err("empty high-stakes");
    let k0_high_stakes = guard(&k0_profile, &produced, &matched, true).expect_err("k0 high-stakes");
    let query_error =
        guard_query(&empty_profile, &ProducedSlots::new(), &[region(cx(1))]).expect_err("query");
    let no_regions_error =
        guard_query(&empty_profile, &ProducedSlots::new(), &[]).expect_err("no regions query");
    let k0_query_error =
        guard_query(&k0_profile, &produced, &[region_with_slot(cx(3), 0.1)]).expect_err("k0 query");
    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap();
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(65_001),
    )
    .unwrap();
    let ledger_error =
        guard_with_ledger(&mut appender, cx(2), &k0_profile, &produced, &matched, true)
            .expect_err("ledger inert");
    let rows = appender.into_store().scan().unwrap();

    write_json(
        &root,
        "issue650-ward-readback.json",
        &json!({
            "empty_high_stakes": error_json(&empty_high_stakes),
            "kofn_zero_high_stakes": error_json(&k0_high_stakes),
            "query_empty_profile": error_json(&query_error),
            "query_empty_profile_no_regions": error_json(&no_regions_error),
            "query_kofn_zero": error_json(&k0_query_error),
            "ledger_kofn_zero": ward_ledger_error_json(&ledger_error),
            "before_rows": row_readback(&before_rows),
            "after_rows": row_readback(&rows),
            "rows_appended": rows.len() - before_rows.len(),
        }),
    );
    write_sha_manifest(&root);

    println!(
        "ISSUE650_WARD_INERT_FSV empty_code={} k0_code={} query_code={} no_regions_code={} k0_query_code={} ledger_code={} rows_appended={}",
        empty_high_stakes.code(),
        k0_high_stakes.code(),
        query_error.code(),
        no_regions_error.code(),
        k0_query_error.code(),
        ward_ledger_error_code(&ledger_error),
        rows.len() - before_rows.len()
    );
}

fn calibrated_profile(policy: GuardPolicy, required_slots: &[SlotId]) -> GuardProfile {
    let mut tau = BTreeMap::new();
    let mut per_slot = BTreeMap::new();
    for slot in required_slots {
        tau.insert(*slot, 0.7);
        per_slot.insert(*slot, slot_calibration());
    }
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "issue650".to_string(),
        tau,
        required_slots: required_slots.to_vec(),
        policy,
        calibration: Some(CalibrationMeta {
            corpus_hash: [6; 32],
            estimator: "synthetic-conformal".to_string(),
            far: 0.01,
            frr: 0.02,
            confidence: 0.95,
            ts: 65_000,
            per_slot,
        }),
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn slot_calibration() -> SlotCalibrationMeta {
    SlotCalibrationMeta {
        corpus_hash: [7; 32],
        estimator: "synthetic-slot".to_string(),
        far: 0.01,
        frr: 0.02,
        confidence: 0.95,
        ts: 65_000,
    }
}

fn region(cx_id: CxId) -> TrustedRegion {
    TrustedRegion {
        cx_id,
        slots: MatchedSlots::new(),
    }
}

fn region_with_slot(cx_id: CxId, cos: f32) -> TrustedRegion {
    TrustedRegion {
        cx_id,
        slots: slot_vectors(&[(slot(1), cos_vector(cos))]),
    }
}

fn slot_vectors(entries: &[(SlotId, Vec<f32>)]) -> BTreeMap<SlotId, Vec<f32>> {
    entries.iter().cloned().collect()
}

fn cos_vector(cos: f32) -> Vec<f32> {
    vec![cos, (1.0 - cos * cos).sqrt()]
}

fn row_readback(rows: &[LedgerRow]) -> Vec<Value> {
    rows.iter()
        .map(|row| json!({"seq": row.seq, "bytes_hex": hex(&row.bytes)}))
        .collect()
}

fn error_json(error: &WardError) -> Value {
    json!({"code": error.code(), "message": error.to_string()})
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
    for entry in fs::read_dir(root).expect("read fsv root") {
        let path = entry.expect("dir entry").path();
        if path.is_file() && path.file_name().unwrap() != "sha256-manifest.txt" {
            let bytes = fs::read(&path).expect("read fsv file");
            lines.push(format!(
                "{:x}  {}\n",
                Sha256::digest(bytes),
                path.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    lines.sort();
    fs::write(root.join("sha256-manifest.txt"), lines.concat()).expect("write manifest");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
