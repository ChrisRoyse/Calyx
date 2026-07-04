use std::fs;

use calyx_core::CxId;
use calyx_lodestar::{
    ClinicalFrontier, CrossVaultChainParams, CrossVaultMolecularCandidate,
    CrossVaultMolecularGateVerdict, MolecularEndpoint, MolecularKernelState,
    run_cross_vault_grounded_chain,
};
use calyx_paths::AssocGraph;
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue1203-cross-vault-chain")
}

#[test]
fn extends_clinical_frontier_into_grounded_molecular_graph() {
    let report = accepted_report();

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.clinical_seed_count, 1);
    assert_eq!(report.molecular_endpoint_count, 1);
    assert_eq!(report.deficit_count, 0);
    assert_eq!(report.candidate_count, 2);

    let terminal = report
        .candidates
        .iter()
        .find(|candidate| candidate.molecular_hop_count == 1)
        .unwrap();
    assert_eq!(terminal.seed_id, "clinical-il6-chain");
    assert_eq!(terminal.clinical_vault_id, "clinical-vault-01");
    assert_eq!(terminal.molecular_vault_id, "molecular-vault-01");
    assert_eq!(terminal.normalized_entity_id, "HGNC:6018");
    assert_eq!(terminal.molecular_evidence_id, "molecular-evidence-il6");
    assert_eq!(terminal.molecular_entry_cx_id, id("mol-il6-entry"));
    assert_eq!(terminal.terminal_molecular_cx_id, id("mol-metformin"));
    assert!(terminal.rank_score > 0.0);
    assert!(terminal.provenance.iter().any(|row| {
        row == "clinical_vault_id=clinical-vault-01"
            || row == "molecular_vault_id=molecular-vault-01"
    }));
}

#[test]
fn missing_molecular_endpoint_drops_hop_and_logs_deficit() {
    let graph = graph();
    let report = run_cross_vault_grounded_chain(
        &frontiers(),
        &graph,
        MolecularKernelState::Grounded,
        &[],
        &params(),
        pass_gate,
    )
    .unwrap();

    assert_eq!(report.candidate_count, 0);
    assert_eq!(report.deficit_count, 1);
    assert_eq!(
        report.deficits[0].code,
        "CALYX_CROSS_VAULT_MOLECULAR_ENDPOINT_MISSING"
    );
    assert_eq!(report.deficits[0].normalized_entity_id, "HGNC:6018");
}

#[test]
fn ungrounded_molecular_kernel_fails_closed() {
    let graph = graph();
    let err = run_cross_vault_grounded_chain(
        &frontiers(),
        &graph,
        MolecularKernelState::Ungrounded,
        &endpoints(0.20, 0.90),
        &params(),
        pass_gate,
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_MOLECULAR_KERNEL_UNGROUNDED");
}

#[test]
fn missing_molecular_kernel_fails_closed() {
    let graph = graph();
    let err = run_cross_vault_grounded_chain(
        &frontiers(),
        &graph,
        MolecularKernelState::Missing,
        &endpoints(0.20, 0.90),
        &params(),
        pass_gate,
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_MOLECULAR_KERNEL_MISSING");
}

#[test]
fn shared_name_with_low_bits_is_refused_not_bridged() {
    let graph = graph();
    let report = run_cross_vault_grounded_chain(
        &frontiers(),
        &graph,
        MolecularKernelState::Grounded,
        &endpoints(0.01, 0.90),
        &params(),
        pass_gate,
    )
    .unwrap();

    assert_eq!(report.candidate_count, 0);
    assert_eq!(report.deficit_count, 1);
    assert_eq!(
        report.deficits[0].code,
        "CALYX_CROSS_VAULT_MOLECULAR_BITS_GATE_FAILED"
    );
    assert_eq!(
        report.deficits[0].molecular_cx_id,
        Some(id("mol-il6-entry"))
    );
}

#[test]
fn refused_molecular_hop_logs_deficit_after_bridge() {
    let graph = graph();
    let report = run_cross_vault_grounded_chain(
        &frontiers(),
        &graph,
        MolecularKernelState::Grounded,
        &endpoints(0.20, 0.90),
        &params(),
        refuse_gate,
    )
    .unwrap();

    assert_eq!(report.candidate_count, 1);
    assert_eq!(report.candidates[0].molecular_hop_count, 0);
    assert_eq!(report.deficit_count, 1);
    assert_eq!(
        report.deficits[0].code,
        "CALYX_CROSS_VAULT_MOLECULAR_GATE_REFUSED"
    );
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let report = accepted_report();
    let terminal = report
        .candidates
        .iter()
        .find(|candidate| candidate.molecular_hop_count == 1)
        .unwrap();
    let value = json!({
        "issue": 1203,
        "schema_version": report.schema_version,
        "candidate_count": report.candidate_count,
        "deficit_count": report.deficit_count,
        "terminal_seed_id": terminal.seed_id,
        "clinical_vault_id": terminal.clinical_vault_id,
        "molecular_vault_id": terminal.molecular_vault_id,
        "normalized_entity_id": terminal.normalized_entity_id,
        "molecular_evidence_id": terminal.molecular_evidence_id,
        "molecular_entry_cx_id": terminal.molecular_entry_cx_id,
        "terminal_molecular_cx_id": terminal.terminal_molecular_cx_id,
        "molecular_hop_count": terminal.molecular_hop_count,
        "rank_score": terminal.rank_score,
        "full_report": report,
    });
    fs::create_dir_all(&root).unwrap();
    let path = root.join("issue1203_cross_vault_chain_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["candidate_count"], 2);
    assert_eq!(readback["deficit_count"], 0);
    assert_eq!(readback["normalized_entity_id"], "HGNC:6018");
    assert_eq!(readback["molecular_evidence_id"], "molecular-evidence-il6");
    println!(
        "issue1203_fsv_path={} bytes={}",
        path.display(),
        bytes.len()
    );
}

fn accepted_report() -> calyx_lodestar::CrossVaultChainReport {
    let graph = graph();
    run_cross_vault_grounded_chain(
        &frontiers(),
        &graph,
        MolecularKernelState::Grounded,
        &endpoints(0.20, 0.90),
        &params(),
        pass_gate,
    )
    .unwrap()
}

fn graph() -> AssocGraph {
    let mut builder = AssocGraph::builder();
    builder.add_node(id("mol-il6-entry"), 1.0).unwrap();
    builder.add_node(id("mol-metformin"), 1.0).unwrap();
    builder
        .add_edge(id("mol-il6-entry"), id("mol-metformin"), 0.80)
        .unwrap();
    builder.build()
}

fn frontiers() -> Vec<ClinicalFrontier> {
    vec![ClinicalFrontier {
        seed_id: "clinical-il6-chain".to_string(),
        clinical_vault_id: "clinical-vault-01".to_string(),
        clinical_cx_id: id("clinical-il6-chain"),
        normalized_entity_id: "HGNC:6018".to_string(),
        grounded_confidence: 0.88,
        provenance: vec!["clinical_chain=issue1203".to_string()],
    }]
}

fn endpoints(bits: f32, confidence: f32) -> Vec<MolecularEndpoint> {
    vec![MolecularEndpoint {
        molecular_vault_id: "molecular-vault-01".to_string(),
        molecular_cx_id: id("mol-il6-entry"),
        normalized_entity_id: "HGNC:6018".to_string(),
        evidence_id: "molecular-evidence-il6".to_string(),
        grounded_bits: bits,
        grounded_confidence: confidence,
        provenance: vec!["molecular_row=issue1203".to_string()],
    }]
}

fn params() -> CrossVaultChainParams {
    CrossVaultChainParams {
        max_molecular_hops: 1,
        max_candidates: 8,
        min_endpoint_bits: 0.05,
        min_bridge_confidence: 0.25,
        min_molecular_gate_confidence: 0.25,
    }
}

fn pass_gate(candidate: &CrossVaultMolecularCandidate) -> CrossVaultMolecularGateVerdict {
    let passed = candidate.to == id("mol-metformin");
    CrossVaultMolecularGateVerdict {
        passed,
        confidence: if passed { 0.82 } else { 0.10 },
        code: if passed {
            "CALYX_CROSS_VAULT_MOLECULAR_GATE_PASS"
        } else {
            "CALYX_CROSS_VAULT_MOLECULAR_GATE_REFUSED"
        }
        .to_string(),
        reason: "synthetic gate verdict".to_string(),
        evidence: vec![format!("to={}", candidate.to)],
    }
}

fn refuse_gate(candidate: &CrossVaultMolecularCandidate) -> CrossVaultMolecularGateVerdict {
    CrossVaultMolecularGateVerdict {
        passed: false,
        confidence: 0.0,
        code: "CALYX_CROSS_VAULT_MOLECULAR_GATE_REFUSED".to_string(),
        reason: "synthetic molecular hop refused".to_string(),
        evidence: vec![format!("to={}", candidate.to)],
    }
}
