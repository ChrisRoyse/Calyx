use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_assay::{
    A37_DIVERSITY_DIAGNOSTIC_ONLY, A37_DIVERSITY_GATE_PASSED, EnsembleCard, a37_association_family,
};

use super::CODE_INVALID_REPORT;
use super::model::{
    InputReport, LensEvidence, LoadedReport, MultiAnchorReport, TargetLensValue, TargetSummary,
};
use super::request::Request;

pub(crate) fn evaluate(request: &Request) -> Result<MultiAnchorReport, String> {
    let loaded = request
        .reports
        .iter()
        .map(|path| load_report(path))
        .collect::<Result<Vec<_>, _>>()?;
    validate_rosters(&loaded, request.min_lenses)?;

    let lens_count = loaded[0].report.card.lenses.len();
    let target_summaries = loaded
        .iter()
        .map(|input| target_summary(input, request.max_redundancy))
        .collect::<Result<Vec<_>, _>>()?;
    let redundancy_bound_pass = target_summaries
        .iter()
        .all(|target| target.redundancy_bound_pass);
    let (families, family_span_pass) = association_families(&loaded[0].report.card);
    let lenses = lens_evidence(&loaded, request.min_marginal_bits)?;
    let passing_lens_count = lenses.iter().filter(|lens| lens.passed).count();
    let no_collapse_pass = passing_lens_count == lens_count;
    let min_best_marginal_bits = lenses
        .iter()
        .map(|lens| lens.best_marginal_bits)
        .fold(f32::INFINITY, f32::min);
    let max_best_marginal_bits = lenses
        .iter()
        .map(|lens| lens.best_marginal_bits)
        .fold(f32::NEG_INFINITY, f32::max);
    let weakest_lens = lenses
        .iter()
        .min_by(|left, right| left.best_marginal_bits.total_cmp(&right.best_marginal_bits))
        .map(|lens| lens.name.clone())
        .unwrap_or_else(|| "none".to_string());
    let gate_passed = family_span_pass && redundancy_bound_pass && no_collapse_pass;
    let status = if gate_passed {
        A37_DIVERSITY_GATE_PASSED
    } else {
        A37_DIVERSITY_DIAGNOSTIC_ONLY
    };
    Ok(MultiAnchorReport {
        schema_version: 1,
        role: "a37_multi_anchor_admission_card".to_string(),
        status: status.to_string(),
        mode: request.mode.as_str().to_string(),
        gate_passed,
        report_count: loaded.len(),
        lens_count,
        passing_lens_count,
        min_lenses: request.min_lenses,
        min_marginal_bits: request.min_marginal_bits,
        max_redundancy: request.max_redundancy,
        family_span_pass,
        redundancy_bound_pass,
        no_collapse_pass,
        association_family_count: families.len(),
        association_families: families,
        min_best_marginal_bits,
        max_best_marginal_bits,
        weakest_lens,
        target_summaries,
        lenses,
        source_reports: loaded
            .iter()
            .map(|input| input.path.display().to_string())
            .collect(),
    })
}

fn load_report(path: &Path) -> Result<LoadedReport, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "{CODE_INVALID_REPORT}: could not read {}: {error}",
            path.display()
        )
    })?;
    let report = serde_json::from_slice::<InputReport>(&bytes).map_err(|error| {
        format!(
            "{CODE_INVALID_REPORT}: could not parse {}: {error}",
            path.display()
        )
    })?;
    validate_card(path, &report.card)?;
    Ok(LoadedReport {
        path: path.to_path_buf(),
        report,
    })
}

fn validate_card(path: &Path, card: &EnsembleCard) -> Result<(), String> {
    if card.lenses.is_empty() {
        return Err(format!(
            "{CODE_INVALID_REPORT}: {} card has no lenses",
            path.display()
        ));
    }
    finite(path, "panel_bits", card.panel_bits)?;
    finite(path, "n_eff", card.n_eff)?;
    for lens in &card.lenses {
        finite(path, "lens.solo_bits", lens.solo_bits)?;
        finite(path, "lens.marginal_bits", lens.marginal_bits)?;
    }
    Ok(())
}

fn validate_rosters(inputs: &[LoadedReport], min_lenses: usize) -> Result<(), String> {
    let first = &inputs[0].report.card.lenses;
    if first.len() < min_lenses {
        return Err(format!(
            "{}: multi-anchor card requires at least {} lenses; got {}",
            calyx_assay::CALYX_ASSAY_PANEL_TOO_SMALL,
            min_lenses,
            first.len()
        ));
    }
    let expected = first
        .iter()
        .map(|lens| (lens.slot.get(), lens.name.clone()))
        .collect::<Vec<_>>();
    for input in inputs.iter().skip(1) {
        let got = input
            .report
            .card
            .lenses
            .iter()
            .map(|lens| (lens.slot.get(), lens.name.clone()))
            .collect::<Vec<_>>();
        if got != expected {
            return Err(format!(
                "{CODE_INVALID_REPORT}: {} lens roster differs from first report",
                input.path.display()
            ));
        }
    }
    Ok(())
}

fn target_summary(input: &LoadedReport, max_redundancy: f32) -> Result<TargetSummary, String> {
    let card = &input.report.card;
    let max_marginal_bits = card
        .lenses
        .iter()
        .map(|lens| lens.marginal_bits)
        .fold(f32::NEG_INFINITY, f32::max);
    let n_eff_floor = card.lenses.len().max(10) as f32 * 0.6;
    let redundancy_bound_pass = card.n_eff >= n_eff_floor
        && card.a37_diversity.mean_pairwise_corr <= max_redundancy
        && card.a37_diversity.mean_pairwise_nmi <= max_redundancy;
    finite(&input.path, "target.max_marginal_bits", max_marginal_bits)?;
    Ok(TargetSummary {
        target_class: input.report.target_class,
        domain: input.report.domain.clone(),
        report_path: input.path.display().to_string(),
        status: card.a37_diversity.status.clone(),
        no_collapse_pass: card.a37_diversity.no_collapse_pass,
        family_span_pass: card.a37_diversity.family_span_pass,
        redundancy_bound_pass,
        n_eff: card.n_eff,
        panel_bits: card.panel_bits,
        max_marginal_bits,
        keep_count: card.keep_count,
        park_count: card.park_count,
    })
}

fn association_families(card: &EnsembleCard) -> (BTreeMap<String, Vec<u16>>, bool) {
    let mut families = BTreeMap::<String, Vec<u16>>::new();
    for lens in &card.lenses {
        let family = a37_association_family(&lens.name);
        if family != "temporal_sidecar" {
            families
                .entry(family.to_string())
                .or_default()
                .push(lens.slot.get());
        }
    }
    let pass = families.len() >= 2;
    (families, pass)
}

fn lens_evidence(
    inputs: &[LoadedReport],
    min_marginal_bits: f32,
) -> Result<Vec<LensEvidence>, String> {
    let mut out = Vec::new();
    for lens_idx in 0..inputs[0].report.card.lenses.len() {
        let first = &inputs[0].report.card.lenses[lens_idx];
        let mut target_values = Vec::new();
        for input in inputs {
            let lens = &input.report.card.lenses[lens_idx];
            target_values.push(TargetLensValue {
                target_class: input.report.target_class,
                domain: input.report.domain.clone(),
                marginal_bits: lens.marginal_bits,
                solo_bits: lens.solo_bits,
                decision: format!("{:?}", lens.decision),
            });
        }
        let best = target_values
            .iter()
            .max_by(|left, right| left.marginal_bits.total_cmp(&right.marginal_bits))
            .ok_or_else(|| format!("{CODE_INVALID_REPORT}: no target values"))?;
        out.push(LensEvidence {
            slot: first.slot.get(),
            name: first.name.clone(),
            association_family: a37_association_family(&first.name).to_string(),
            passed: best.marginal_bits >= min_marginal_bits,
            best_target_class: best.target_class,
            best_domain: best.domain.clone(),
            best_marginal_bits: best.marginal_bits,
            best_solo_bits: best.solo_bits,
            target_values,
        });
    }
    Ok(out)
}

fn finite(path: &Path, field: &str, value: f32) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!(
            "{CODE_INVALID_REPORT}: {} has non-finite {field}",
            path.display()
        ))
    }
}
