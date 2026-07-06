use std::collections::BTreeMap;
use std::fs;

use serde::Deserialize;

use crate::error::CliResult;

use super::super::args::Args;
use super::super::{io_error, local_error};

#[derive(Clone, Debug, Deserialize)]
struct BitsReport {
    lenses: Option<Vec<BitsLens>>,
    report: Option<BitsReportInner>,
}

#[derive(Clone, Debug, Deserialize)]
struct BitsReportInner {
    lenses: Vec<BitsLens>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct BitsLens {
    pub(super) name: String,
    pub(super) bits_about: f32,
    pub(super) admitted: bool,
}

pub(super) fn streamable_for_mode(bits: &BitsLens, args: &Args) -> bool {
    bits.bits_about.is_finite()
        && bits.bits_about >= args.min_bits
        && (bits.admitted || !args.mode.requires_gate())
}

pub(super) fn load_bits(args: &Args) -> CliResult<BTreeMap<String, BitsLens>> {
    if args.a37_admission_cf_root.is_some() {
        return load_a37_admission_bits(args);
    }
    if args.diagnostic_bootstrap_without_admission() {
        return Ok(BTreeMap::new());
    }
    if args.mode.requires_gate() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_REQUIRED",
            "--bits-report is diagnostic-only; gate mode must read A37 admission from Calyx/Aster",
            "write and read the A37 admission row through Calyx/Aster Graph CF before streaming",
        ));
    }
    let bits_report = args.bits_report.as_ref().ok_or_else(|| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_MISSING",
            "missing --bits-report",
            "pass a diagnostic bits report or a DB-native A37 admission CF root",
        )
    })?;
    let report: BitsReport = serde_json::from_slice(&fs::read(bits_report).map_err(io_error)?)
        .map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_INVALID",
                format!("parse {} failed: {error}", bits_report.display()),
                "pass assay_abundance.json or full bits-validate evidence",
            )
        })?;
    let lenses = report
        .lenses
        .or_else(|| report.report.map(|inner| inner.lenses))
        .ok_or_else(|| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_INVALID",
                "bits report missing lenses",
                "pass a bits report with per-lens bits_about",
            )
        })?;
    Ok(lenses
        .into_iter()
        .map(|lens| (lens.name.clone(), lens))
        .collect())
}

pub(super) fn diagnostic_bootstrap_bits(name: &str, args: &Args) -> BitsLens {
    BitsLens {
        name: name.to_string(),
        bits_about: args.min_bits,
        admitted: false,
    }
}

pub(super) fn load_a37_admission(
    args: &Args,
) -> CliResult<crate::assay_multi_anchor_card::model::MultiAnchorReport> {
    let cf_root = args.a37_admission_cf_root.as_ref().ok_or_else(|| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_MISSING",
            "missing --a37-admission-cf-root",
            "pass the A37 admission Graph CF root",
        )
    })?;
    let (report, _readback) = crate::a37_admission_store::read::<
        crate::assay_multi_anchor_card::model::MultiAnchorReport,
    >(cf_root, &args.a37_admission_key)
    .map_err(|error| {
        local_error(
            error.code,
            error.message,
            "write and read the A37 admission record through Calyx/Aster Graph CF",
        )
    })?;
    Ok(report)
}

fn load_a37_admission_bits(args: &Args) -> CliResult<BTreeMap<String, BitsLens>> {
    let report = load_a37_admission(args)?;
    if report.lenses.len() != report.lens_count {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_INVALID",
            format!(
                "A37 admission lens_count={} lenses={}",
                report.lens_count,
                report.lenses.len()
            ),
            "rewrite the DB-native A37 admission from a valid multi-anchor card",
        ));
    }
    let mut out = BTreeMap::new();
    for lens in report.lenses {
        if lens.name.trim().is_empty() || !lens.best_marginal_bits.is_finite() {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_INVALID",
                format!(
                    "A37 admission lens slot={} name='{}' best_marginal_bits={}",
                    lens.slot, lens.name, lens.best_marginal_bits
                ),
                "rewrite the DB-native A37 admission with finite named lens rows",
            ));
        }
        if out
            .insert(
                lens.name.clone(),
                BitsLens {
                    name: lens.name,
                    bits_about: lens.best_marginal_bits,
                    admitted: lens.passed && report.gate_passed,
                },
            )
            .is_some()
        {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_A37_DB_INVALID",
                "A37 admission contains duplicate lens names",
                "rewrite the DB-native A37 admission with a unique lens roster",
            ));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::assay_multi_anchor_card::model::{
        LensEvidence, MultiAnchorReport, TargetLensValue, TargetSummary,
    };
    use crate::assay_stream_fbin::args::StreamMode;
    use crate::assay_stream_fbin::format::VectorFormat;

    use super::*;

    #[test]
    fn db_admission_supplies_streamable_bits() {
        let root =
            std::env::temp_dir().join(format!("calyx-stream-a37-db-bits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let report = report(&["lens_a", "lens_b"]);
        crate::a37_admission_store::write(&root, "unit", &report).unwrap();
        let args = args(&root);

        let bits = load_bits(&args).unwrap();

        assert_eq!(bits["lens_a"].bits_about, 0.07);
        assert!(bits["lens_b"].admitted);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn args(root: &std::path::Path) -> Args {
        Args {
            rows_jsonl: PathBuf::from("rows.jsonl"),
            out_dir: PathBuf::from("out"),
            dataset: "unit".to_string(),
            target_class: 1,
            manifests: Vec::new(),
            lens_template_cf_root: None,
            lens_template_key: crate::assay_stream_fbin::template::DEFAULT_ASSOCIATION_KEY
                .to_string(),
            lens_template_specs: Vec::new(),
            bits_report: None,
            a37_admission_cf_root: Some(root.to_path_buf()),
            a37_admission_key: "unit".to_string(),
            query_count: 1,
            limit_per_class: None,
            batch_size: 1,
            cost_override_json: None,
            embedding_model_id: None,
            min_bits: 0.05,
            vector_format: VectorFormat::Fbin,
            mode: StreamMode::Gate,
            worker_report: None,
            worker_slot: None,
            lens_parallelism: 1,
            worker_gpu_mem_limit_mib: None,
            emit_artifacts: true,
        }
    }

    fn report(names: &[&str]) -> MultiAnchorReport {
        MultiAnchorReport {
            schema_version: 1,
            role: "a37_multi_anchor_admission_card".to_string(),
            status: calyx_assay::A37_DIVERSITY_GATE_PASSED.to_string(),
            mode: "gate".to_string(),
            gate_passed: true,
            report_count: 1,
            lens_count: names.len(),
            passing_lens_count: names.len(),
            min_lenses: 2,
            min_marginal_bits: 0.05,
            max_redundancy: 0.6,
            family_span_pass: true,
            redundancy_bound_pass: true,
            no_collapse_pass: true,
            association_family_count: 2,
            association_families: BTreeMap::new(),
            min_best_marginal_bits: 0.07,
            max_best_marginal_bits: 0.08,
            weakest_lens: names[0].to_string(),
            target_summaries: vec![TargetSummary {
                target_class: 1,
                domain: "unit".to_string(),
                report_path: "db".to_string(),
                status: "gate_passed".to_string(),
                no_collapse_pass: true,
                family_span_pass: true,
                redundancy_bound_pass: true,
                n_eff: names.len() as f32,
                panel_bits: 1.0,
                max_marginal_bits: 0.08,
                keep_count: names.len(),
                park_count: 0,
            }],
            lenses: names
                .iter()
                .enumerate()
                .map(|(slot, name)| LensEvidence {
                    slot: slot as u16,
                    name: (*name).to_string(),
                    association_family: "unit".to_string(),
                    passed: true,
                    best_target_class: 1,
                    best_domain: "unit".to_string(),
                    best_marginal_bits: 0.07 + slot as f32 * 0.01,
                    best_solo_bits: 0.1,
                    target_values: vec![TargetLensValue {
                        target_class: 1,
                        domain: "unit".to_string(),
                        marginal_bits: 0.07,
                        solo_bits: 0.1,
                        decision: "Keep".to_string(),
                    }],
                })
                .collect(),
            source_reports: vec!["db".to_string()],
        }
    }
}
