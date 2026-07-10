use super::*;

pub(super) fn validate_evidence(evidence: &LiveCalyxNativeEvidence) -> Result<()> {
    validate_panel(&evidence.panel)?;
    validate_kernel(&evidence.kernel_recall, &evidence.panel.domain)?;
    validate_calibration(&evidence.calibration, &evidence.panel.domain)?;
    validate_goodhart(&evidence.goodhart, &evidence.goodhart_held_out)?;
    validate_mistakes(&evidence.mistake_replay)
}

fn validate_panel(report: &PolyPanelSufficiencyReport) -> Result<()> {
    if report.schema_version != POLY_PANEL_SUFFICIENCY_SCHEMA_VERSION
        || report.artifact_kind != POLY_PANEL_SUFFICIENCY_ARTIFACT_KIND
        || report.domain.trim().is_empty()
        || report.panel_version == 0
        || report.n_samples != report.assay_card.n_samples
        || report.lens_count != report.assay_card.panel_lens_count
        || report.sufficient != report.assay_card.sufficient
        || !non_negative(report.panel_bits as f64)
        || !non_negative(report.anchor_entropy_bits as f64)
    {
        return invalid("panel-sufficiency report is malformed or internally inconsistent");
    }
    Ok(())
}

fn validate_kernel(report: &ComputedKernelRecall, domain: &str) -> Result<()> {
    let measured = report.recall.ratio as f64;
    let derived_ratio = report.recall.kernel_only / report.recall.full;
    if report.schema_version != COMPUTED_KERNEL_RECALL_SCHEMA_VERSION
        || report.artifact_kind != COMPUTED_KERNEL_RECALL_ARTIFACT_KIND
        || report.domain.slug() != domain
        || report.corpus_len == 0
        || report.n_queries_tested == 0
        || report.n_queries_tested != report.recall.n_queries_tested
        || report.fvs_kernel.kernel_member_count == 0
        || report.fvs_kernel.kernel_member_count != report.fvs_kernel.kernel_members.len()
        || !unit(report.recall.kernel_only as f64)
        || !unit(report.recall.full as f64)
        || report.recall.full <= 0.0
        || (report.recall.ratio - derived_ratio).abs() > FLOAT_EPSILON as f32
        || report.recall.held_out.len() != report.n_queries_tested
        || !unit(report.measured_ratio)
        || (report.measured_ratio - measured).abs() > FLOAT_EPSILON
        || (report.min_ratio - POLY_KERNEL_RECALL_MIN_RATIO).abs() > FLOAT_EPSILON
        || report.gate_passed != (report.measured_ratio >= report.min_ratio)
    {
        return invalid("computed-kernel recall report is malformed or inconsistent with policy");
    }
    Ok(())
}

fn validate_calibration(report: &CalibrationRefitReport, domain: &str) -> Result<()> {
    let expected_improvement = report.slope.brier_raw - report.slope.brier_calibrated;
    if report.schema_version != CALIBRATION_REFIT_SCHEMA_VERSION
        || report.artifact_kind != CALIBRATION_REFIT_ARTIFACT_KIND
        || report.slope.domain != domain
        || report.slope.horizon_bucket.trim().is_empty()
        || report.observation_count != report.slope.n
        || report.observation_count < MIN_CALIBRATION_SAMPLES
        || report.positives == 0
        || report.positives >= report.observation_count
        || !report.brier_improvement.is_finite()
        || report.brier_improvement <= 0.0
        || (report.brier_improvement - expected_improvement).abs() > FLOAT_EPSILON
    {
        return invalid("calibration refit is malformed, mismatched, or non-improving");
    }
    Ok(())
}

fn validate_goodhart(report: &GoodhartReport, held_out: &HeldOutSet) -> Result<()> {
    let in_region = report.in_region_frac.unwrap_or(f64::NAN);
    if !held_out.sealed
        || held_out.grounded_anchor_count == 0
        || held_out.before.is_none()
        || held_out.after.is_none()
        || !unit(in_region)
        || report.passed != report.violations.is_empty()
    {
        return invalid("Goodhart report or sealed held-out set is malformed or inconsistent");
    }
    Ok(())
}

fn validate_mistakes(report: &RegressionReport) -> Result<()> {
    let recurring = report.results.iter().filter(|row| row.recurred).count();
    if report.results.is_empty()
        || report.regression_count != recurring
        || report.passed != (recurring == 0)
    {
        return invalid("mistake-replay regression report is empty or internally inconsistent");
    }
    Ok(())
}

fn unit(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}
