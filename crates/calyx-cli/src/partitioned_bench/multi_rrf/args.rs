use std::path::PathBuf;

use crate::error::{CliError, CliResult};

#[derive(Clone, Debug)]
pub(super) struct Args {
    pub(super) plan: Option<PathBuf>,
    pub(super) plan_cf_root: Option<PathBuf>,
    pub(super) plan_key: String,
    pub(super) timeline_cf_root: Option<PathBuf>,
    pub(super) timeline_key: String,
    pub(super) n: usize,
    pub(super) k: usize,
    pub(super) n_probe: usize,
    pub(super) region_beam: usize,
    pub(super) pruning_epsilon: Option<f32>,
    pub(super) ground_truth: usize,
    pub(super) recall_floor: Option<f32>,
    pub(super) truth_depth: Option<usize>,
    pub(super) fused_ground_truth_file: Option<PathBuf>,
    pub(super) fused_ground_truth_manifest: Option<PathBuf>,
    pub(super) fused_ground_truth_cf_root: Option<PathBuf>,
    pub(super) fused_ground_truth_key: String,
    pub(super) slot_ground_truth_manifest: Option<PathBuf>,
    pub(super) slot_ground_truth_cf_root: Option<PathBuf>,
    pub(super) slot_ground_truth_key: String,
    pub(super) ensemble_card: Option<PathBuf>,
    pub(super) a37_admission_card: Option<PathBuf>,
    pub(super) a37_admission_cf_root: Option<PathBuf>,
    pub(super) a37_admission_key: String,
    pub(super) write_fused_ground_truth_file: Option<PathBuf>,
    pub(super) write_fused_ground_truth_manifest: Option<PathBuf>,
    pub(super) write_fused_ground_truth_cf_root: Option<PathBuf>,
    pub(super) write_fused_ground_truth_key: String,
    pub(super) report_cf_root: Option<PathBuf>,
    pub(super) report_key: String,
    pub(super) report_db_only: bool,
    pub(super) out: Option<PathBuf>,
    pub(super) anneal_vault: Option<PathBuf>,
    pub(super) tuner_slo_us: Option<u64>,
}

impl Args {
    pub(super) fn parse(raw: &[String]) -> CliResult<Self> {
        let mut plan = None;
        let mut plan_cf_root = None;
        let mut plan_key = crate::partitioned_bench::rrf_plan::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut timeline_cf_root = None;
        let mut timeline_key =
            crate::partitioned_bench::timeline_store::DEFAULT_ASSOCIATION_KEY.to_string();
        let (mut n, mut k, mut n_probe, mut region_beam) = (1000, 10, 8, 64);
        let mut pruning_epsilon = None;
        let mut ground_truth = 0;
        let mut recall_floor = None;
        let mut truth_depth = None;
        let mut fused_ground_truth_file = None;
        let mut fused_ground_truth_manifest = None;
        let mut fused_ground_truth_cf_root = None;
        let mut fused_ground_truth_key = super::fused_truth_db::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut slot_ground_truth_manifest = None;
        let mut slot_ground_truth_cf_root = None;
        let mut slot_ground_truth_key =
            crate::partitioned_bench::slot_truth_store::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut ensemble_card = None;
        let mut a37_admission_card = None;
        let mut a37_admission_cf_root = None;
        let mut a37_admission_key = "a37_multi_anchor_admission".to_string();
        let mut write_fused_ground_truth_file = None;
        let mut write_fused_ground_truth_manifest = None;
        let mut write_fused_ground_truth_cf_root = None;
        let mut write_fused_ground_truth_key =
            super::fused_truth_db::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut report_cf_root = None;
        let mut report_key =
            crate::partitioned_rrf_report_store::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut report_db_only = false;
        let mut out = None;
        let mut anneal_vault = None;
        let mut tuner_slo_us = None;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--plan" => plan = Some(PathBuf::from(next()?)),
                "--plan-cf-root" => plan_cf_root = Some(PathBuf::from(next()?)),
                "--plan-key" => plan_key = next()?,
                "--timeline-cf-root" => timeline_cf_root = Some(PathBuf::from(next()?)),
                "--timeline-key" => timeline_key = next()?,
                "--n" => n = parse(&next()?, "--n")?,
                "--k" => k = parse(&next()?, "--k")?,
                "--n-probe" => n_probe = parse(&next()?, "--n-probe")?,
                "--region-beam" => region_beam = parse(&next()?, "--region-beam")?,
                "--pruning-epsilon" => {
                    pruning_epsilon = Some(super::super::parse_pruning_epsilon(&next()?)?)
                }
                "--ground-truth" => ground_truth = parse(&next()?, "--ground-truth")?,
                "--recall-floor" => {
                    recall_floor = Some(super::super::parse_recall_floor(&next()?)?)
                }
                "--truth-depth" => truth_depth = Some(parse(&next()?, "--truth-depth")?),
                "--fused-ground-truth-file" => {
                    fused_ground_truth_file = Some(PathBuf::from(next()?))
                }
                "--fused-ground-truth-manifest" => {
                    fused_ground_truth_manifest = Some(PathBuf::from(next()?))
                }
                "--fused-ground-truth-cf-root" => {
                    fused_ground_truth_cf_root = Some(PathBuf::from(next()?))
                }
                "--fused-ground-truth-key" => fused_ground_truth_key = next()?,
                "--slot-ground-truth-manifest" => {
                    slot_ground_truth_manifest = Some(PathBuf::from(next()?))
                }
                "--slot-ground-truth-cf-root" => {
                    slot_ground_truth_cf_root = Some(PathBuf::from(next()?))
                }
                "--slot-ground-truth-key" => slot_ground_truth_key = next()?,
                "--ensemble-card" => ensemble_card = Some(PathBuf::from(next()?)),
                "--a37-admission-card" => a37_admission_card = Some(PathBuf::from(next()?)),
                "--a37-admission-cf-root" => a37_admission_cf_root = Some(PathBuf::from(next()?)),
                "--a37-admission-key" => a37_admission_key = next()?,
                "--write-fused-ground-truth-file" => {
                    write_fused_ground_truth_file = Some(PathBuf::from(next()?))
                }
                "--write-fused-ground-truth-manifest" => {
                    write_fused_ground_truth_manifest = Some(PathBuf::from(next()?))
                }
                "--write-fused-ground-truth-cf-root" => {
                    write_fused_ground_truth_cf_root = Some(PathBuf::from(next()?))
                }
                "--write-fused-ground-truth-key" => write_fused_ground_truth_key = next()?,
                "--report-cf-root" => report_cf_root = Some(PathBuf::from(next()?)),
                "--report-key" => report_key = next()?,
                "--report-db-only" | "--no-report-stdout" => report_db_only = true,
                "--out" => out = Some(PathBuf::from(next()?)),
                "--anneal-vault" => anneal_vault = Some(PathBuf::from(next()?)),
                "--tuner-slo-us" => {
                    let value = parse(&next()?, "--tuner-slo-us")?;
                    if value == 0 {
                        return Err(CliError::usage("--tuner-slo-us must be > 0"));
                    }
                    tuner_slo_us = Some(value);
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        if plan.is_some() == plan_cf_root.is_some() {
            return Err(CliError::usage(
                "pass exactly one of --plan <json> or --plan-cf-root <aster-dir>",
            ));
        }
        if recall_floor.is_some() {
            validate_recall_gate_args(RecallGateArgRefs {
                plan_cf_root: plan_cf_root.as_ref(),
                timeline_cf_root: timeline_cf_root.as_ref(),
                fused_file: fused_ground_truth_file.as_ref(),
                fused_manifest: fused_ground_truth_manifest.as_ref(),
                fused_cf_root: fused_ground_truth_cf_root.as_ref(),
                slot_manifest: slot_ground_truth_manifest.as_ref(),
                slot_cf_root: slot_ground_truth_cf_root.as_ref(),
                ensemble_card: ensemble_card.as_ref(),
                a37_card: a37_admission_card.as_ref(),
                a37_cf_root: a37_admission_cf_root.as_ref(),
                write_file: write_fused_ground_truth_file.as_ref(),
                write_manifest: write_fused_ground_truth_manifest.as_ref(),
            })?;
        }
        if plan_key.trim().is_empty() {
            return Err(CliError::usage("--plan-key must be non-empty"));
        }
        if timeline_key.trim().is_empty() {
            return Err(CliError::usage("--timeline-key must be non-empty"));
        }
        if k == 0 {
            return Err(CliError::usage("--k must be > 0"));
        }
        validate_truth_args(TruthArgRefs {
            fused_file: fused_ground_truth_file.as_ref(),
            fused_manifest: fused_ground_truth_manifest.as_ref(),
            fused_cf_root: fused_ground_truth_cf_root.as_ref(),
            slot_manifest: slot_ground_truth_manifest.as_ref(),
            slot_cf_root: slot_ground_truth_cf_root.as_ref(),
            write_file: write_fused_ground_truth_file.as_ref(),
            write_manifest: write_fused_ground_truth_manifest.as_ref(),
            write_cf_root: write_fused_ground_truth_cf_root.as_ref(),
            a37_card: a37_admission_card.as_ref(),
            a37_cf_root: a37_admission_cf_root.as_ref(),
            report_cf_root: report_cf_root.as_ref(),
            report_db_only,
            out: out.as_ref(),
        })?;
        if slot_ground_truth_key.trim().is_empty() {
            return Err(CliError::usage("--slot-ground-truth-key must be non-empty"));
        }
        if fused_ground_truth_key.trim().is_empty() {
            return Err(CliError::usage(
                "--fused-ground-truth-key must be non-empty",
            ));
        }
        if write_fused_ground_truth_key.trim().is_empty() {
            return Err(CliError::usage(
                "--write-fused-ground-truth-key must be non-empty",
            ));
        }
        if a37_admission_key.trim().is_empty() {
            return Err(CliError::usage("--a37-admission-key must be non-empty"));
        }
        if report_key.trim().is_empty() {
            return Err(CliError::usage("--report-key must be non-empty"));
        }
        Ok(Self {
            plan,
            plan_cf_root,
            plan_key,
            timeline_cf_root,
            timeline_key,
            n,
            k,
            n_probe,
            region_beam,
            pruning_epsilon,
            ground_truth,
            recall_floor,
            truth_depth,
            fused_ground_truth_file,
            fused_ground_truth_manifest,
            fused_ground_truth_cf_root,
            fused_ground_truth_key,
            slot_ground_truth_manifest,
            slot_ground_truth_cf_root,
            slot_ground_truth_key,
            ensemble_card,
            a37_admission_card,
            a37_admission_cf_root,
            a37_admission_key,
            write_fused_ground_truth_file,
            write_fused_ground_truth_manifest,
            write_fused_ground_truth_cf_root,
            write_fused_ground_truth_key,
            report_cf_root,
            report_key,
            report_db_only,
            out,
            anneal_vault,
            tuner_slo_us,
        })
    }
}

struct TruthArgRefs<'a> {
    fused_file: Option<&'a PathBuf>,
    fused_manifest: Option<&'a PathBuf>,
    fused_cf_root: Option<&'a PathBuf>,
    slot_manifest: Option<&'a PathBuf>,
    slot_cf_root: Option<&'a PathBuf>,
    write_file: Option<&'a PathBuf>,
    write_manifest: Option<&'a PathBuf>,
    write_cf_root: Option<&'a PathBuf>,
    a37_card: Option<&'a PathBuf>,
    a37_cf_root: Option<&'a PathBuf>,
    report_cf_root: Option<&'a PathBuf>,
    report_db_only: bool,
    out: Option<&'a PathBuf>,
}

struct RecallGateArgRefs<'a> {
    plan_cf_root: Option<&'a PathBuf>,
    timeline_cf_root: Option<&'a PathBuf>,
    fused_file: Option<&'a PathBuf>,
    fused_manifest: Option<&'a PathBuf>,
    fused_cf_root: Option<&'a PathBuf>,
    slot_manifest: Option<&'a PathBuf>,
    slot_cf_root: Option<&'a PathBuf>,
    ensemble_card: Option<&'a PathBuf>,
    a37_card: Option<&'a PathBuf>,
    a37_cf_root: Option<&'a PathBuf>,
    write_file: Option<&'a PathBuf>,
    write_manifest: Option<&'a PathBuf>,
}

fn validate_recall_gate_args(args: RecallGateArgRefs<'_>) -> CliResult {
    if args.plan_cf_root.is_none() {
        return Err(CliError::usage(
            "--recall-floor requires --plan-cf-root so gate-bearing RRF recall uses DB plan authority",
        ));
    }
    if args.timeline_cf_root.is_none() {
        return Err(CliError::usage(
            "--recall-floor requires --timeline-cf-root so gate-bearing RRF recall uses DB timeline authority",
        ));
    }
    if args.a37_cf_root.is_none() {
        return Err(CliError::usage(
            "--recall-floor requires --a37-admission-cf-root so gate-bearing RRF recall uses DB admission authority",
        ));
    }
    if args.a37_card.is_some() || args.ensemble_card.is_some() {
        return Err(CliError::usage(
            "--recall-floor cannot use JSON A37 admission or ensemble cards as gate authority",
        ));
    }
    if args.fused_file.is_some() || args.fused_manifest.is_some() || args.slot_manifest.is_some() {
        return Err(CliError::usage(
            "--recall-floor cannot use file or manifest truth; pass DB fused or slot truth",
        ));
    }
    if args.write_file.is_some() || args.write_manifest.is_some() {
        return Err(CliError::usage(
            "--recall-floor cannot write file truth; use --write-fused-ground-truth-cf-root for DB output",
        ));
    }
    if args.fused_cf_root.is_none() && args.slot_cf_root.is_none() {
        return Err(CliError::usage(
            "--recall-floor requires --fused-ground-truth-cf-root or --slot-ground-truth-cf-root",
        ));
    }
    Ok(())
}

fn validate_truth_args(args: TruthArgRefs<'_>) -> CliResult {
    if args.fused_file.is_some() != args.fused_manifest.is_some() {
        return Err(CliError::usage(
            "--fused-ground-truth-file requires --fused-ground-truth-manifest",
        ));
    }
    if args.write_file.is_some() != args.write_manifest.is_some() {
        return Err(CliError::usage(
            "--write-fused-ground-truth-file requires --write-fused-ground-truth-manifest",
        ));
    }
    if args.fused_cf_root.is_some() && args.fused_file.is_some() {
        return Err(CliError::usage(
            "file and DB fused ground truth sources are mutually exclusive",
        ));
    }
    let write_outputs =
        usize::from(args.write_file.is_some()) + usize::from(args.write_cf_root.is_some());
    if write_outputs > 1 {
        return Err(CliError::usage(
            "file and DB fused ground-truth writes are mutually exclusive",
        ));
    }
    if (args.fused_file.is_some() || args.fused_cf_root.is_some()) && write_outputs > 0 {
        return Err(CliError::usage(
            "precomputed and generated fused ground truth are mutually exclusive in one run",
        ));
    }
    let truth_sources = usize::from(args.fused_file.is_some())
        + usize::from(args.fused_cf_root.is_some())
        + usize::from(args.slot_manifest.is_some())
        + usize::from(args.slot_cf_root.is_some());
    if truth_sources > 1 {
        return Err(CliError::usage(
            "precomputed fused file, fused DB, slot manifest, and slot DB ground truth are mutually exclusive",
        ));
    }
    if args.a37_card.is_some() && args.a37_cf_root.is_some() {
        return Err(CliError::usage(
            "--a37-admission-card and --a37-admission-cf-root are mutually exclusive",
        ));
    }
    if args.report_db_only && args.report_cf_root.is_none() {
        return Err(CliError::usage(
            "--report-db-only requires --report-cf-root",
        ));
    }
    if args.report_db_only && args.out.is_some() {
        return Err(CliError::usage(
            "--report-db-only and --out are mutually exclusive",
        ));
    }
    Ok(())
}

fn parse<T: std::str::FromStr>(value: &str, flag: &str) -> CliResult<T> {
    value
        .parse::<T>()
        .map_err(|_| CliError::usage(format!("{flag} expects a valid value, got {value}")))
}
