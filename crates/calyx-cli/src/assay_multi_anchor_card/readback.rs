use std::path::{Path, PathBuf};

use crate::a37_admission_store::{self, A37AdmissionDbReadback};

use super::CODE_INVALID_CONFIG;
use super::model::{LensEvidence, MultiAnchorReport, TargetSummary};

const DEFAULT_ASSOCIATION_KEY: &str = "a37_multi_anchor_admission";
const DEFAULT_LIMIT_LENSES: usize = 32;
const DEFAULT_LIMIT_TARGETS: usize = 16;

#[derive(Clone, Debug)]
struct ReadbackRequest {
    cf_root: PathBuf,
    association_key: String,
    limit_lenses: usize,
    limit_targets: usize,
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let request = ReadbackRequest::parse(args)?;
    let (report, readback) = load_report(&request.cf_root, &request.association_key)?;
    print!("{}", render_readback(&report, &readback, &request));
    Ok(())
}

pub(crate) fn load_report(
    cf_root: &Path,
    association_key: &str,
) -> Result<(MultiAnchorReport, A37AdmissionDbReadback), String> {
    a37_admission_store::read::<MultiAnchorReport>(cf_root, association_key)
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

fn render_readback(
    report: &MultiAnchorReport,
    readback: &A37AdmissionDbReadback,
    request: &ReadbackRequest,
) -> String {
    let mut out = String::new();
    out.push_str("a37_multi_anchor_readback\n");
    out.push_str(&format!("cf_root={}\n", readback.cf_root));
    out.push_str(&format!("association_key={}\n", readback.association_key));
    out.push_str(&format!("row_key_sha256={}\n", readback.row_key_sha256));
    out.push_str(&format!("value_bytes={}\n", readback.value_bytes));
    out.push_str(&format!("value_sha256={}\n", readback.value_sha256));
    out.push_str(&format!("readback_matches={}\n", readback.readback_matches));
    out.push_str(&format!(
        "status={} mode={} gate_passed={}\n",
        report.status, report.mode, report.gate_passed
    ));
    out.push_str(&format!(
        "report_count={} lens_count={} passing_lens_count={} min_lenses={}\n",
        report.report_count, report.lens_count, report.passing_lens_count, report.min_lenses
    ));
    out.push_str(&format!(
        "family_span_pass={} redundancy_bound_pass={} no_collapse_pass={}\n",
        report.family_span_pass, report.redundancy_bound_pass, report.no_collapse_pass
    ));
    out.push_str(&format!(
        "min_best_marginal_bits={:.9} max_best_marginal_bits={:.9} weakest_lens={}\n",
        report.min_best_marginal_bits, report.max_best_marginal_bits, report.weakest_lens
    ));
    out.push_str(&format!(
        "association_family_count={}\n",
        report.association_family_count
    ));
    for (family, slots) in &report.association_families {
        out.push_str(&format!("family={} slots={}\n", family, slot_list(slots)));
    }
    render_targets(&mut out, &report.target_summaries, request.limit_targets);
    render_lenses(&mut out, &report.lenses, request.limit_lenses);
    out
}

fn slot_list(slots: &[u16]) -> String {
    if slots.is_empty() {
        return "none".to_string();
    }
    slots
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn render_targets(out: &mut String, targets: &[TargetSummary], limit: usize) {
    out.push_str(&format!(
        "targets_shown={} targets_total={}\n",
        targets.len().min(limit),
        targets.len()
    ));
    for target in targets.iter().take(limit) {
        out.push_str(&format!(
            "target domain={} class={} status={} no_collapse={} redundancy={} n_eff={:.6} max_marginal_bits={:.6}\n",
            target.domain,
            target.target_class,
            target.status,
            target.no_collapse_pass,
            target.redundancy_bound_pass,
            target.n_eff,
            target.max_marginal_bits
        ));
    }
}

fn render_lenses(out: &mut String, lenses: &[LensEvidence], limit: usize) {
    let mut sorted = lenses.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.best_marginal_bits.total_cmp(&right.best_marginal_bits));
    out.push_str(&format!(
        "lenses_shown={} lenses_total={} order=weakest_first\n",
        sorted.len().min(limit),
        sorted.len()
    ));
    for lens in sorted.into_iter().take(limit) {
        out.push_str(&format!(
            "lens slot={} name={} family={} passed={} best_domain={} best_target={} best_marginal_bits={:.9} best_solo_bits={:.9}\n",
            lens.slot,
            lens.name,
            lens.association_family,
            lens.passed,
            lens.best_domain,
            lens.best_target_class,
            lens.best_marginal_bits,
            lens.best_solo_bits
        ));
    }
}

impl ReadbackRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut cf_root = None;
        let mut association_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut limit_lenses = DEFAULT_LIMIT_LENSES;
        let mut limit_targets = DEFAULT_LIMIT_TARGETS;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--cf-root" => {
                    cf_root = Some(PathBuf::from(value(args, idx, "--cf-root")?));
                    idx += 2;
                }
                "--association-key" => {
                    association_key = value(args, idx, "--association-key")?.to_string();
                    idx += 2;
                }
                "--limit-lenses" => {
                    limit_lenses = parse_limit(args, idx, "--limit-lenses")?;
                    idx += 2;
                }
                "--limit-targets" => {
                    limit_targets = parse_limit(args, idx, "--limit-targets")?;
                    idx += 2;
                }
                other => return Err(format!("{CODE_INVALID_CONFIG}: unknown arg {other}")),
            }
        }
        let cf_root = cf_root.ok_or_else(|| {
            format!("{CODE_INVALID_CONFIG}: --cf-root is required for multi-anchor readback")
        })?;
        if association_key.trim().is_empty() {
            return Err(format!(
                "{CODE_INVALID_CONFIG}: --association-key must be non-empty"
            ));
        }
        Ok(Self {
            cf_root,
            association_key,
            limit_lenses,
            limit_targets,
        })
    }
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{CODE_INVALID_CONFIG}: {flag} requires a value"))
}

fn parse_limit(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    let parsed = value(args, idx, flag)?
        .parse::<usize>()
        .map_err(|_| format!("{CODE_INVALID_CONFIG}: {flag} must be an unsigned integer"))?;
    if parsed == 0 {
        return Err(format!("{CODE_INVALID_CONFIG}: {flag} must be > 0"));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a37_admission_store;
    use crate::assay_multi_anchor_card::model::{
        LensEvidence, MultiAnchorReport, TargetLensValue, TargetSummary,
    };

    #[test]
    fn readback_output_is_plain_bounded_and_weakest_first() {
        let root = std::env::temp_dir().join(format!(
            "calyx-multi-anchor-readback-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let report = sample_report();
        let readback = a37_admission_store::write(&root, "unit-readback", &report).unwrap();
        let (loaded, loaded_readback) = load_report(&root, "unit-readback").unwrap();
        let request = ReadbackRequest {
            cf_root: root.clone(),
            association_key: "unit-readback".to_string(),
            limit_lenses: 1,
            limit_targets: 1,
        };

        let rendered = render_readback(&loaded, &loaded_readback, &request);

        assert_eq!(loaded.weakest_lens, "weak-lens");
        assert_eq!(readback.value_sha256, loaded_readback.value_sha256);
        assert!(rendered.contains("a37_multi_anchor_readback\n"));
        assert!(rendered.contains("readback_matches=true\n"));
        assert!(rendered.contains("targets_shown=1 targets_total=2\n"));
        assert!(rendered.contains("lenses_shown=1 lenses_total=2 order=weakest_first\n"));
        assert!(rendered.contains("name=weak-lens"));
        assert!(!rendered.contains('{'));
        assert!(!rendered.contains('['));
        let _ = std::fs::remove_dir_all(root);
    }

    fn sample_report() -> MultiAnchorReport {
        MultiAnchorReport {
            schema_version: 1,
            role: "a37_multi_anchor_admission_card".to_string(),
            status: "diagnostic_only".to_string(),
            mode: "diagnostic".to_string(),
            gate_passed: false,
            report_count: 2,
            lens_count: 2,
            passing_lens_count: 1,
            min_lenses: 2,
            min_marginal_bits: 0.05,
            max_redundancy: 0.6,
            family_span_pass: true,
            redundancy_bound_pass: true,
            no_collapse_pass: false,
            association_family_count: 2,
            association_families: Default::default(),
            min_best_marginal_bits: 0.01,
            max_best_marginal_bits: 0.07,
            weakest_lens: "weak-lens".to_string(),
            target_summaries: vec![target(0), target(1)],
            lenses: vec![lens("strong-lens", 1, 0.07), lens("weak-lens", 2, 0.01)],
            source_reports: vec!["db-a".to_string(), "db-b".to_string()],
        }
    }

    fn target(target_class: usize) -> TargetSummary {
        TargetSummary {
            target_class,
            domain: "unit".to_string(),
            report_path: format!("db-{target_class}"),
            status: "diagnostic_only".to_string(),
            no_collapse_pass: false,
            family_span_pass: true,
            redundancy_bound_pass: true,
            n_eff: 1.5,
            panel_bits: 0.2,
            max_marginal_bits: 0.07,
            keep_count: 1,
            park_count: 1,
        }
    }

    fn lens(name: &str, slot: u16, best_marginal_bits: f32) -> LensEvidence {
        LensEvidence {
            slot,
            name: name.to_string(),
            association_family: "unit_family".to_string(),
            passed: best_marginal_bits >= 0.05,
            best_target_class: 1,
            best_domain: "unit".to_string(),
            best_marginal_bits,
            best_solo_bits: 0.2,
            target_values: vec![TargetLensValue {
                target_class: 1,
                domain: "unit".to_string(),
                marginal_bits: best_marginal_bits,
                solo_bits: 0.2,
                decision: "keep".to_string(),
            }],
        }
    }
}
