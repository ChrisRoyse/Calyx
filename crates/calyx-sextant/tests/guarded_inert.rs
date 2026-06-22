use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId,
    SlotVector,
};
use calyx_sextant::{HitGuardMode, HnswIndex, Query, QueryGuard, SearchEngine, SlotIndexMap};
use calyx_ward::{GuardPolicy, GuardProfile, NoveltyAction};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::{
    cx_u8_fill as cx, default_vault_id as vault, dense, guarded_test_guard_id as guard_id,
};

#[test]
fn in_region_only_rejects_empty_required_profile_before_hits() {
    let engine = engine();
    let error = engine
        .search_with_guard_report(&guarded_query(empty_profile()))
        .expect_err("empty profile");

    assert_eq!(error.code, "CALYX_GUARD_INERT_PROFILE");
    assert!(error.message.contains("empty_required_slots"));
}

#[test]
fn in_region_only_rejects_kofn_zero_before_hits() {
    let engine = engine();
    let error = engine
        .search_with_guard_report(&guarded_query(kofn_zero_profile()))
        .expect_err("k0 profile");

    assert_eq!(error.code, "CALYX_GUARD_INERT_PROFILE");
    assert!(error.message.contains("kofn_zero"));
}

#[test]
fn non_inert_uncalibrated_profile_keeps_provisional_guard_evidence() {
    let engine = engine();
    let report = engine
        .search_with_guard_report(&guarded_query(provisional_profile()))
        .expect("provisional guarded search");

    assert_eq!(report.hits.len(), 1);
    let guard = report.hits[0].guard.as_ref().expect("guard evidence");
    assert_eq!(guard.mode, HitGuardMode::InRegionOnly);
    assert!(guard.verdict.overall_pass);
    assert!(guard.verdict.provisional);
    assert_eq!(guard.verdict.per_slot.len(), 1);
}

#[test]
#[ignore = "manual FSV for issue #650 Sextant inert GuardProfile surfaces"]
fn issue650_sextant_inert_guard_fsv_writes_readbacks() {
    let root = std::env::var("CALYX_SEXTANT_ISSUE650_FSV_DIR")
        .map(std::path::PathBuf::from)
        .expect("CALYX_SEXTANT_ISSUE650_FSV_DIR is required");
    reset_dir(&root);
    let engine = engine();
    let before = engine.search(&base_query()).expect("unguarded before");
    let empty_error = engine
        .search_with_guard_report(&guarded_query(empty_profile()))
        .expect_err("empty required");
    let k0_error = engine
        .search_with_guard_report(&guarded_query(kofn_zero_profile()))
        .expect_err("kofn zero");
    let provisional = engine
        .search_with_guard_report(&guarded_query(provisional_profile()))
        .expect("provisional guarded");

    write_json(
        &root,
        "before-unguarded-hits.json",
        &json!(hit_ids(&before)),
    );
    write_json(
        &root,
        "inregion-empty-required-error.json",
        &json!({"code": empty_error.code, "message": empty_error.message}),
    );
    write_json(
        &root,
        "inregion-kofn0-error.json",
        &json!({"code": k0_error.code, "message": k0_error.message}),
    );
    write_json(
        &root,
        "provisional-guarded-report.json",
        &json!(provisional),
    );
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "before_ids": hit_ids(&before),
            "empty_required_code": empty_error.code,
            "kofn_zero_code": k0_error.code,
            "provisional_hit_count": provisional.hits.len(),
            "provisional_flag": provisional.hits[0].guard.as_ref().map(|guard| guard.verdict.provisional),
            "provisional_per_slot_count": provisional.hits[0].guard.as_ref().map(|guard| guard.verdict.per_slot.len()),
        }),
    );
    write_sha_manifest(&root);

    println!(
        "ISSUE650_SEXTANT_INERT_FSV empty_code={} k0_code={} provisional_hits={} provisional_flag={}",
        empty_error.code,
        k0_error.code,
        provisional.hits.len(),
        provisional.hits[0]
            .guard
            .as_ref()
            .unwrap()
            .verdict
            .provisional
    );
}

fn engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(slot(), 2, 42)).unwrap();
    let mut engine = SearchEngine::new(map);
    engine
        .indexes
        .insert(slot(), cx(1), dense(vec![1.0, 0.0]), 1)
        .unwrap();
    engine.put_constellation(row(cx(1), dense(vec![1.0, 0.0]), 1));
    engine
}

fn base_query() -> Query {
    let mut query = Query::new("issue650")
        .with_slots(vec![slot()])
        .with_vector(dense(vec![1.0, 0.0]));
    query.k = 1;
    query
}

fn guarded_query(profile: GuardProfile) -> Query {
    base_query().with_guard(QueryGuard::InRegionOnly(profile))
}

fn empty_profile() -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "issue650-empty".to_string(),
        tau: BTreeMap::new(),
        required_slots: Vec::new(),
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn kofn_zero_profile() -> GuardProfile {
    let mut profile = provisional_profile();
    profile.policy = GuardPolicy::KofN { k: 0 };
    profile
}

fn provisional_profile() -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "issue650-provisional".to_string(),
        tau: BTreeMap::from([(slot(), 0.7)]),
        required_slots: vec![slot()],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn row(cx_id: CxId, vector: SlotVector, seq: u64) -> calyx_core::Constellation {
    calyx_core::Constellation {
        cx_id,
        vault_id: vault(),
        panel_version: 42,
        created_at: seq,
        input_ref: InputRef {
            hash: [seq as u8; 32],
            pointer: Some(format!("zfs://calyx/issue650/{seq}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::from([(slot(), vector)]),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("guard-region".to_string()),
            value: AnchorValue::Enum("trusted".to_string()),
            source: "issue650-fsv".to_string(),
            observed_at: seq,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq,
            hash: [seq as u8; 32],
        },
        flags: CxFlags::default(),
    }
}

fn hit_ids(hits: &[calyx_sextant::Hit]) -> Vec<CxId> {
    hits.iter().map(|hit| hit.cx_id).collect()
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

fn reset_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
}

const fn slot() -> SlotId {
    SlotId::new(8)
}
