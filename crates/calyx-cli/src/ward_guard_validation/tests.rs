use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use super::data::ScoreCorpus;
use super::engine::evaluate;
use super::metrics::write_metric_outputs;
use super::request::WardGuardRequest;

static CTR: AtomicU32 = AtomicU32::new(0);

fn tmp(name: &str) -> PathBuf {
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "calyx-ward-gv-{}-{}-{}",
        std::process::id(),
        n,
        name
    ))
}

fn write_scores(rows: &[(u8, f32)]) -> PathBuf {
    let path = tmp("scores.jsonl");
    let mut s = String::new();
    for (i, (label, score)) in rows.iter().enumerate() {
        s.push_str(&format!(
            "{{\"split\":\"test\",\"row\":{i},\"label\":{label},\"inj_prob\":{:.6},\"benign_score\":{score:.6}}}\n",
            1.0 - score
        ));
    }
    std::fs::write(&path, s).unwrap();
    path
}

fn request_for(scores: PathBuf) -> WardGuardRequest {
    WardGuardRequest::parse(&[
        "--scores".into(),
        scores.display().to_string(),
        "--metrics-dir".into(),
        tmp("metrics").display().to_string(),
    ])
    .unwrap()
}

/// Clearly separated: benign score high, injection score low.
fn separable_rows() -> Vec<(u8, f32)> {
    let mut v = Vec::new();
    for i in 0..300 {
        v.push((0u8, 0.95 + (i % 40) as f32 / 1000.0)); // 0.950..0.989
    }
    for i in 0..300 {
        v.push((1u8, 0.001 + (i % 40) as f32 / 1000.0)); // 0.001..0.040
    }
    v
}

#[test]
fn separable_scores_pass_both_gates_and_route_novelty() {
    let req = request_for(write_scores(&separable_rows()));
    let corpus = ScoreCorpus::load(&req).expect("load");
    let report = evaluate(&corpus, &req).expect("evaluate");
    assert!(
        report.heldout.block_rate >= 0.99,
        "block_rate {}",
        report.heldout.block_rate
    );
    assert!(
        report.heldout.benign_frr <= 0.01,
        "benign_frr {}",
        report.heldout.benign_frr
    );
    assert!(report.tau.is_finite());
    assert!(report.gates.block_pass && report.gates.frr_pass);
    assert!(
        report.novelty.routed && report.novelty.novel_regions >= 1,
        "OOD must be routed to a new region"
    );
    let evidence = write_metric_outputs(&req, &report).expect("metrics");
    let tau_txt = std::fs::read_to_string(&evidence.tau_path).unwrap();
    assert!(tau_txt.trim().parse::<f32>().is_ok());
    let verdicts = std::fs::read_to_string(&report.verdicts_path).unwrap();
    assert!(verdicts.lines().count() == corpus.heldout.len());
}

#[test]
fn overlapping_injections_fail_closed_not_a_hollow_gate() {
    // Injections score as benign as the benign set -> not separable -> a gate must fail.
    let mut rows = Vec::new();
    for i in 0..300 {
        rows.push((0u8, 0.95 + (i % 40) as f32 / 1000.0));
    }
    for i in 0..300 {
        rows.push((1u8, 0.95 + (i % 40) as f32 / 1000.0));
    }
    let req = request_for(write_scores(&rows));
    let corpus = ScoreCorpus::load(&req).expect("load");
    let err = evaluate(&corpus, &req).expect_err("degenerate must fail closed");
    assert!(
        err.contains("CALYX_FSV_WARD_BLOCK_RATE_BELOW_99PCT")
            || err.contains("CALYX_FSV_WARD_FRR_ABOVE_TARGET"),
        "unexpected error: {err}"
    );
}

#[test]
fn missing_scores_file_reports_not_found() {
    let req = WardGuardRequest::parse(&[
        "--scores".into(),
        tmp("nope.jsonl").display().to_string(),
        "--metrics-dir".into(),
        tmp("m").display().to_string(),
    ])
    .unwrap();
    let err = ScoreCorpus::load(&req).expect_err("missing file");
    assert!(
        err.contains("CALYX_FSV_WARD_SCORES_NOT_FOUND"),
        "unexpected: {err}"
    );
}

#[test]
fn too_few_injections_fails_closed() {
    let mut rows = Vec::new();
    for i in 0..300 {
        rows.push((0u8, 0.97 + (i % 20) as f32 / 1000.0));
    }
    for i in 0..10 {
        rows.push((1u8, 0.01 + (i % 5) as f32 / 1000.0));
    }
    let req = request_for(write_scores(&rows));
    let err = ScoreCorpus::load(&req).expect_err("too few injections");
    assert!(
        err.contains("CALYX_FSV_WARD_INVALID_SCORES"),
        "unexpected: {err}"
    );
}

#[test]
fn all_benign_split_fails_closed() {
    let rows: Vec<(u8, f32)> = (0..200)
        .map(|i| (0u8, 0.97 + (i % 20) as f32 / 1000.0))
        .collect();
    let req = request_for(write_scores(&rows));
    let err = ScoreCorpus::load(&req).expect_err("all benign");
    assert!(
        err.contains("CALYX_FSV_WARD_INVALID_SCORES"),
        "unexpected: {err}"
    );
}
