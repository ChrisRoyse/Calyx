use std::collections::BTreeMap;

use calyx_assay::{
    A37_DIVERSITY_GATE_PASSED, A37DiversityGate, EnsembleCard, EnsembleConfig,
    a37_association_family, ensemble_card_with_redundancy,
};
use serde::Serialize;

use crate::assay_bits_validation::calyx_error_detail;

use super::label_store::{self, LabelAnchorDbReadback};
use super::matrix::{MatrixReadout, read_vectors};
use super::plan::{LoadedPlan, PlanSlot, PlanSourceReadout};
use super::request::I8binEnsembleRequest;
use super::rows::LabelRows;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct I8binEnsembleReport {
    pub(crate) plan_path: String,
    pub(crate) plan_source: PlanSourceReadout,
    pub(crate) rows_jsonl: String,
    pub(crate) label_source: LabelSourceReadout,
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
    pub(crate) a37_mode: String,
    pub(crate) a37_gate_required: bool,
    pub(crate) lens_roster: Vec<LensReadout>,
    pub(crate) matrix: MatrixReadout,
    pub(crate) diversity: A37DiversityGate,
    pub(crate) card: EnsembleCard,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LabelSourceReadout {
    pub(crate) mode: String,
    pub(crate) rows_jsonl: Option<String>,
    pub(crate) cf_root: Option<String>,
    pub(crate) association_key: Option<String>,
    pub(crate) row_count: usize,
    pub(crate) positive_count: usize,
    pub(crate) negative_count: usize,
    pub(crate) imported_rows_sha256: Option<String>,
    pub(crate) db_readback: Option<LabelAnchorDbReadback>,
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

pub(crate) fn evaluate(request: &I8binEnsembleRequest) -> Result<I8binEnsembleReport, String> {
    let plan = LoadedPlan::load(request)?;
    if plan.slots.len() < request.min_lenses {
        return Err(format!(
            "{}: i8bin ensemble card requires at least {} lenses; got {}",
            calyx_assay::CALYX_ASSAY_PANEL_TOO_SMALL,
            request.min_lenses,
            plan.slots.len()
        ));
    }
    let (rows, label_source) = load_labels(request)?;
    let sample = rows.balanced_sample(request.sample_rows)?;
    let vectors = read_vectors(
        &plan,
        &rows,
        &sample,
        request.signature_rows,
        request.nmi_bins,
    )?;
    let plan_ref = request
        .plan
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| {
            format!(
                "aster-graph-cf:{}",
                request
                    .plan_cf_root
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            )
        });
    let config = EnsembleConfig {
        source: format!(
            "assay i8bin-ensemble-card plan={} rows={} sample_rows={} signature_rows={}",
            plan_ref.as_str(),
            label_source_name(&label_source),
            sample.indices.len(),
            vectors.redundancy.method.row_count
        ),
        min_gate_lenses: request.min_lenses,
        min_marginal_bits: request.min_marginal_bits,
        max_redundancy: request.max_redundancy,
        nmi_bins: request.nmi_bins,
    };
    let card = ensemble_card_with_redundancy(
        &vectors.lenses,
        &sample.labels,
        Some(&sample.groups),
        &config,
        &vectors.redundancy,
    )
    .map_err(calyx_error_detail)?;
    let matrix = MatrixReadout::from_card(&card)?;
    let lens_roster = plan
        .slots
        .iter()
        .zip(vectors.dims)
        .map(|(slot, dim)| lens_readout(slot, dim))
        .collect::<Vec<_>>();
    let diversity = card.a37_diversity.clone();
    Ok(I8binEnsembleReport {
        plan_path: plan_ref,
        plan_source: plan.source,
        rows_jsonl: request.rows_jsonl.display().to_string(),
        label_source,
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
        signature_rows: matrix.signature_rows,
        a37_mode: request.mode.as_str().to_string(),
        a37_gate_required: request.mode.requires_gate(),
        lens_roster,
        matrix,
        diversity,
        card,
    })
}

fn load_labels(request: &I8binEnsembleRequest) -> Result<(LabelRows, LabelSourceReadout), String> {
    if let Some(cf_root) = request.labels_cf_root.as_ref() {
        let loaded = label_store::read(cf_root, &request.labels_key)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        if loaded.manifest.target_class != request.target_class {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_LABELS_TARGET_MISMATCH: DB label target_class={} request target_class={}",
                loaded.manifest.target_class, request.target_class
            ));
        }
        let readback = loaded.db_readback.clone();
        let manifest = loaded.manifest;
        let rows = LabelRows::from_parts(
            loaded.labels,
            manifest.label_counts.clone(),
            "CALYX_FSV_ASSAY_I8BIN_LABELS_INVALID",
        )?;
        let source = LabelSourceReadout {
            mode: "aster_graph_cf".to_string(),
            rows_jsonl: None,
            cf_root: Some(cf_root.display().to_string()),
            association_key: Some(request.labels_key.clone()),
            row_count: manifest.row_count,
            positive_count: manifest.positive_count,
            negative_count: manifest.negative_count,
            imported_rows_sha256: Some(manifest.imported_rows_sha256),
            db_readback: Some(readback),
        };
        return Ok((rows, source));
    }
    if request.mode.requires_gate() {
        return Err(
            "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: gate mode requires labels from Calyx/Aster Graph CF"
                .to_string(),
        );
    }
    let rows = LabelRows::load_file(&request.rows_jsonl, request.target_class)?;
    let positive_count = rows.labels.iter().filter(|label| **label).count();
    let source = LabelSourceReadout {
        mode: "rows_jsonl_diagnostic_import".to_string(),
        rows_jsonl: Some(request.rows_jsonl.display().to_string()),
        cf_root: None,
        association_key: None,
        row_count: rows.labels.len(),
        positive_count,
        negative_count: rows.labels.len().saturating_sub(positive_count),
        imported_rows_sha256: None,
        db_readback: None,
    };
    Ok((rows, source))
}

fn label_source_name(source: &LabelSourceReadout) -> String {
    source
        .cf_root
        .as_ref()
        .zip(source.association_key.as_ref())
        .map(|(root, key)| format!("aster-graph-cf:{root}:{key}"))
        .or_else(|| source.rows_jsonl.clone())
        .unwrap_or_else(|| "unknown".to_string())
}

pub(crate) fn enforce_a37_mode(
    request: &I8binEnsembleRequest,
    report: &I8binEnsembleReport,
) -> Result<(), String> {
    if !request.mode.requires_gate() || report.diversity.status == A37_DIVERSITY_GATE_PASSED {
        return Ok(());
    }
    Err(format!(
        "CALYX_FSV_ASSAY_A37_DIVERSITY_GATE_REFUSED: A37 gate mode requires status={} but got {}; {}",
        A37_DIVERSITY_GATE_PASSED, report.diversity.status, report.diversity.verdict
    ))
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
        association_family: a37_association_family(&slot.name).to_string(),
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
