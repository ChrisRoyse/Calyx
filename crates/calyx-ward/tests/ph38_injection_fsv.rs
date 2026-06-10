#[path = "ph38_injection_fsv/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use calyx_core::FixedClock;
use calyx_ward::{
    CalibrationInput, ESTIMATOR, GuardProfile, MatchedSlots, NoveltyHandler, NoveltyStatus,
    ProducedSlots, SlotKind, WardError, calibrate, guard, novel_regions,
};
use serde_json::json;
use support::*;

const CALIBRATION_SPLIT: &str = "train";
const HELDOUT_SPLIT: &str = "test";

#[test]
fn missing_corpus_reports_path() {
    let path = std::env::temp_dir().join("calyx-ph38-t05-missing-corpus-edge");

    let error = load_corpus(&path).expect_err("missing corpus");

    assert_eq!(error.code(), "CALYX_WARD_MISSING_INJECTION_CORPUS");
    assert!(error.to_string().contains(path.to_string_lossy().as_ref()));
}

#[test]
fn deterministic_vector_at_cosine_hits_target() {
    let anchor = normalize(&[0.2, 0.4, 0.8]).expect("anchor");
    let produced = vector_at_cos(&anchor, NOVELTY_COS).expect("novel vector");

    assert_close(
        cosine(&produced, &anchor).expect("cosine"),
        NOVELTY_COS,
        1.0e-5,
    );
}

#[test]
fn file_vault_sink_roundtrips_novel_records() {
    let path = unique_temp_path("novel-vault-cf.jsonl");
    let vault = FileVault::new(path.clone());
    let (mut profile, produced, matched) = synthetic_novelty_case();
    profile.calibration = Some(calyx_ward::CalibrationMeta::new(
        [9; 32],
        ESTIMATOR,
        0.0,
        0.0,
        0.95,
        &FixedClock::new(CLOCK_TS),
    ));
    let verdict = guard(&profile, &produced, &matched, false).expect("guard verdict");
    let handler = NoveltyHandler::new(Arc::new(vault.clone()), Arc::new(FixedClock::new(CLOCK_TS)));

    let record = handler
        .handle(&profile, &verdict, &produced)
        .expect("novelty record");
    let listed = novel_regions(&vault, Some(0)).expect("novel regions");

    assert_eq!(record.status, NoveltyStatus::AwaitingGrounding);
    assert_eq!(listed, vec![record]);
    std::fs::remove_file(path).ok();
}

#[test]
#[ignore = "manual aiwonder FSV fixture; set CALYX_WARD_PH38_T05_FSV_DIR"]
fn ph38_t05_fsv_fixture_writes_readback_artifacts() {
    let root = PathBuf::from(
        std::env::var("CALYX_WARD_PH38_T05_FSV_DIR")
            .expect("CALYX_WARD_PH38_T05_FSV_DIR is required"),
    );
    std::fs::create_dir_all(&root).expect("create fsv root");
    let corpus_dir = PathBuf::from(
        std::env::var("CALYX_WARD_INJECTION_CORPUS_DIR").unwrap_or_else(|_| CORPUS_DIR.to_string()),
    );
    write_json(
        &root,
        "missing-corpus-error.json",
        &error_json(&load_corpus(&root.join("missing-corpus-edge")).expect_err("missing edge")),
    );

    let corpus = match load_corpus(&corpus_dir) {
        Ok(corpus) => corpus,
        Err(error) => {
            write_json(&root, "real-corpus-error.json", &error_json(&error));
            panic!("real injection corpus unavailable: {error}");
        }
    };
    let calibration_rows = rows_for_split(&corpus.items, CALIBRATION_SPLIT);
    let heldout_rows = rows_for_split(&corpus.items, HELDOUT_SPLIT);
    assert_split_ready(&calibration_rows, &heldout_rows);
    let centroid = benign_centroid_for_rows(&calibration_rows);
    let profile = calibrate(
        profile_template(),
        vec![CalibrationInput {
            slot: CONTENT_SLOT,
            good_scores: scores_for_label_rows(&calibration_rows, &centroid, 0),
            bad_scores: scores_for_label_rows(&calibration_rows, &centroid, 1),
            slot_kind: SlotKind::Content,
            target_far: TARGET_FAR,
        }],
        ALPHA,
        &FixedClock::new(CLOCK_TS),
    )
    .expect("calibrate train split");
    let heldout_block = block_rate_for_rows(
        &profile,
        &corpus,
        &heldout_rows,
        &centroid,
        HELDOUT_SPLIT,
        "heldout_injection_block_rate",
    )
    .expect("heldout block rate");
    let whole_block = block_rate(&profile, &corpus, &centroid).expect("whole corpus block rate");
    assert!(
        heldout_block.block_rate >= REQUIRED_BLOCK_RATE,
        "held-out injection block rate {:.4} < {:.2} required",
        heldout_block.block_rate,
        REQUIRED_BLOCK_RATE
    );
    let novel = valid_novelty_readback(&root, &profile, &centroid).expect("valid novelty");

    write_json(&root, "corpus-readback.json", &corpus_readback(&corpus));
    write_json(
        &root,
        "split-readback.json",
        &split_readback(&corpus, &calibration_rows, &heldout_rows),
    );
    write_json(
        &root,
        "calibration-provenance.json",
        &json!({
            "estimator": ESTIMATOR,
            "alpha": ALPHA,
            "confidence": profile.calibration.as_ref().expect("calibration").confidence,
            "calibration_split": CALIBRATION_SPLIT,
            "calibration_good_count": label_count(&calibration_rows, 0),
            "calibration_bad_count": label_count(&calibration_rows, 1),
            "calibration_far": profile.calibration.as_ref().expect("calibration").far,
            "calibration_frr": profile.calibration.as_ref().expect("calibration").frr,
            "tau": profile.tau_for(&CONTENT_SLOT).expect("content tau"),
            "corpus_hash": hash_hex(
                &profile.calibration.as_ref().expect("calibration").corpus_hash
            ),
            "profile": profile,
            "target_far": TARGET_FAR,
            "corpus_vectors_sha256": corpus.vectors_sha256,
        }),
    );
    write_json(&root, "heldout-block-rate.json", &heldout_block);
    write_json(&root, "whole-corpus-block-rate.json", &whole_block);
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "dataset": corpus.manifest["dataset"],
            "row_count": corpus.items.len(),
            "calibration_split": CALIBRATION_SPLIT,
            "calibration_good_count": label_count(&calibration_rows, 0),
            "calibration_bad_count": label_count(&calibration_rows, 1),
            "calibration_far": profile.calibration.as_ref().expect("calibration").far,
            "heldout_split": HELDOUT_SPLIT,
            "heldout_good_count": label_count(&heldout_rows, 0),
            "heldout_injection_total": heldout_block.injection_total,
            "heldout_blocked": heldout_block.blocked,
            "heldout_passed": heldout_block.passed,
            "heldout_block_rate": heldout_block.block_rate,
            "whole_corpus_block_rate": whole_block.block_rate,
            "required_block_rate": REQUIRED_BLOCK_RATE,
            "estimator": ESTIMATOR,
            "tau": heldout_block.tau,
            "novel_status": novel.record.status,
            "novel_regions_count": novel.listed_count,
            "novel_vault_bytes": novel.vault_bytes,
        }),
    );
    write_sha_manifest(&root);

    println!(
        "FSV_PH38_T05 heldout_split={} heldout_injection_block_rate={:.6} heldout_blocked={} heldout_total={} calibration_split={} calibration_far={:.6} whole_corpus_block_rate={:.6} tau={:.6} estimator={} novel_status={:?} novel_regions={}",
        HELDOUT_SPLIT,
        heldout_block.block_rate,
        heldout_block.blocked,
        heldout_block.injection_total,
        CALIBRATION_SPLIT,
        profile.calibration.as_ref().expect("calibration").far,
        whole_block.block_rate,
        heldout_block.tau,
        ESTIMATOR,
        novel.record.status,
        novel.listed_count,
    );
}

fn rows_for_split<'a>(items: &'a [VectorRow], split: &str) -> Vec<&'a VectorRow> {
    items.iter().filter(|item| item.split == split).collect()
}

fn assert_split_ready(calibration_rows: &[&VectorRow], heldout_rows: &[&VectorRow]) {
    assert!(
        label_count(calibration_rows, 1) >= calyx_ward::MIN_BAD_SCORES,
        "calibration split must contain enough injection rows"
    );
    assert!(
        label_count(calibration_rows, 0) > 0,
        "calibration split must contain benign rows"
    );
    assert!(
        label_count(heldout_rows, 1) > 0,
        "held-out split must contain injection rows"
    );
}

fn label_count(rows: &[&VectorRow], label: u8) -> usize {
    rows.iter().filter(|item| item.label == label).count()
}

fn hash_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn benign_centroid_for_rows(rows: &[&VectorRow]) -> Vec<f32> {
    let first = rows.first().expect("non-empty calibration rows");
    let mut count = 0_usize;
    let mut sum = vec![0.0; first.vec.len()];
    for item in rows.iter().filter(|item| item.label == 0) {
        count += 1;
        for (acc, value) in sum.iter_mut().zip(&item.vec) {
            *acc += *value;
        }
    }
    assert!(count > 0, "benign calibration rows required");
    for value in &mut sum {
        *value /= count as f32;
    }
    normalize(&sum).expect("benign centroid")
}

fn scores_for_label_rows(rows: &[&VectorRow], centroid: &[f32], label: u8) -> Vec<f32> {
    rows.iter()
        .filter(|item| item.label == label)
        .map(|item| cosine(&item.vec, centroid).expect("validated vector"))
        .collect()
}

fn block_rate_for_rows(
    profile: &GuardProfile,
    corpus: &Corpus,
    rows: &[&VectorRow],
    centroid: &[f32],
    evaluation_split: &str,
    metric: &str,
) -> Result<BlockRateReadback, WardError> {
    let mut blocked = 0_usize;
    let mut passed_ids = Vec::new();
    let matched = MatchedSlots::from([(CONTENT_SLOT, centroid.to_vec())]);
    for item in rows.iter().filter(|item| item.label == 1) {
        let produced = ProducedSlots::from([(CONTENT_SLOT, item.vec.clone())]);
        let verdict = guard(profile, &produced, &matched, false)?;
        if verdict.overall_pass {
            passed_ids.push(item.id.clone());
        } else {
            blocked += 1;
        }
    }
    let injection_total = blocked + passed_ids.len();
    assert!(injection_total > 0, "evaluation split has injection rows");
    let meta = profile.calibration.as_ref().expect("calibrated profile");
    Ok(BlockRateReadback {
        metric: metric.to_string(),
        evaluation_split: evaluation_split.to_string(),
        evaluation_row_count: rows.len(),
        dataset: corpus.manifest["dataset"].clone(),
        vectors_sha256: corpus.vectors_sha256.clone(),
        target_far: TARGET_FAR,
        required_block_rate: REQUIRED_BLOCK_RATE,
        achieved_far: meta.far,
        frr: meta.frr,
        tau: profile.tau_for(&CONTENT_SLOT).expect("content tau"),
        injection_total,
        blocked,
        passed: passed_ids.len(),
        block_rate: blocked as f32 / injection_total as f32,
        passed_ids,
    })
}

fn split_readback(
    corpus: &Corpus,
    calibration_rows: &[&VectorRow],
    heldout_rows: &[&VectorRow],
) -> serde_json::Value {
    let mut split_counts = BTreeMap::<String, usize>::new();
    let mut split_label_counts = BTreeMap::<String, BTreeMap<String, usize>>::new();
    for item in &corpus.items {
        *split_counts.entry(item.split.clone()).or_default() += 1;
        *split_label_counts
            .entry(item.split.clone())
            .or_default()
            .entry(item.label.to_string())
            .or_default() += 1;
    }
    json!({
        "chosen_calibration_split": CALIBRATION_SPLIT,
        "chosen_heldout_split": HELDOUT_SPLIT,
        "split_counts": split_counts,
        "split_label_counts": split_label_counts,
        "calibration_row_count": calibration_rows.len(),
        "heldout_row_count": heldout_rows.len(),
        "heldout_injection_count": label_count(heldout_rows, 1),
    })
}
