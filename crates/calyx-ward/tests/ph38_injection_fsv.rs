#[path = "ph38_injection_fsv/support.rs"]
mod support;

use std::path::PathBuf;
use std::sync::Arc;

use calyx_core::FixedClock;
use calyx_ward::{
    CalibrationInput, ESTIMATOR, NoveltyHandler, NoveltyStatus, SlotKind, calibrate, guard,
    novel_regions,
};
use serde_json::json;
use support::*;

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
    let centroid = benign_centroid(&corpus.items).expect("benign centroid");
    let profile = calibrate(
        profile_template(),
        vec![CalibrationInput {
            slot: CONTENT_SLOT,
            good_scores: scores_for_label(&corpus.items, &centroid, 0),
            bad_scores: scores_for_label(&corpus.items, &centroid, 1),
            slot_kind: SlotKind::Content,
            target_far: TARGET_FAR,
        }],
        ALPHA,
        &FixedClock::new(CLOCK_TS),
    )
    .expect("calibrate real corpus");
    let block = block_rate(&profile, &corpus, &centroid).expect("block rate");
    assert!(
        block.block_rate >= REQUIRED_BLOCK_RATE,
        "injection block rate {:.4} < {:.2} required",
        block.block_rate,
        REQUIRED_BLOCK_RATE
    );
    let novel = valid_novelty_readback(&root, &profile, &centroid).expect("valid novelty");

    write_json(&root, "corpus-readback.json", &corpus_readback(&corpus));
    write_json(
        &root,
        "calibration-provenance.json",
        &json!({
            "estimator": ESTIMATOR,
            "profile": profile,
            "target_far": TARGET_FAR,
            "corpus_vectors_sha256": corpus.vectors_sha256,
        }),
    );
    write_json(&root, "block-rate.json", &block);
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "dataset": corpus.manifest["dataset"],
            "row_count": corpus.items.len(),
            "injection_total": block.injection_total,
            "blocked": block.blocked,
            "passed": block.passed,
            "block_rate": block.block_rate,
            "required_block_rate": REQUIRED_BLOCK_RATE,
            "estimator": ESTIMATOR,
            "tau": block.tau,
            "novel_status": novel.record.status,
            "novel_regions_count": novel.listed_count,
            "novel_vault_bytes": novel.vault_bytes,
        }),
    );
    write_sha_manifest(&root);

    println!(
        "FSV_PH38_T05 injection_block_rate={:.6} blocked={} total={} tau={:.6} estimator={} novel_status={:?} novel_regions={}",
        block.block_rate,
        block.blocked,
        block.injection_total,
        block.tau,
        ESTIMATOR,
        novel.record.status,
        novel.listed_count,
    );
}
