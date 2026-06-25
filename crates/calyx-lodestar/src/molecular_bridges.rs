use calyx_core::CxId;
use serde::{Deserialize, Serialize};

use crate::{LodestarError, Result};

pub const MOLECULAR_BRIDGE_SCHEMA_VERSION: u32 = 1;
const BINDING_WEIGHT: f32 = 0.35;
const TARGET_WEIGHT: f32 = 0.25;
const DISEASE_WEIGHT: f32 = 0.20;
const GROUNDING_WEIGHT: f32 = 0.20;
const MAX_P_AFFINITY: f32 = 10.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MolecularBridgeParams {
    pub max_candidates: usize,
    pub min_rank_score: f32,
    pub require_binding_affinity: bool,
}

impl Default for MolecularBridgeParams {
    fn default() -> Self {
        Self {
            max_candidates: 32,
            min_rank_score: 0.0,
            require_binding_affinity: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClinicalMolecularSeed {
    pub seed_id: String,
    pub clinical_cx_id: CxId,
    pub disease_id: String,
    pub disease_name: String,
    pub target_hint: Option<String>,
    pub grounded_confidence: f32,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MolecularEvidenceRow {
    pub evidence_id: String,
    pub compound_id: String,
    pub compound_name: String,
    pub smiles: String,
    pub target_id: String,
    pub target_name: String,
    pub protein_sequence: Option<String>,
    pub dna_locus_id: Option<String>,
    pub disease_id: String,
    pub disease_name: String,
    pub assay_id: String,
    pub affinity_nm: Option<f32>,
    pub activity_score: f32,
    pub target_confidence: f32,
    pub disease_confidence: f32,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MolecularBridgeCandidate {
    pub seed_id: String,
    pub clinical_cx_id: CxId,
    pub compound_id: String,
    pub compound_name: String,
    pub target_id: String,
    pub target_name: String,
    pub disease_id: String,
    pub disease_name: String,
    pub assay_id: String,
    pub affinity_nm: Option<f32>,
    pub binding_score: f32,
    pub target_confidence: f32,
    pub disease_confidence: f32,
    pub grounded_confidence: f32,
    pub rank_score: f32,
    pub testable_claim: String,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MolecularBridgeReport {
    pub schema_version: u32,
    pub seed_count: usize,
    pub evidence_count: usize,
    pub candidate_count: usize,
    pub candidates: Vec<MolecularBridgeCandidate>,
}

pub fn rank_molecular_bridges(
    seeds: &[ClinicalMolecularSeed],
    evidence: &[MolecularEvidenceRow],
    params: &MolecularBridgeParams,
) -> Result<MolecularBridgeReport> {
    validate_params(params)?;
    if seeds.is_empty() {
        return invalid_params("at least one clinical seed is required");
    }
    for seed in seeds {
        validate_seed(seed)?;
    }
    for row in evidence {
        validate_evidence(row, params)?;
    }

    let mut candidates = Vec::new();
    for seed in seeds {
        for row in evidence
            .iter()
            .filter(|row| row.disease_id == seed.disease_id)
        {
            if !target_matches(seed, row) {
                continue;
            }
            let candidate = candidate_from(seed, row);
            if candidate.rank_score >= params.min_rank_score {
                candidates.push(candidate);
            }
        }
    }
    sort_candidates(&mut candidates);
    candidates.truncate(params.max_candidates);
    Ok(MolecularBridgeReport {
        schema_version: MOLECULAR_BRIDGE_SCHEMA_VERSION,
        seed_count: seeds.len(),
        evidence_count: evidence.len(),
        candidate_count: candidates.len(),
        candidates,
    })
}

fn candidate_from(
    seed: &ClinicalMolecularSeed,
    row: &MolecularEvidenceRow,
) -> MolecularBridgeCandidate {
    let binding_score = row.affinity_nm.map_or(row.activity_score, binding_score);
    let rank_score = binding_score * BINDING_WEIGHT
        + row.target_confidence * TARGET_WEIGHT
        + row.disease_confidence * DISEASE_WEIGHT
        + seed.grounded_confidence * GROUNDING_WEIGHT;
    let mut provenance = seed.provenance.clone();
    provenance.extend(row.provenance.iter().cloned());
    provenance.push(format!("molecular_evidence_id={}", row.evidence_id));
    MolecularBridgeCandidate {
        seed_id: seed.seed_id.clone(),
        clinical_cx_id: seed.clinical_cx_id,
        compound_id: row.compound_id.clone(),
        compound_name: row.compound_name.clone(),
        target_id: row.target_id.clone(),
        target_name: row.target_name.clone(),
        disease_id: row.disease_id.clone(),
        disease_name: row.disease_name.clone(),
        assay_id: row.assay_id.clone(),
        affinity_nm: row.affinity_nm,
        binding_score,
        target_confidence: row.target_confidence,
        disease_confidence: row.disease_confidence,
        grounded_confidence: seed.grounded_confidence,
        rank_score,
        testable_claim: format!(
            "{} may modulate {} for {}",
            row.compound_name, row.target_name, row.disease_name
        ),
        provenance,
    }
}

fn binding_score(affinity_nm: f32) -> f32 {
    let molar = affinity_nm * 1.0e-9;
    (-molar.log10() / MAX_P_AFFINITY).clamp(0.0, 1.0)
}

fn target_matches(seed: &ClinicalMolecularSeed, row: &MolecularEvidenceRow) -> bool {
    let Some(hint) = seed.target_hint.as_deref() else {
        return true;
    };
    let hint = hint.to_ascii_lowercase();
    row.target_id.to_ascii_lowercase().contains(&hint)
        || row.target_name.to_ascii_lowercase().contains(&hint)
}

fn sort_candidates(candidates: &mut [MolecularBridgeCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .rank_score
            .total_cmp(&left.rank_score)
            .then_with(|| right.binding_score.total_cmp(&left.binding_score))
            .then_with(|| left.compound_id.cmp(&right.compound_id))
            .then_with(|| left.target_id.cmp(&right.target_id))
    });
}

fn validate_params(params: &MolecularBridgeParams) -> Result<()> {
    if params.max_candidates == 0 {
        return invalid_params("max_candidates must be greater than zero");
    }
    if !score_is_valid(params.min_rank_score) {
        return invalid_params("min_rank_score must be finite and in [0,1]");
    }
    Ok(())
}

fn validate_seed(seed: &ClinicalMolecularSeed) -> Result<()> {
    if seed.seed_id.trim().is_empty()
        || seed.disease_id.trim().is_empty()
        || seed.disease_name.trim().is_empty()
    {
        return invalid_params("seed id and disease identifiers must not be empty");
    }
    if !score_is_valid(seed.grounded_confidence) {
        return invalid_params("seed grounded_confidence must be finite and in [0,1]");
    }
    if seed.provenance.is_empty() {
        return invalid_params("seed provenance must not be empty");
    }
    Ok(())
}

fn validate_evidence(row: &MolecularEvidenceRow, params: &MolecularBridgeParams) -> Result<()> {
    if row.evidence_id.trim().is_empty()
        || row.compound_id.trim().is_empty()
        || row.compound_name.trim().is_empty()
        || row.smiles.trim().is_empty()
        || row.target_id.trim().is_empty()
        || row.target_name.trim().is_empty()
        || row.disease_id.trim().is_empty()
        || row.assay_id.trim().is_empty()
    {
        return invalid_params("molecular evidence identifiers must not be empty");
    }
    if params.require_binding_affinity && row.affinity_nm.is_none() {
        return invalid_params("binding affinity is required for molecular bridge ranking");
    }
    if let Some(affinity) = row.affinity_nm
        && (!affinity.is_finite() || affinity <= 0.0)
    {
        return invalid_params("affinity_nm must be finite and greater than zero");
    }
    if !score_is_valid(row.activity_score)
        || !score_is_valid(row.target_confidence)
        || !score_is_valid(row.disease_confidence)
    {
        return invalid_params("molecular evidence scores must be finite and in [0,1]");
    }
    if let Some(sequence) = &row.protein_sequence
        && !sequence
            .bytes()
            .all(|byte| matches!(byte, b'A'..=b'Z' | b'*'))
    {
        return invalid_params("protein_sequence must contain uppercase amino-acid symbols");
    }
    if row.provenance.is_empty() {
        return invalid_params("molecular evidence provenance must not be empty");
    }
    Ok(())
}

fn score_is_valid(score: f32) -> bool {
    score.is_finite() && (0.0..=1.0).contains(&score)
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}
