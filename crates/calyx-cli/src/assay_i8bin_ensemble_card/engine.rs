use std::collections::BTreeMap;

use calyx_assay::{EnsembleCard, EnsembleConfig, ensemble_card};
use serde::Serialize;

use crate::assay_bits_validation::calyx_error_detail;

use super::matrix::{MatrixReadout, read_vectors};
use super::plan::{LoadedPlan, PlanSlot};
use super::request::I8binEnsembleRequest;
use super::rows::LabelRows;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct I8binEnsembleReport {
    pub(crate) plan_path: String,
    pub(crate) rows_jsonl: String,
    pub(crate) stream_report: Option<String>,
    pub(crate) target_class: usize,
    pub(crate) domain: String,
    pub(crate) row_count: usize,
    pub(crate) label_counts: BTreeMap<String, usize>,
    pub(crate) sample_rows_requested: usize,
    pub(crate) sample_rows_selected: usize,
    pub(crate) sample_positive_rows: usize,
    pub(crate) sample_negative_rows: usize,
    pub(crate) signature_rows: usize,
    pub(crate) lens_roster: Vec<LensReadout>,
    pub(crate) matrix: MatrixReadout,
    pub(crate) diversity: DiversityReadout,
    pub(crate) card: EnsembleCard,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LensReadout {
    pub(crate) slot: u16,
    pub(crate) name: String,
    pub(crate) lens_id: String,
    pub(crate) weights_sha256: String,
    pub(crate) runtime: String,
    pub(crate) dim: usize,
    pub(crate) max_batch: Option<usize>,
    pub(crate) elapsed_ms: Option<u64>,
    pub(crate) rows_per_sec: Option<f64>,
    pub(crate) bits_about: f32,
    pub(crate) association_family: String,
    pub(crate) corpus: String,
    pub(crate) queries: String,
    pub(crate) vault: String,
    pub(crate) manifest: Option<String>,
    pub(crate) corpus_rows_written: Option<usize>,
    pub(crate) query_rows_written: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct DiversityReadout {
    pub(crate) role: String,
    pub(crate) status: String,
    pub(crate) association_family_count: usize,
    pub(crate) association_families: BTreeMap<String, Vec<u16>>,
    pub(crate) family_span_pass: bool,
    pub(crate) redundancy_bound_pass: bool,
    pub(crate) no_collapse_pass: bool,
    pub(crate) n_eff: f32,
    pub(crate) n_eff_floor: f32,
    pub(crate) mean_pairwise_corr: f32,
    pub(crate) mean_pairwise_nmi: f32,
    pub(crate) max_redundancy: f32,
    pub(crate) sum_unique_pid_bits: f32,
    pub(crate) min_marginal_bits: f32,
    pub(crate) verdict: String,
}

pub(crate) fn evaluate(request: &I8binEnsembleRequest) -> Result<I8binEnsembleReport, String> {
    let plan = LoadedPlan::load(&request.plan, request.stream_report.as_deref())?;
    if plan.slots.len() < request.min_lenses {
        return Err(format!(
            "{}: i8bin ensemble card requires at least {} lenses; got {}",
            calyx_assay::CALYX_ASSAY_PANEL_TOO_SMALL,
            request.min_lenses,
            plan.slots.len()
        ));
    }
    let rows = LabelRows::load(&request.rows_jsonl, request.target_class)?;
    let sample = rows.balanced_sample(request.sample_rows)?;
    let vectors = read_vectors(
        &plan,
        &rows,
        &sample,
        request.signature_rows,
        request.nmi_bins,
    )?;
    let config = EnsembleConfig {
        source: format!(
            "assay i8bin-ensemble-card plan={} rows={} sample_rows={} signature_rows={}",
            request.plan.display(),
            request.rows_jsonl.display(),
            sample.indices.len(),
            vectors.matrix.signature_rows
        ),
        min_gate_lenses: request.min_lenses,
        min_marginal_bits: request.min_marginal_bits,
        max_redundancy: request.max_redundancy,
        nmi_bins: request.nmi_bins,
    };
    let mut card = ensemble_card(
        &vectors.lenses,
        &sample.labels,
        Some(&sample.groups),
        &config,
    )
    .map_err(calyx_error_detail)?;
    apply_full_matrix_redundancy(&mut card, &vectors.matrix);
    let lens_roster = plan
        .slots
        .iter()
        .zip(vectors.dims)
        .map(|(slot, dim)| lens_readout(slot, dim))
        .collect::<Vec<_>>();
    let diversity = diversity_readout(&lens_roster, &card, &vectors.matrix, request);
    Ok(I8binEnsembleReport {
        plan_path: request.plan.display().to_string(),
        rows_jsonl: request.rows_jsonl.display().to_string(),
        stream_report: request
            .stream_report
            .as_ref()
            .map(|path| path.display().to_string()),
        target_class: request.target_class,
        domain: request.domain.clone(),
        row_count: rows.labels.len(),
        label_counts: rows.label_counts,
        sample_rows_requested: request.sample_rows,
        sample_rows_selected: sample.indices.len(),
        sample_positive_rows: sample.positives,
        sample_negative_rows: sample.negatives,
        signature_rows: vectors.matrix.signature_rows,
        lens_roster,
        matrix: vectors.matrix,
        diversity,
        card,
    })
}

fn apply_full_matrix_redundancy(card: &mut EnsembleCard, matrix: &MatrixReadout) {
    card.n_eff = matrix.n_eff;
    for pair in &mut card.pairs {
        if let Some(readout) = matrix.pairs.iter().find(|readout| {
            (readout.slot_a == pair.slot_a.get() && readout.slot_b == pair.slot_b.get())
                || (readout.slot_a == pair.slot_b.get() && readout.slot_b == pair.slot_a.get())
        }) {
            pair.corr = readout.corr;
            pair.nmi = readout.nmi;
        }
    }
    for lens in &mut card.lenses {
        lens.max_pairwise_corr = matrix
            .pairs
            .iter()
            .filter(|pair| pair.slot_a == lens.slot.get() || pair.slot_b == lens.slot.get())
            .map(|pair| pair.corr)
            .fold(0.0_f32, f32::max);
        lens.max_pairwise_nmi = matrix
            .pairs
            .iter()
            .filter(|pair| pair.slot_a == lens.slot.get() || pair.slot_b == lens.slot.get())
            .map(|pair| pair.nmi)
            .fold(0.0_f32, f32::max);
    }
}

fn lens_readout(slot: &PlanSlot, dim: usize) -> LensReadout {
    LensReadout {
        slot: slot.slot,
        name: slot.name.clone(),
        lens_id: slot.lens_id.clone(),
        weights_sha256: slot.weights_sha256.clone(),
        runtime: slot
            .runtime
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        dim,
        max_batch: slot.max_batch,
        elapsed_ms: slot.elapsed_ms,
        rows_per_sec: slot.elapsed_ms.and_then(|elapsed| {
            (elapsed > 0)
                .then_some(slot.corpus_rows_written.unwrap_or(0) as f64 * 1000.0 / elapsed as f64)
        }),
        bits_about: slot.bits_about,
        association_family: association_family(&slot.name).to_string(),
        corpus: slot.corpus.display().to_string(),
        queries: slot.queries.display().to_string(),
        vault: slot.vault.display().to_string(),
        manifest: slot
            .manifest
            .as_ref()
            .map(|path| path.display().to_string()),
        corpus_rows_written: slot.corpus_rows_written,
        query_rows_written: slot.query_rows_written,
    }
}

fn diversity_readout(
    lenses: &[LensReadout],
    card: &EnsembleCard,
    matrix: &MatrixReadout,
    request: &I8binEnsembleRequest,
) -> DiversityReadout {
    let mut families = BTreeMap::<String, Vec<u16>>::new();
    for lens in lenses {
        families
            .entry(lens.association_family.clone())
            .or_default()
            .push(lens.slot);
    }
    let n_eff_floor = lenses.len() as f32 * 0.6;
    let family_span_pass = families.len() >= 2;
    let redundancy_bound_pass = matrix.n_eff >= n_eff_floor
        && matrix.mean_pairwise_corr <= request.max_redundancy
        && matrix.mean_pairwise_nmi <= request.max_redundancy;
    let no_collapse_pass = card
        .lenses
        .iter()
        .all(|lens| lens.marginal_bits >= request.min_marginal_bits);
    let sum_unique_pid_bits = card
        .lenses
        .iter()
        .map(|lens| lens.pid.unique_bits)
        .sum::<f32>();
    let status = if family_span_pass && redundancy_bound_pass && no_collapse_pass {
        "gate_candidate"
    } else {
        "diagnostic_only"
    };
    DiversityReadout {
        role: "a37_homogeneous_baseline_control".to_string(),
        status: status.to_string(),
        association_family_count: families.len(),
        association_families: families,
        family_span_pass,
        redundancy_bound_pass,
        no_collapse_pass,
        n_eff: matrix.n_eff,
        n_eff_floor,
        mean_pairwise_corr: matrix.mean_pairwise_corr,
        mean_pairwise_nmi: matrix.mean_pairwise_nmi,
        max_redundancy: request.max_redundancy,
        sum_unique_pid_bits,
        min_marginal_bits: request.min_marginal_bits,
        verdict: format!(
            "A37 status={status}; family_span={family_span_pass}; redundancy_bound={redundancy_bound_pass}; no_collapse={no_collapse_pass}"
        ),
    }
}

fn association_family(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let tokens = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if lower.contains("splade") || lower.contains("sparse") || lower.contains("lexical") {
        "lexical-sparse"
    } else if lower.contains("entity") || lower.contains("cameo") || lower.contains("graph") {
        "entity-graph"
    } else if lower.contains("byte") || lower.contains("char") {
        "char-byte"
    } else if tokens.iter().any(|token| matches!(*token, "ast" | "cfg"))
        || lower.contains("structural")
        || lower.contains("dataflow")
    {
        "structural"
    } else if lower.contains("rerank") || lower.contains("cross-encoder") {
        "reranker-asymmetric"
    } else if lower.contains("temporal") || lower.contains("time") {
        "temporal-sidecar"
    } else {
        "dense-semantic"
    }
}
