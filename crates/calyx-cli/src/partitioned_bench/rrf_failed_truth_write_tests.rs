use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[test]
fn rrf_recall_floor_failure_does_not_write_generated_db_truth() {
    let root = temp_root("rrf-floor-no-truth-write");
    let corpus = root.join("corpus.i8bin");
    let queries = root.join("queries.i8bin");
    let vault = root.join("vault");
    let plan_path = root.join("plan.json");
    let plan_cf = root.join("plan-cf");
    let timeline_cf = root.join("timeline-cf");
    let a37_cf = root.join("a37-cf");
    let slot_truth_cf = root.join("slot-truth-cf");
    let fused_truth_cf = root.join("fused-truth-cf");

    write_i8bin(&corpus, 2, &[&[10, 0], &[0, 10], &[0, 0]]);
    write_i8bin(&queries, 2, &[&[10, 0]]);
    run_build(&[
        "--vault".into(),
        path_arg(&vault),
        "--vectors".into(),
        path_arg(&corpus),
        "--regions".into(),
        "1".into(),
        "--distance-metric".into(),
        "raw-l2".into(),
        "--sample".into(),
        "3".into(),
        "--chunk".into(),
        "3".into(),
        "--m-max".into(),
        "2".into(),
        "--ef".into(),
        "4".into(),
        "--region-build-parallelism".into(),
        "1".into(),
    ])
    .expect("build tiny real partitioned vault");
    write_rrf_plan(&plan_path, 10);
    run_rrf_plan(&[
        "--plan".into(),
        path_arg(&plan_path),
        "--cf-root".into(),
        path_arg(&plan_cf),
        "--plan-key".into(),
        "unit_plan".into(),
    ])
    .expect("import RRF plan through DB");
    let loaded_plan = rrf_plan::load_from_file(&plan_path).unwrap();
    write_active_timeline_cf(&timeline_cf, "unit_timeline", 3);
    write_a37_admission_cf(&a37_cf, "unit_a37", &loaded_plan.plan);
    write_slot_truth_cf(&slot_truth_cf, "unit_slot_truth", &loaded_plan, 3);

    let err = run_rrf(&[
        "--plan-cf-root".into(),
        path_arg(&plan_cf),
        "--plan-key".into(),
        "unit_plan".into(),
        "--timeline-cf-root".into(),
        path_arg(&timeline_cf),
        "--timeline-key".into(),
        "unit_timeline".into(),
        "--a37-admission-cf-root".into(),
        path_arg(&a37_cf),
        "--a37-admission-key".into(),
        "unit_a37".into(),
        "--slot-ground-truth-cf-root".into(),
        path_arg(&slot_truth_cf),
        "--slot-ground-truth-key".into(),
        "unit_slot_truth".into(),
        "--write-fused-ground-truth-cf-root".into(),
        path_arg(&fused_truth_cf),
        "--write-fused-ground-truth-key".into(),
        "should_not_exist".into(),
        "--n".into(),
        "1".into(),
        "--k".into(),
        "1".into(),
        "--n-probe".into(),
        "1".into(),
        "--region-beam".into(),
        "8".into(),
        "--ground-truth".into(),
        "1".into(),
        "--truth-depth".into(),
        "1".into(),
        "--recall-floor".into(),
        "1.0".into(),
    ])
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RECALL_BELOW_FLOOR");
    assert_eq!(file_count(&fused_truth_cf), 0);
    let _ = std::fs::remove_dir_all(root);
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "calyx-partitioned-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn write_i8bin(path: &Path, dim: u32, rows: &[&[i8]]) {
    let mut bytes = Vec::with_capacity(8 + rows.len() * dim as usize);
    bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&dim.to_le_bytes());
    for row in rows {
        assert_eq!(row.len(), dim as usize);
        bytes.extend(row.iter().map(|value| *value as u8));
    }
    std::fs::write(path, bytes).unwrap();
}

fn write_rrf_plan(path: &Path, lens_count: u16) {
    let slots = (0..lens_count)
        .map(|idx| {
            serde_json::json!({
                "slot": idx,
                "name": format!("unit-lens-{idx}"),
                "lens_id": format!("{:032x}", idx + 1),
                "weights_sha256": format!("{:064x}", idx + 1),
                "signal_kind": "learned_encoder",
                "bits_about": 0.1,
                "vault": "vault",
                "queries": "queries.i8bin",
                "corpus": "corpus.i8bin",
            })
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({ "slots": slots })).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn write_active_timeline_cf(cf_root: &Path, key: &str, rows: usize) {
    let records = (0..rows)
        .map(|idx| timeline_store::TimelineRowRecord {
            row_idx: idx,
            id: format!("row-{idx}"),
            source_event_time_secs: Some(1_704_153_600 + idx as i64),
            source_event_time_raw: Some((1_704_153_600 + idx as i64).to_string()),
            temporal_lane_state: calyx_core::TEMPORAL_LANE_ACTIVE.to_string(),
            temporal_inactive_reason: None,
            source_sequence: "unit".to_string(),
            source_sequence_index: Some(idx),
            query_row: idx == 0,
        })
        .collect::<Vec<_>>();
    timeline_store::write(cf_root, key, "unit-timeline-source", &records, 2).unwrap();
}

fn write_a37_admission_cf(cf_root: &Path, key: &str, plan: &rrf_plan::Plan) {
    use crate::assay_multi_anchor_card::model::{
        LensEvidence, MultiAnchorReport, TargetLensValue, TargetSummary,
    };

    let lenses = plan
        .slots
        .iter()
        .map(|slot| LensEvidence {
            slot: slot.slot,
            name: slot.name.clone().unwrap(),
            association_family: "unit_family".to_string(),
            passed: true,
            best_target_class: 0,
            best_domain: "unit".to_string(),
            best_marginal_bits: 0.1,
            best_solo_bits: 0.2,
            target_values: vec![TargetLensValue {
                target_class: 0,
                domain: "unit".to_string(),
                marginal_bits: 0.1,
                solo_bits: 0.2,
                decision: "keep".to_string(),
            }],
        })
        .collect::<Vec<_>>();
    let mut association_families = BTreeMap::new();
    association_families.insert(
        "unit_family".to_string(),
        plan.slots.iter().map(|slot| slot.slot).collect::<Vec<_>>(),
    );
    let report = MultiAnchorReport {
        schema_version: 1,
        role: "a37_multi_anchor_admission_card".to_string(),
        status: calyx_assay::A37_DIVERSITY_GATE_PASSED.to_string(),
        mode: "gate".to_string(),
        gate_passed: true,
        report_count: 1,
        lens_count: plan.slots.len(),
        passing_lens_count: plan.slots.len(),
        min_lenses: plan.slots.len(),
        min_marginal_bits: calyx_assay::DEFAULT_MIN_MARGINAL_BITS,
        max_redundancy: 0.6,
        family_span_pass: true,
        redundancy_bound_pass: true,
        no_collapse_pass: true,
        association_family_count: association_families.len(),
        association_families,
        min_best_marginal_bits: 0.1,
        max_best_marginal_bits: 0.1,
        weakest_lens: "unit-lens-0".to_string(),
        target_summaries: vec![TargetSummary {
            target_class: 0,
            domain: "unit".to_string(),
            report_path: "db://unit".to_string(),
            status: calyx_assay::A37_DIVERSITY_GATE_PASSED.to_string(),
            no_collapse_pass: true,
            family_span_pass: true,
            redundancy_bound_pass: true,
            n_eff: plan.slots.len() as f32,
            panel_bits: 1.0,
            max_marginal_bits: 0.1,
            keep_count: plan.slots.len(),
            park_count: 0,
        }],
        lenses,
        source_reports: vec!["db://unit-a37-source".to_string()],
    };
    crate::a37_admission_store::write(cf_root, key, &report).unwrap();
}

fn write_slot_truth_cf(
    cf_root: &Path,
    key: &str,
    loaded: &rrf_plan::LoadedPlan,
    corpus_rows: usize,
) {
    let slots = loaded
        .plan
        .slots
        .iter()
        .map(|slot| slot_truth_store::SlotTruthRecordSlot {
            slot: slot.slot,
            lens_id: slot.lens_id.clone().unwrap(),
            weights_sha256: slot.weights_sha256.clone().unwrap(),
            signal_kind: slot.signal_kind.clone().unwrap(),
            rows: vec![vec![1]],
        })
        .collect::<Vec<_>>();
    let record = slot_truth_store::SlotTruthRecord {
        format: slot_truth_store::FORMAT.to_string(),
        mode: slot_truth_store::MODE.to_string(),
        row_id_space: slot_truth_store::ROW_ID_SPACE.to_string(),
        plan_sha256: loaded.plan_sha256.clone(),
        query_count: 1,
        truth_depth: 1,
        corpus_rows,
        reference_backend: "unit-test-slot-truth".to_string(),
        scale_suitable: true,
        slots,
    };
    slot_truth_store::write(cf_root, key, &record).unwrap();
}

fn file_count(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() { file_count(&path) } else { 1 }
        })
        .sum()
}
