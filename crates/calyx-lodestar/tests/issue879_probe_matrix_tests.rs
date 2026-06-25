use std::fs;

use calyx_core::{CxId, SlotId};
use calyx_lodestar::{
    ProbeFusionMode, ProbeHit, ProbeLength, ProbeMatrixSpec, ProbePhrasing, ProbeRefusal,
    ProbeResponse, build_probe_matrix, run_probe_matrix,
};
use calyx_sextant::RrfProfile;
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue879-probe-matrix")
}

fn small_spec() -> ProbeMatrixSpec {
    ProbeMatrixSpec {
        frontier: "type 2 diabetes".to_string(),
        active_slots: vec![SlotId::new(1), SlotId::new(8)],
        weighted_profiles: vec![RrfProfile::General, RrfProfile::Kernel],
        phrasings: vec![ProbePhrasing::Terse, ProbePhrasing::Mechanistic],
        lengths: vec![ProbeLength::Entity, ProbeLength::Paragraph],
        top_k: 5,
    }
}

#[test]
fn build_matrix_cross_product_uses_existing_probe_axes() {
    let variants = build_probe_matrix(&small_spec()).unwrap();

    assert_eq!(variants.len(), 28);
    assert!(
        variants
            .iter()
            .any(|variant| variant.fusion == ProbeFusionMode::KernelFirst)
    );
    assert!(
        variants
            .iter()
            .any(|variant| variant.fusion == ProbeFusionMode::WeightedRrf)
    );
    assert!(
        variants
            .iter()
            .any(|variant| variant.fusion == ProbeFusionMode::SingleLens)
    );
    assert!(
        variants
            .iter()
            .any(|variant| variant.query_text.contains("provenance"))
    );
}

#[test]
fn run_matrix_logs_productive_unique_grounded_hits() {
    let spec = ProbeMatrixSpec {
        frontier: "migraine magnesium".to_string(),
        active_slots: vec![SlotId::new(8)],
        weighted_profiles: vec![RrfProfile::Kernel],
        phrasings: vec![ProbePhrasing::Clinical],
        lengths: vec![ProbeLength::Phrase],
        top_k: 3,
    };
    let shared = id("shared-grounded");
    let unique = id("unique-grounded");
    let ungrounded = id("ungrounded");
    let log = run_probe_matrix(&spec, |variant| {
        let mut response = ProbeResponse::default();
        response.hits.push(hit(shared, 0.8, true));
        if variant.fusion == ProbeFusionMode::WeightedRrf {
            response.hits.push(hit(unique, 0.9, true));
        }
        if variant.fusion == ProbeFusionMode::Pipeline {
            response.hits.push(hit(ungrounded, 0.95, false));
            response.refusals.push(ProbeRefusal {
                code: "CALYX_PROBE_INSUFFICIENT_BITS".to_string(),
                reason: "synthetic deficit".to_string(),
                deficit_bits: Some(0.25),
            });
        }
        Ok(response)
    })
    .unwrap();

    assert_eq!(log.records.len(), 5);
    assert_eq!(log.productive.len(), 1);
    assert_eq!(log.productive[0].fusion, ProbeFusionMode::WeightedRrf);
    assert_eq!(log.productive[0].unique_hit_count, 1);
    assert!(log.records.iter().any(|record| record.refusals.len() == 1));
    assert!(
        log.records
            .iter()
            .all(|record| !record.unique_grounded_hits.contains(&ungrounded))
    );
}

#[test]
fn invalid_spec_fails_closed() {
    let spec = ProbeMatrixSpec {
        frontier: " ".to_string(),
        active_slots: vec![SlotId::new(1)],
        weighted_profiles: vec![RrfProfile::General],
        phrasings: vec![ProbePhrasing::Terse],
        lengths: vec![ProbeLength::Entity],
        top_k: 5,
    };
    let err = build_probe_matrix(&spec).unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
}

#[test]
fn invalid_probe_response_fails_closed() {
    let spec = ProbeMatrixSpec {
        frontier: "frontier".to_string(),
        active_slots: vec![SlotId::new(1)],
        weighted_profiles: vec![RrfProfile::General],
        phrasings: vec![ProbePhrasing::Terse],
        lengths: vec![ProbeLength::Entity],
        top_k: 5,
    };
    let err = run_probe_matrix(&spec, |_| {
        Ok(ProbeResponse {
            hits: vec![hit(id("bad"), f32::NAN, true)],
            refusals: Vec::new(),
        })
    })
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let spec = ProbeMatrixSpec {
        frontier: "asthma beta blocker".to_string(),
        active_slots: vec![SlotId::new(8)],
        weighted_profiles: vec![RrfProfile::Bridge],
        phrasings: vec![ProbePhrasing::Clinical],
        lengths: vec![ProbeLength::Phrase],
        top_k: 3,
    };
    let log = run_probe_matrix(&spec, |variant| {
        let unique = variant.fusion == ProbeFusionMode::WeightedRrf;
        Ok(ProbeResponse {
            hits: vec![
                hit(id("fsv-shared"), 0.7, true),
                hit(id("fsv-unique"), 0.9, unique),
            ],
            refusals: if variant.fusion == ProbeFusionMode::Pipeline {
                vec![ProbeRefusal {
                    code: "CALYX_PROBE_SYNTHETIC_REFUSAL".to_string(),
                    reason: "pipeline intentionally refused in synthetic FSV".to_string(),
                    deficit_bits: Some(0.1),
                }]
            } else {
                Vec::new()
            },
        })
    })
    .unwrap();
    let value = json!({
        "issue": 879,
        "schema_version": log.schema_version,
        "variant_count": log.records.len(),
        "productive_count": log.productive.len(),
        "top_productive_fusion": log.productive.first().map(|row| format!("{:?}", row.fusion)),
        "refusal_count": log.records.iter().map(|record| record.refusals.len()).sum::<usize>(),
        "full_log": log,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue879_probe_matrix_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["variant_count"], 5);
    assert_eq!(readback["productive_count"], 1);
    assert_eq!(readback["refusal_count"], 1);
    println!("issue879_fsv_path={} bytes={}", path.display(), bytes.len());
}

fn hit(cx_id: CxId, score: f32, grounded: bool) -> ProbeHit {
    ProbeHit {
        cx_id,
        score,
        grounded,
        provenance: vec!["synthetic-provenance".to_string()],
    }
}
