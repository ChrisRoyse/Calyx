use std::fs;

use calyx_core::{CxId, SlotId};
use calyx_lodestar::{
    BlindSpotGateVerdict, BlindSpotNeighbor, BlindSpotObservation, BlindSpotSweepParams,
    sweep_blind_spots,
};
use calyx_loom::Severity;
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue875-blind-spot-sweep")
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

#[test]
fn ranks_gate_passing_high_severity_disagreements() {
    let observations = vec![
        observation("candidate-low", 0.91, 0.08, true, 0.60),
        observation("candidate-top", 0.98, 0.02, true, 0.90),
        observation("gate-refused", 0.99, 0.01, false, 0.95),
        observation("not-a-blind-spot", 0.55, 0.20, true, 0.90),
    ];

    let log = sweep_blind_spots(&observations, &BlindSpotSweepParams::default()).unwrap();

    assert_eq!(log.observation_count, 4);
    assert_eq!(log.detected_alert_count, 3);
    assert_eq!(log.gate_refused_count, 1);
    assert_eq!(log.candidates.len(), 2);
    assert!(log.candidates[0].rank_score > log.candidates[1].rank_score);
    assert!(log.candidates[0].text.contains("candidate-top"));
    assert_eq!(log.candidates[0].alert.severity, Severity::High);
    assert_eq!(log.candidates[0].lens_a_neighbors.len(), 2);
    assert_eq!(log.candidates[0].lens_b_neighbors.len(), 2);
}

#[test]
fn severity_filter_drops_medium_alerts() {
    let observations = vec![observation("medium", 0.75, 0.08, true, 0.90)];

    let log = sweep_blind_spots(&observations, &BlindSpotSweepParams::default()).unwrap();

    assert_eq!(log.detected_alert_count, 1);
    assert_eq!(log.severity_filtered_count, 1);
    assert!(log.candidates.is_empty());
}

#[test]
fn max_candidates_truncates_after_ranking() {
    let observations = vec![
        observation("second", 0.92, 0.02, true, 0.60),
        observation("first", 0.99, 0.01, true, 0.95),
    ];
    let params = BlindSpotSweepParams {
        max_candidates: 1,
        ..BlindSpotSweepParams::default()
    };

    let log = sweep_blind_spots(&observations, &params).unwrap();

    assert_eq!(log.candidates.len(), 1);
    assert!(log.candidates[0].text.contains("first"));
}

#[test]
fn invalid_input_fails_closed() {
    let mut observations = vec![observation("bad", f32::NAN, 0.01, true, 0.95)];
    let err = sweep_blind_spots(&observations, &BlindSpotSweepParams::default()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    observations[0].lens_a_similarity = 0.90;
    observations[0].gate.confidence = 1.5;
    let err = sweep_blind_spots(&observations, &BlindSpotSweepParams::default()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let observations = vec![
        observation("fsv-top", 0.99, 0.01, true, 0.90),
        observation("fsv-refused", 0.98, 0.02, false, 0.90),
        observation("fsv-medium", 0.74, 0.08, true, 0.90),
    ];
    let log = sweep_blind_spots(&observations, &BlindSpotSweepParams::default()).unwrap();
    let value = json!({
        "issue": 875,
        "schema_version": log.schema_version,
        "observation_count": log.observation_count,
        "detected_alert_count": log.detected_alert_count,
        "gate_refused_count": log.gate_refused_count,
        "severity_filtered_count": log.severity_filtered_count,
        "candidate_count": log.candidates.len(),
        "top_severity": log.candidates.first().map(|candidate| format!("{:?}", candidate.alert.severity)),
        "top_delta": log.candidates.first().map(|candidate| candidate.alert.delta),
        "neighbor_evidence_count": log.candidates.first()
            .map(|candidate| candidate.lens_a_neighbors.len() + candidate.lens_b_neighbors.len())
            .unwrap_or_default(),
        "full_log": log,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue875_blind_spot_sweep_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["candidate_count"], 1);
    assert_eq!(readback["gate_refused_count"], 1);
    assert_eq!(readback["severity_filtered_count"], 1);
    println!("issue875_fsv_path={} bytes={}", path.display(), bytes.len());
}

fn observation(
    label: &str,
    lens_a_similarity: f32,
    lens_b_neighbor_mean: f32,
    gate_passed: bool,
    gate_confidence: f32,
) -> BlindSpotObservation {
    BlindSpotObservation {
        cx_id: id(label),
        text: format!("source text for {label}"),
        lens_a: slot(8),
        lens_b: slot(14),
        lens_a_similarity,
        lens_b_neighbor_mean,
        lens_a_neighbors: vec![
            neighbor(&format!("{label}-a1"), 0.92),
            neighbor(&format!("{label}-a2"), 0.88),
        ],
        lens_b_neighbors: vec![
            neighbor(&format!("{label}-b1"), lens_b_neighbor_mean),
            neighbor(&format!("{label}-b2"), lens_b_neighbor_mean + 0.02),
        ],
        gate: BlindSpotGateVerdict {
            passed: gate_passed,
            confidence: gate_confidence,
            code: if gate_passed {
                "CALYX_BLIND_SPOT_GATE_PASS".to_string()
            } else {
                "CALYX_BLIND_SPOT_GATE_REFUSED".to_string()
            },
            reason: "synthetic gate verdict".to_string(),
            evidence: vec!["synthetic sufficiency evidence".to_string()],
        },
    }
}

fn neighbor(label: &str, similarity: f32) -> BlindSpotNeighbor {
    BlindSpotNeighbor {
        cx_id: id(label),
        text: format!("neighbor text for {label}"),
        similarity,
    }
}
