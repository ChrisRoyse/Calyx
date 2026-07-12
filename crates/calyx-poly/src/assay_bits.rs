//! Vault-backed per-slot Assay bits about a grounded outcome anchor (issue #50).
//!
//! This adapter owns the Poly boundary around `calyx-assay`: extract boolean
//! outcome labels from grounded anchors, enforce the pairwise redundancy
//! contract before persistence, run the real KSG mixed continuous/discrete MI
//! estimator per slot, persist scoped `AssayStore` rows into Aster's Assay CF,
//! then reload those rows from the vault before writing the report.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use calyx_assay::{
    AssayCacheKey, AssayRow, AssayStore, AssaySubject, EstimateBound, EstimatorKind,
    MIN_ASSAY_SAMPLES, MiEstimate, TrustTag, entropy_bits, ksg_mi_continuous_discrete,
    pair_redundancy,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{Anchor, AnchorKind, AnchorValue, CalyxError, Clock, SlotId, VaultId};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{PolyError, Result};
use crate::grounding::rollup_trust;
use crate::panel_diagnostics::{ESTIMATOR_KSG, PanelMatrix};

pub const ASSAY_BITS_SCHEMA_VERSION: &str = "poly.assay_bits.v2";
pub const ASSAY_BITS_ARTIFACT_KIND: &str = "poly_assay_bits";
pub const DEFAULT_ASSAY_BITS_K: usize = 3;

pub const ERR_ASSAY_BITS_INVALID_REQUEST: &str = "CALYX_POLY_ASSAY_BITS_INVALID_REQUEST";
pub const ERR_ASSAY_BITS_ANCHOR_KIND_MISMATCH: &str = "CALYX_POLY_ASSAY_BITS_ANCHOR_KIND_MISMATCH";
pub const ERR_ASSAY_BITS_NON_BOOL_ANCHOR: &str = "CALYX_POLY_ASSAY_BITS_NON_BOOL_ANCHOR";
pub const ERR_ASSAY_BITS_DEGENERATE_OUTCOME: &str = "CALYX_POLY_ASSAY_BITS_DEGENERATE_OUTCOME";
pub const ERR_ASSAY_BITS_READBACK_MISMATCH: &str = "CALYX_POLY_ASSAY_BITS_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssayBitsRequest {
    pub domain: String,
    pub panel_version: u32,
    pub corpus_shard: String,
    pub anchor_kind: AnchorKind,
    pub k_neighbors: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotAssayBits {
    pub slot: u16,
    pub slot_key: String,
    pub bits: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub max_pairwise_corr: f32,
    pub trust: TrustTag,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound: Option<EstimateBound>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RedundancyPair {
    pub slot_a: u16,
    pub slot_b: u16,
    pub key_a: String,
    pub key_b: String,
    pub correlation: f32,
    pub redundancy: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssayBitsReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub panel_version: u32,
    pub corpus_shard: String,
    pub vault_id: VaultId,
    pub anchor_kind: AnchorKind,
    pub estimator: String,
    pub k_neighbors: usize,
    pub slot_keys: Vec<String>,
    pub n_samples: usize,
    pub outcome_entropy_bits: f32,
    pub label_true_count: usize,
    pub label_false_count: usize,
    pub trust: TrustTag,
    pub persisted_seq: u64,
    pub persisted_rows: usize,
    pub slot_bits: Vec<SlotAssayBits>,
    pub redundancy_pairs: Vec<RedundancyPair>,
    pub assay_row_order: Vec<String>,
    pub assay_rows: Vec<AssayRow>,
    pub provenance_hash: String,
    pub computed_at: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssayBitsRun {
    pub report_path: PathBuf,
    pub report: AssayBitsReport,
    pub persisted_seq: u64,
    pub persisted_rows: usize,
}

pub fn run_assay_bits_to_vault<C: Clock>(
    vault: &AsterVault<C>,
    request: &AssayBitsRequest,
    matrix: &PanelMatrix,
    clock: &dyn Clock,
    output_dir: &Path,
) -> Result<AssayBitsRun> {
    validate_request(request, matrix)?;
    let vault_id = vault.vault_id();
    let key = AssayCacheKey::scoped(
        request.panel_version,
        request.corpus_shard.clone(),
        vault_id,
        request.anchor_kind.clone(),
    );
    let computation = compute_store(vault, request, matrix, clock, &key)?;
    let persisted_rows = computation.store.persist_to_vault(vault)?;
    let persisted_seq = vault.latest_seq();
    let assay_rows = readback_expected_rows(vault, &computation.store.rows())?;

    let report = AssayBitsReport {
        schema_version: ASSAY_BITS_SCHEMA_VERSION.to_string(),
        artifact_kind: ASSAY_BITS_ARTIFACT_KIND.to_string(),
        domain: request.domain.clone(),
        panel_version: request.panel_version,
        corpus_shard: request.corpus_shard.clone(),
        vault_id,
        anchor_kind: request.anchor_kind.clone(),
        estimator: ESTIMATOR_KSG.to_string(),
        k_neighbors: request.k_neighbors,
        slot_keys: matrix.slot_keys().to_vec(),
        n_samples: matrix.n_samples(),
        outcome_entropy_bits: computation.outcome_entropy_bits,
        label_true_count: computation.label_true_count,
        label_false_count: computation.label_false_count,
        trust: computation.trust,
        persisted_seq,
        persisted_rows,
        slot_bits: computation.slot_bits,
        redundancy_pairs: computation.redundancy_pairs,
        assay_row_order: assay_rows.iter().map(assay_row_id).collect(),
        provenance_hash: provenance_hash(request, matrix, &assay_rows),
        assay_rows,
        computed_at: clock.now(),
    };
    let report_path = write_assay_bits_report(output_dir, &report)?;
    Ok(AssayBitsRun {
        report_path,
        report,
        persisted_seq,
        persisted_rows,
    })
}

pub fn write_assay_bits_report(dir: &Path, report: &AssayBitsReport) -> Result<PathBuf> {
    let file_name = format!(
        "assay_bits_{}_v{}.json",
        sanitize(&report.domain),
        report.panel_version
    );
    crate::diagnostics_store::write_json(dir, &file_name, report)
}

pub fn read_assay_bits_report(path: &Path) -> Result<AssayBitsReport> {
    crate::diagnostics_store::read_json(path)
}

struct AssayBitsComputation {
    store: AssayStore,
    slot_bits: Vec<SlotAssayBits>,
    redundancy_pairs: Vec<RedundancyPair>,
    outcome_entropy_bits: f32,
    label_true_count: usize,
    label_false_count: usize,
    trust: TrustTag,
}

fn compute_store<C: Clock>(
    vault: &AsterVault<C>,
    request: &AssayBitsRequest,
    matrix: &PanelMatrix,
    clock: &dyn Clock,
    key: &AssayCacheKey,
) -> Result<AssayBitsComputation> {
    let labels = labels_for_anchors(matrix.anchors(), &request.anchor_kind)?;
    let label_true_count = labels.iter().filter(|&&label| label == 1).count();
    let label_false_count = labels.len() - label_true_count;
    if label_true_count == 0 || label_false_count == 0 {
        return Err(PolyError::diagnostics(
            ERR_ASSAY_BITS_DEGENERATE_OUTCOME,
            format!(
                "outcome is single-class ({label_true_count} of {} true); per-slot MI about a constant outcome is undefined",
                labels.len()
            ),
        ));
    }

    let trust = rollup_trust(matrix.anchors())?;
    let redundancy_pairs = redundancy_pairs(matrix)?;
    let max_corrs = max_corrs(matrix.slot_keys().len(), &redundancy_pairs);
    let written_at_seq = vault.latest_seq().saturating_add(1);
    let mut store = AssayStore::default();
    let mut slot_bits = Vec::with_capacity(matrix.slot_keys().len());

    for (idx, column) in matrix.columns().iter().enumerate() {
        let subject = AssaySubject::Lens {
            slot: checked_slot_id(idx)?,
        };
        let x: Vec<Vec<f32>> = column.iter().map(|&value| vec![value]).collect();
        let mut estimate = ksg_mi_continuous_discrete(&x, &labels, request.k_neighbors)?;
        estimate.trust = trust;
        let max_pairwise_corr = max_corrs[idx];
        store.put_with_payload(
            key.clone(),
            subject,
            estimate.clone(),
            "calyx_poly::assay_bits/calyx_assay::ksg_mi_continuous_discrete",
            written_at_seq,
            json!({
                "slot": idx,
                "slot_key": matrix.slot_keys()[idx],
                "max_pairwise_corr": format!("{max_pairwise_corr:.8}"),
                "computed_at": clock.now()
            }),
        );
        slot_bits.push(SlotAssayBits {
            slot: idx as u16,
            slot_key: matrix.slot_keys()[idx].clone(),
            bits: estimate.bits,
            ci_low: estimate.ci_low,
            ci_high: estimate.ci_high,
            max_pairwise_corr,
            trust: estimate.trust,
            bound: Some(estimate.bound),
        });
    }

    let outcome_entropy_bits = entropy_bits(&labels);
    let entropy_estimate = MiEstimate::point(
        outcome_entropy_bits,
        labels.len(),
        EstimatorKind::OutcomeEntropy,
        trust,
    );
    store.put_with_payload(
        key.clone(),
        AssaySubject::OutcomeEntropy,
        entropy_estimate,
        "calyx_poly::assay_bits/calyx_assay::entropy_bits",
        written_at_seq,
        json!({
            "label_false_count": label_false_count,
            "label_true_count": label_true_count,
            "computed_at": clock.now()
        }),
    );

    Ok(AssayBitsComputation {
        store,
        slot_bits,
        redundancy_pairs,
        outcome_entropy_bits,
        label_true_count,
        label_false_count,
        trust,
    })
}

fn validate_request(request: &AssayBitsRequest, matrix: &PanelMatrix) -> Result<()> {
    if request.domain.trim().is_empty()
        || request.corpus_shard.trim().is_empty()
        || request.k_neighbors == 0
    {
        return Err(PolyError::diagnostics(
            ERR_ASSAY_BITS_INVALID_REQUEST,
            "assay bits require non-empty domain/corpus_shard and k_neighbors > 0",
        ));
    }
    if matrix.n_samples() < MIN_ASSAY_SAMPLES {
        return Err(PolyError::from(CalyxError::assay_insufficient_samples(
            format!(
                "assay bits require at least {MIN_ASSAY_SAMPLES} paired samples, got {}",
                matrix.n_samples()
            ),
        )));
    }
    if matrix.slot_keys().len() > u16::MAX as usize {
        return Err(PolyError::diagnostics(
            ERR_ASSAY_BITS_INVALID_REQUEST,
            format!(
                "{} slots exceeds SlotId u16 capacity",
                matrix.slot_keys().len()
            ),
        ));
    }
    let mut seen = BTreeSet::new();
    for key in matrix.slot_keys() {
        if key.trim().is_empty() || !seen.insert(key) {
            return Err(PolyError::diagnostics(
                ERR_ASSAY_BITS_INVALID_REQUEST,
                format!("slot keys must be non-empty and unique, got '{key}'"),
            ));
        }
    }
    Ok(())
}

fn labels_for_anchors(anchors: &[Anchor], expected: &AnchorKind) -> Result<Vec<usize>> {
    anchors
        .iter()
        .map(|anchor| {
            if &anchor.kind != expected {
                return Err(PolyError::diagnostics(
                    ERR_ASSAY_BITS_ANCHOR_KIND_MISMATCH,
                    format!(
                        "anchor kind {:?} does not match requested outcome anchor {:?}",
                        anchor.kind, expected
                    ),
                ));
            }
            match &anchor.value {
                AnchorValue::Bool(value) => Ok(usize::from(*value)),
                other => Err(PolyError::diagnostics(
                    ERR_ASSAY_BITS_NON_BOOL_ANCHOR,
                    format!("assay bits require a boolean outcome anchor, got {other:?}"),
                )),
            }
        })
        .collect()
}

fn redundancy_pairs(matrix: &PanelMatrix) -> Result<Vec<RedundancyPair>> {
    let mut pairs = Vec::new();
    for i in 0..matrix.slot_keys().len() {
        for j in (i + 1)..matrix.slot_keys().len() {
            let correlation = pearson_corr(&matrix.columns()[i], &matrix.columns()[j])?;
            let redundancy = pair_redundancy(correlation)?;
            pairs.push(RedundancyPair {
                slot_a: i as u16,
                slot_b: j as u16,
                key_a: matrix.slot_keys()[i].clone(),
                key_b: matrix.slot_keys()[j].clone(),
                correlation,
                redundancy,
            });
        }
    }
    Ok(pairs)
}

fn pearson_corr(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() || left.len() < 2 {
        return Err(PolyError::from(CalyxError::assay_insufficient_samples(
            format!(
                "correlation requires equal paired samples with n >= 2, got left={} right={}",
                left.len(),
                right.len()
            ),
        )));
    }
    let n = left.len() as f32;
    let mean_left = left.iter().sum::<f32>() / n;
    let mean_right = right.iter().sum::<f32>() / n;
    let mut cov = 0.0;
    let mut var_left = 0.0;
    let mut var_right = 0.0;
    for (&a, &b) in left.iter().zip(right) {
        let da = a - mean_left;
        let db = b - mean_right;
        cov += da * db;
        var_left += da * da;
        var_right += db * db;
    }
    let denom = (var_left * var_right).sqrt();
    if !denom.is_finite() || denom <= f32::EPSILON {
        return Err(PolyError::from(CalyxError::assay_degenerate_input(
            "pairwise redundancy requires non-constant slot columns",
        )));
    }
    Ok((cov / denom).clamp(-1.0, 1.0))
}

fn max_corrs(slot_count: usize, pairs: &[RedundancyPair]) -> Vec<f32> {
    let mut out = vec![0.0f32; slot_count];
    for pair in pairs {
        out[pair.slot_a as usize] = out[pair.slot_a as usize].max(pair.correlation.abs());
        out[pair.slot_b as usize] = out[pair.slot_b as usize].max(pair.correlation.abs());
    }
    out
}

fn checked_slot_id(idx: usize) -> Result<SlotId> {
    let slot = u16::try_from(idx).map_err(|_| {
        PolyError::diagnostics(
            ERR_ASSAY_BITS_INVALID_REQUEST,
            format!("slot index {idx} exceeds SlotId u16 capacity"),
        )
    })?;
    Ok(SlotId::new(slot))
}

fn readback_expected_rows<C: Clock>(
    vault: &AsterVault<C>,
    expected: &[AssayRow],
) -> Result<Vec<AssayRow>> {
    let loaded = AssayStore::load_from_vault(vault)?;
    let mut readback = Vec::with_capacity(expected.len());
    for row in expected {
        let loaded_row = loaded
            .get(&row.cache_key, &row.subject)
            .ok_or_else(|| {
                PolyError::diagnostics(
                    ERR_ASSAY_BITS_READBACK_MISMATCH,
                    format!("missing Assay CF row {}", assay_row_id(row)),
                )
            })?
            .clone();
        if loaded_row != *row {
            let expected = serde_json::to_string(row)
                .unwrap_or_else(|err| format!("failed to encode expected row for debug: {err}"));
            let loaded = serde_json::to_string(&loaded_row)
                .unwrap_or_else(|err| format!("failed to encode loaded row for debug: {err}"));
            return Err(PolyError::diagnostics(
                ERR_ASSAY_BITS_READBACK_MISMATCH,
                format!(
                    "Assay CF row {} did not round-trip; expected={expected}; loaded={loaded}",
                    assay_row_id(row)
                ),
            ));
        }
        readback.push(loaded_row);
    }
    Ok(readback)
}

fn assay_row_id(row: &AssayRow) -> String {
    match &row.subject {
        AssaySubject::Lens { slot } => format!("lens:{}", slot.get()),
        AssaySubject::Pair { a, b } => format!("pair:{}:{}", a.get(), b.get()),
        AssaySubject::Panel => "panel".to_string(),
        AssaySubject::OutcomeEntropy => "outcome_entropy".to_string(),
        AssaySubject::EnsembleCard => "ensemble_card".to_string(),
    }
}

fn provenance_hash(request: &AssayBitsRequest, matrix: &PanelMatrix, rows: &[AssayRow]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ASSAY_BITS_SCHEMA_VERSION.as_bytes());
    hasher.update(&[0]);
    hasher.update(request.domain.as_bytes());
    hasher.update(&request.panel_version.to_le_bytes());
    hasher.update(request.corpus_shard.as_bytes());
    hasher.update(&(matrix.n_samples() as u64).to_le_bytes());
    for key in matrix.slot_keys() {
        hasher.update(key.as_bytes());
        hasher.update(&[0]);
    }
    for row in rows {
        hasher.update(assay_row_id(row).as_bytes());
        hasher.update(&row.estimate.bits.to_le_bytes());
        hasher.update(&row.estimate.ci_low.to_le_bytes());
        hasher.update(&row.estimate.ci_high.to_le_bytes());
        hasher.update(&[estimate_bound_tag(Some(row.estimate.bound))]);
    }
    hasher.finalize().to_hex().to_string()
}

fn estimate_bound_tag(bound: Option<EstimateBound>) -> u8 {
    match bound {
        None => 0,
        Some(EstimateBound::LowerBound) => 1,
        Some(EstimateBound::Point) => 2,
        Some(EstimateBound::UpperBound) => 3,
    }
}

fn sanitize(domain: &str) -> String {
    domain
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
