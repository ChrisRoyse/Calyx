use std::path::PathBuf;

use crate::error::{CliError, CliResult};

#[derive(Clone, Debug)]
pub(super) struct Args {
    pub(super) plan: PathBuf,
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
    pub(super) slot_ground_truth_manifest: Option<PathBuf>,
    pub(super) ensemble_card: Option<PathBuf>,
    pub(super) a37_admission_card: Option<PathBuf>,
    pub(super) a37_admission_cf_root: Option<PathBuf>,
    pub(super) a37_admission_key: String,
    pub(super) write_fused_ground_truth_file: Option<PathBuf>,
    pub(super) write_fused_ground_truth_manifest: Option<PathBuf>,
    pub(super) out: Option<PathBuf>,
    pub(super) anneal_vault: Option<PathBuf>,
    pub(super) tuner_slo_us: Option<u64>,
}

impl Args {
    pub(super) fn parse(raw: &[String]) -> CliResult<Self> {
        let mut plan = None;
        let (mut n, mut k, mut n_probe, mut region_beam) = (1000, 10, 8, 64);
        let mut pruning_epsilon = None;
        let mut ground_truth = 0;
        let mut recall_floor = None;
        let mut truth_depth = None;
        let mut fused_ground_truth_file = None;
        let mut fused_ground_truth_manifest = None;
        let mut slot_ground_truth_manifest = None;
        let mut ensemble_card = None;
        let mut a37_admission_card = None;
        let mut a37_admission_cf_root = None;
        let mut a37_admission_key = "a37_multi_anchor_admission".to_string();
        let mut write_fused_ground_truth_file = None;
        let mut write_fused_ground_truth_manifest = None;
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
                "--slot-ground-truth-manifest" => {
                    slot_ground_truth_manifest = Some(PathBuf::from(next()?))
                }
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
        let plan = plan.ok_or_else(|| CliError::usage("--plan <json> is required"))?;
        if k == 0 {
            return Err(CliError::usage("--k must be > 0"));
        }
        validate_truth_args(
            fused_ground_truth_file.as_ref(),
            fused_ground_truth_manifest.as_ref(),
            slot_ground_truth_manifest.as_ref(),
            write_fused_ground_truth_file.as_ref(),
            write_fused_ground_truth_manifest.as_ref(),
            a37_admission_card.as_ref(),
            a37_admission_cf_root.as_ref(),
        )?;
        if a37_admission_key.trim().is_empty() {
            return Err(CliError::usage("--a37-admission-key must be non-empty"));
        }
        Ok(Self {
            plan,
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
            slot_ground_truth_manifest,
            ensemble_card,
            a37_admission_card,
            a37_admission_cf_root,
            a37_admission_key,
            write_fused_ground_truth_file,
            write_fused_ground_truth_manifest,
            out,
            anneal_vault,
            tuner_slo_us,
        })
    }
}

fn validate_truth_args(
    fused_file: Option<&PathBuf>,
    fused_manifest: Option<&PathBuf>,
    slot_manifest: Option<&PathBuf>,
    write_file: Option<&PathBuf>,
    write_manifest: Option<&PathBuf>,
    a37_card: Option<&PathBuf>,
    a37_cf_root: Option<&PathBuf>,
) -> CliResult {
    if fused_file.is_some() != fused_manifest.is_some() {
        return Err(CliError::usage(
            "--fused-ground-truth-file requires --fused-ground-truth-manifest",
        ));
    }
    if write_file.is_some() != write_manifest.is_some() {
        return Err(CliError::usage(
            "--write-fused-ground-truth-file requires --write-fused-ground-truth-manifest",
        ));
    }
    if fused_file.is_some() && write_file.is_some() {
        return Err(CliError::usage(
            "precomputed and generated fused ground truth are mutually exclusive in one run",
        ));
    }
    if fused_file.is_some() && slot_manifest.is_some() {
        return Err(CliError::usage(
            "--fused-ground-truth-file and --slot-ground-truth-manifest are mutually exclusive",
        ));
    }
    if a37_card.is_some() && a37_cf_root.is_some() {
        return Err(CliError::usage(
            "--a37-admission-card and --a37-admission-cf-root are mutually exclusive",
        ));
    }
    Ok(())
}

fn parse<T: std::str::FromStr>(value: &str, flag: &str) -> CliResult<T> {
    value
        .parse::<T>()
        .map_err(|_| CliError::usage(format!("{flag} expects a valid value, got {value}")))
}
