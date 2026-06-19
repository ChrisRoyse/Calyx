use std::fs;
use std::path::Path;

use serde::Serialize;

use super::engine::EnsembleCardReport;
use super::request::EnsembleCardRequest;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct EnsembleMetricEvidence {
    pub(crate) metrics_dir: String,
    pub(crate) ensemble_card_path: String,
    pub(crate) lens_values_path: String,
    pub(crate) pair_values_path: String,
    pub(crate) cf_root: String,
    pub(crate) assay_cf_rows_persisted: usize,
    pub(crate) assay_cf_rows_readback: usize,
    pub(crate) ensemble_card_row_present: bool,
    pub(crate) ensemble_card_payload_readback: bool,
    pub(crate) report: EnsembleCardReport,
}

pub(crate) fn write_outputs(
    request: &EnsembleCardRequest,
    report: &EnsembleCardReport,
) -> Result<EnsembleMetricEvidence, String> {
    check_finite(report)?;
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;

    let ensemble_card_path = request.metrics_dir.join("ensemble_card.json");
    fs::write(
        &ensemble_card_path,
        serde_json::to_vec_pretty(&report.card).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let lens_values_path = request.metrics_dir.join("ensemble_lens_values.txt");
    fs::write(&lens_values_path, lens_values(report)).map_err(|error| error.to_string())?;

    let pair_values_path = request.metrics_dir.join("ensemble_pair_values.txt");
    fs::write(&pair_values_path, pair_values(report)).map_err(|error| error.to_string())?;

    Ok(EnsembleMetricEvidence {
        metrics_dir: request.metrics_dir.display().to_string(),
        ensemble_card_path: display(&ensemble_card_path),
        lens_values_path: display(&lens_values_path),
        pair_values_path: display(&pair_values_path),
        cf_root: report.cf_root.clone(),
        assay_cf_rows_persisted: report.assay_cf_rows_persisted,
        assay_cf_rows_readback: report.assay_cf_rows_readback,
        ensemble_card_row_present: report.ensemble_card_row_present,
        ensemble_card_payload_readback: report.ensemble_card_payload_readback,
        report: report.clone(),
    })
}

fn lens_values(report: &EnsembleCardReport) -> String {
    let mut out = String::new();
    for lens in &report.card.lenses {
        out.push_str(&format!(
            "lens={} slot={} solo={:.6} marginal={:.6} pid_unique={:.6} pid_redundant={:.6} pid_synergy={:.6} corr={:.6} nmi={:.6} decision={:?}\n",
            lens.name,
            lens.slot,
            lens.solo_bits,
            lens.marginal_bits,
            lens.pid.unique_bits,
            lens.pid.redundant_bits,
            lens.pid.synergistic_bits,
            lens.max_pairwise_corr,
            lens.max_pairwise_nmi,
            lens.decision
        ));
    }
    out
}

fn pair_values(report: &EnsembleCardReport) -> String {
    let mut out = String::new();
    for pair in &report.card.pairs {
        out.push_str(&format!(
            "pair={}+{} slots={}+{} corr={:.6} nmi={:.6} pair_bits={:.6} synergy_gain={:.6}\n",
            pair.a,
            pair.b,
            pair.slot_a,
            pair.slot_b,
            pair.corr,
            pair.nmi,
            pair.pair_bits,
            pair.synergy_gain_bits
        ));
    }
    out
}

fn check_finite(report: &EnsembleCardReport) -> Result<(), String> {
    let mut values = vec![
        ("anchor_entropy_bits", report.card.anchor_entropy_bits),
        ("panel_bits", report.card.panel_bits),
        ("panel_ci_low", report.card.panel_ci[0]),
        ("panel_ci_high", report.card.panel_ci[1]),
        ("n_eff", report.card.n_eff),
        ("deficit_bits", report.card.deficit_bits),
    ];
    for lens in &report.card.lenses {
        values.push(("lens.solo_bits", lens.solo_bits));
        values.push(("lens.marginal_bits", lens.marginal_bits));
        values.push(("lens.pid.unique_bits", lens.pid.unique_bits));
        values.push(("lens.pid.redundant_bits", lens.pid.redundant_bits));
        values.push(("lens.pid.synergistic_bits", lens.pid.synergistic_bits));
        values.push(("lens.max_pairwise_corr", lens.max_pairwise_corr));
        values.push(("lens.max_pairwise_nmi", lens.max_pairwise_nmi));
    }
    for pair in &report.card.pairs {
        values.push(("pair.corr", pair.corr));
        values.push(("pair.nmi", pair.nmi));
        values.push(("pair.pair_bits", pair.pair_bits));
        values.push(("pair.synergy_gain_bits", pair.synergy_gain_bits));
    }
    for (name, value) in values {
        if !value.is_finite() {
            return Err(format!("CALYX_FSV_ASSAY_NONFINITE_METRIC: {name}={value}"));
        }
    }
    Ok(())
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
