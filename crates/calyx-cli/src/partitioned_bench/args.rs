use std::path::PathBuf;

use crate::error::{CliError, CliResult};

pub(super) struct SearchArgs {
    pub(super) vault: PathBuf,
    pub(super) queries: Option<PathBuf>,
    pub(super) corpus: Option<PathBuf>,
    pub(super) ground_truth_file: Option<PathBuf>,
    pub(super) ground_truth_id_map: Option<PathBuf>,
    pub(super) n: usize,
    pub(super) k: usize,
    pub(super) n_probe: usize,
    pub(super) region_beam: usize,
    pub(super) ground_truth: usize,
    pub(super) recall_floor: Option<f32>,
    pub(super) anneal_vault: Option<PathBuf>,
    pub(super) tuner_slo_us: Option<u64>,
}

impl SearchArgs {
    pub(super) fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut queries = None;
        let mut corpus = None;
        let mut ground_truth_file = None;
        let mut ground_truth_id_map = None;
        let (mut n, mut k, mut n_probe, mut region_beam) = (1000usize, 10usize, 8usize, 64usize);
        let mut ground_truth = 0usize;
        let mut recall_floor = None;
        let mut anneal_vault = None;
        let mut tuner_slo_us = None;
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--vault" => vault = Some(PathBuf::from(next()?)),
                "--queries" => queries = Some(PathBuf::from(next()?)),
                "--corpus" => corpus = Some(PathBuf::from(next()?)),
                "--ground-truth-file" => ground_truth_file = Some(PathBuf::from(next()?)),
                "--ground-truth-id-map" => ground_truth_id_map = Some(PathBuf::from(next()?)),
                "--n" => n = parse(&next()?, "--n")?,
                "--k" => k = parse(&next()?, "--k")?,
                "--n-probe" => n_probe = parse(&next()?, "--n-probe")?,
                "--region-beam" => region_beam = parse(&next()?, "--region-beam")?,
                "--ground-truth" => ground_truth = parse(&next()?, "--ground-truth")?,
                "--recall-floor" => recall_floor = Some(parse_recall_floor(&next()?)?),
                "--anneal-vault" => anneal_vault = Some(PathBuf::from(next()?)),
                "--tuner-slo-us" => {
                    let value = parse(&next()?, "--tuner-slo-us")?;
                    if value == 0 {
                        return Err(CliError::usage("--tuner-slo-us must be > 0"));
                    }
                    tuner_slo_us = Some(value);
                }
                // --seed and --report are accepted for harness symmetry; the query
                // seed is taken from the vault manifest (must match the build seed).
                "--seed" | "--report" => {
                    let _ = next()?;
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let vault = vault.ok_or_else(|| CliError::usage("--vault <dir> is required"))?;
        if n == 0 {
            return Err(CliError::usage("--n must be > 0"));
        }
        if k == 0 {
            return Err(CliError::usage("--k must be > 0"));
        }
        if n_probe == 0 {
            return Err(CliError::usage("--n-probe must be > 0"));
        }
        if region_beam == 0 {
            return Err(CliError::usage("--region-beam must be > 0"));
        }
        if ground_truth_id_map.is_some() && ground_truth_file.is_none() {
            return Err(CliError::usage(
                "--ground-truth-id-map requires --ground-truth-file",
            ));
        }
        Ok(Self {
            vault,
            queries,
            corpus,
            ground_truth_file,
            ground_truth_id_map,
            n,
            k,
            n_probe,
            region_beam,
            ground_truth,
            recall_floor,
            anneal_vault,
            tuner_slo_us,
        })
    }
}

pub(super) fn parse<T: std::str::FromStr>(v: &str, flag: &str) -> CliResult<T> {
    v.parse::<T>()
        .map_err(|_| CliError::usage(format!("{flag} expects a valid value, got {v}")))
}

pub(super) fn parse_recall_floor(v: &str) -> CliResult<f32> {
    let value: f32 = parse(v, "--recall-floor")?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::usage(
            "--recall-floor expects a finite value in [0, 1]",
        ));
    }
    Ok(value)
}
