use std::fs;

use calyx_core::CxId;
use calyx_lodestar::{
    ClinicalMolecularSeed, MolecularBridgeParams, MolecularEvidenceRow, rank_molecular_bridges,
};
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue884-molecular-bridges")
}

#[test]
fn ranks_clinical_to_molecular_candidates_by_binding_and_grounding() {
    let seeds = seeds();
    let evidence = evidence();

    let report =
        rank_molecular_bridges(&seeds, &evidence, &MolecularBridgeParams::default()).unwrap();

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.seed_count, 2);
    assert_eq!(report.evidence_count, 4);
    assert_eq!(report.candidate_count, 3);
    assert_eq!(report.candidates[0].compound_id, "CHEMBL-TOP");
    assert_eq!(report.candidates[0].target_id, "TARG-IL6");
    assert!(report.candidates[0].binding_score > report.candidates[1].binding_score);
    assert!(
        report.candidates[0]
            .provenance
            .iter()
            .any(|row| row == "chembl:activity:top")
    );
}

#[test]
fn target_hint_filters_candidate_target_space() {
    let mut seeds = seeds();
    seeds[0].target_hint = Some("tnf".to_string());

    let report = rank_molecular_bridges(&[seeds[0].clone()], &evidence(), &params()).unwrap();

    assert_eq!(report.candidate_count, 1);
    assert_eq!(report.candidates[0].target_id, "TARG-TNF");
}

#[test]
fn max_candidates_and_score_floor_apply_after_ranking() {
    let params = MolecularBridgeParams {
        max_candidates: 1,
        min_rank_score: 0.50,
        ..MolecularBridgeParams::default()
    };

    let report = rank_molecular_bridges(&seeds(), &evidence(), &params).unwrap();

    assert_eq!(report.candidate_count, 1);
    assert_eq!(report.candidates[0].compound_id, "CHEMBL-TOP");
}

#[test]
fn invalid_inputs_fail_closed() {
    let err = rank_molecular_bridges(&[], &evidence(), &params()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut bad_evidence = evidence();
    bad_evidence[0].affinity_nm = Some(0.0);
    let err = rank_molecular_bridges(&seeds(), &bad_evidence, &params()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    bad_evidence = evidence();
    bad_evidence[0].protein_sequence = Some("MTEyk".to_string());
    let err = rank_molecular_bridges(&seeds(), &bad_evidence, &params()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut missing_affinity = evidence();
    missing_affinity[0].affinity_nm = None;
    let err = rank_molecular_bridges(&seeds(), &missing_affinity, &params()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
}

#[test]
fn activity_only_mode_can_rank_without_affinity() {
    let params = MolecularBridgeParams {
        require_binding_affinity: false,
        ..MolecularBridgeParams::default()
    };
    let mut evidence = evidence();
    evidence[0].affinity_nm = None;
    evidence[0].activity_score = 0.97;

    let report = rank_molecular_bridges(&seeds(), &evidence, &params).unwrap();

    assert_eq!(report.candidates[0].compound_id, "CHEMBL-TOP");
    assert_eq!(report.candidates[0].binding_score, 0.97);
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let report = rank_molecular_bridges(&seeds(), &evidence(), &params()).unwrap();
    let top = &report.candidates[0];
    let value = json!({
        "issue": 884,
        "schema_version": report.schema_version,
        "seed_count": report.seed_count,
        "evidence_count": report.evidence_count,
        "candidate_count": report.candidate_count,
        "top_compound_id": top.compound_id,
        "top_target_id": top.target_id,
        "top_disease_id": top.disease_id,
        "top_affinity_nm": top.affinity_nm,
        "top_binding_score": top.binding_score,
        "top_rank_score": top.rank_score,
        "full_report": report,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue884_molecular_bridges_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["candidate_count"], 3);
    assert_eq!(readback["top_compound_id"], "CHEMBL-TOP");
    assert_eq!(readback["top_target_id"], "TARG-IL6");
    println!("issue884_fsv_path={} bytes={}", path.display(), bytes.len());
}

fn seeds() -> Vec<ClinicalMolecularSeed> {
    vec![
        ClinicalMolecularSeed {
            seed_id: "clinical-il6".to_string(),
            clinical_cx_id: id("clinical-il6"),
            disease_id: "EFO-DISEASE-1".to_string(),
            disease_name: "synthetic inflammatory disease".to_string(),
            target_hint: None,
            grounded_confidence: 0.92,
            provenance: vec!["clinical_chain=hypothesis-1".to_string()],
        },
        ClinicalMolecularSeed {
            seed_id: "clinical-kras".to_string(),
            clinical_cx_id: id("clinical-kras"),
            disease_id: "EFO-DISEASE-2".to_string(),
            disease_name: "synthetic oncology disease".to_string(),
            target_hint: Some("KRAS".to_string()),
            grounded_confidence: 0.75,
            provenance: vec!["clinical_chain=hypothesis-2".to_string()],
        },
    ]
}

fn evidence() -> Vec<MolecularEvidenceRow> {
    vec![
        row(
            "CHEMBL-TOP",
            "TARG-IL6",
            "IL6",
            "EFO-DISEASE-1",
            Some(8.0),
            0.94,
        ),
        row(
            "CHEMBL-MID",
            "TARG-TNF",
            "TNF alpha",
            "EFO-DISEASE-1",
            Some(120.0),
            0.80,
        ),
        row(
            "CHEMBL-KRAS",
            "TARG-KRAS",
            "KRAS",
            "EFO-DISEASE-2",
            Some(20.0),
            0.86,
        ),
        row(
            "CHEMBL-OFF",
            "TARG-OFF",
            "off disease target",
            "EFO-DISEASE-X",
            Some(5.0),
            0.99,
        ),
    ]
}

fn row(
    compound_id: &str,
    target_id: &str,
    target_name: &str,
    disease_id: &str,
    affinity_nm: Option<f32>,
    target_confidence: f32,
) -> MolecularEvidenceRow {
    MolecularEvidenceRow {
        evidence_id: format!("evidence-{compound_id}-{target_id}"),
        compound_id: compound_id.to_string(),
        compound_name: format!("compound {compound_id}"),
        smiles: "CC(=O)NC1=CC=C(O)C=C1".to_string(),
        target_id: target_id.to_string(),
        target_name: target_name.to_string(),
        protein_sequence: Some("MTEYKLVVVG".to_string()),
        dna_locus_id: Some(format!("locus-{target_id}")),
        disease_id: disease_id.to_string(),
        disease_name: format!("disease {disease_id}"),
        assay_id: format!("assay-{compound_id}-{target_id}"),
        affinity_nm,
        activity_score: 0.60,
        target_confidence,
        disease_confidence: 0.82,
        provenance: vec![format!(
            "{}:{}",
            if compound_id == "CHEMBL-TOP" {
                "chembl"
            } else {
                "bindingdb"
            },
            if compound_id == "CHEMBL-TOP" {
                "activity:top"
            } else {
                "activity"
            }
        )],
    }
}

fn params() -> MolecularBridgeParams {
    MolecularBridgeParams::default()
}
