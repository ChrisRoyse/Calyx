use calyx_paths::AssocGraph;

use super::{ClinicalFrontier, CrossVaultChainParams, MolecularEndpoint, MolecularKernelState};
use crate::{LodestarError, Result};

pub(super) fn validate_kernel_state(state: MolecularKernelState) -> Result<()> {
    match state {
        MolecularKernelState::Grounded => Ok(()),
        MolecularKernelState::Missing => Err(LodestarError::MolecularKernelMissing {
            detail: "molecular kernel state is missing".to_string(),
        }),
        MolecularKernelState::Ungrounded => Err(LodestarError::MolecularKernelUngrounded {
            detail: "molecular kernel is ungrounded".to_string(),
        }),
    }
}

pub(super) fn validate_inputs(
    clinical_frontiers: &[ClinicalFrontier],
    molecular_graph: &AssocGraph,
    endpoints: &[MolecularEndpoint],
    params: &CrossVaultChainParams,
) -> Result<()> {
    if clinical_frontiers.is_empty() {
        return invalid_params("at least one clinical frontier is required");
    }
    if molecular_graph.is_empty() {
        return Err(LodestarError::KernelEmptyGraph);
    }
    if params.max_candidates == 0 {
        return invalid_params("max_candidates must be greater than zero");
    }
    validate_bits("min_endpoint_bits", params.min_endpoint_bits)?;
    validate_score("min_bridge_confidence", params.min_bridge_confidence)?;
    validate_score(
        "min_molecular_gate_confidence",
        params.min_molecular_gate_confidence,
    )?;
    for seed in clinical_frontiers {
        validate_seed(seed)?;
    }
    for endpoint in endpoints {
        validate_endpoint(endpoint, molecular_graph)?;
    }
    Ok(())
}

fn validate_seed(seed: &ClinicalFrontier) -> Result<()> {
    if seed.seed_id.trim().is_empty()
        || seed.clinical_vault_id.trim().is_empty()
        || seed.normalized_entity_id.trim().is_empty()
    {
        return invalid_params("clinical frontier identifiers must not be empty");
    }
    validate_score("clinical grounded_confidence", seed.grounded_confidence)?;
    if seed.provenance.is_empty() {
        return invalid_params("clinical frontier provenance must not be empty");
    }
    Ok(())
}

fn validate_endpoint(endpoint: &MolecularEndpoint, graph: &AssocGraph) -> Result<()> {
    if endpoint.molecular_vault_id.trim().is_empty()
        || endpoint.normalized_entity_id.trim().is_empty()
        || endpoint.evidence_id.trim().is_empty()
    {
        return invalid_params("molecular endpoint identifiers must not be empty");
    }
    validate_bits("molecular grounded_bits", endpoint.grounded_bits)?;
    validate_score(
        "molecular grounded_confidence",
        endpoint.grounded_confidence,
    )?;
    if endpoint.provenance.is_empty() {
        return invalid_params("molecular endpoint provenance must not be empty");
    }
    graph.require_node_index(endpoint.molecular_cx_id)?;
    Ok(())
}

fn validate_score(field: &str, value: f32) -> Result<()> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        invalid_params(format!("{field} must be finite and in [0,1]"))
    }
}

fn validate_bits(field: &str, value: f32) -> Result<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        invalid_params(format!("{field} must be finite and non-negative"))
    }
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}
