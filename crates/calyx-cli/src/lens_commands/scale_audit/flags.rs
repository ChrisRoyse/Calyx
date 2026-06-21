use std::path::PathBuf;

use super::model::{
    DEFAULT_BATCH_SIZE, DEFAULT_LENS_TIMEOUT_SECS, DEFAULT_MAX_ABS_DELTA, DEFAULT_MIN_BATCH_COSINE,
    DEFAULT_MIN_CONTENT_LENSES, DEFAULT_MIN_EFFECTIVE_BATCH, DEFAULT_MIN_GPU_CONTENT_LENSES, Flags,
};
use crate::error::{CliError, CliResult};

impl Flags {
    pub(super) fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self {
            manifests: Vec::new(),
            out: PathBuf::new(),
            batch_size: DEFAULT_BATCH_SIZE,
            min_content_lenses: DEFAULT_MIN_CONTENT_LENSES,
            min_gpu_content_lenses: DEFAULT_MIN_GPU_CONTENT_LENSES,
            min_effective_batch: DEFAULT_MIN_EFFECTIVE_BATCH,
            min_batch_cosine: DEFAULT_MIN_BATCH_COSINE,
            max_abs_delta: DEFAULT_MAX_ABS_DELTA,
            lens_timeout_secs: DEFAULT_LENS_TIMEOUT_SECS,
            probes: Vec::new(),
            probe_files: Vec::new(),
            worker: false,
        };
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--manifest" => {
                    idx += 1;
                    flags.manifests.push(value(args, idx, "--manifest")?.into());
                }
                "--out" => {
                    idx += 1;
                    flags.out = value(args, idx, "--out")?.into();
                }
                "--batch-size" => {
                    idx += 1;
                    flags.batch_size =
                        parse_usize(value(args, idx, "--batch-size")?, "--batch-size")?;
                }
                "--min-content-lenses" => {
                    idx += 1;
                    flags.min_content_lenses = parse_usize(
                        value(args, idx, "--min-content-lenses")?,
                        "--min-content-lenses",
                    )?;
                }
                "--min-gpu-content-lenses" => {
                    idx += 1;
                    flags.min_gpu_content_lenses = parse_usize(
                        value(args, idx, "--min-gpu-content-lenses")?,
                        "--min-gpu-content-lenses",
                    )?;
                }
                "--min-effective-batch" => {
                    idx += 1;
                    flags.min_effective_batch = parse_usize(
                        value(args, idx, "--min-effective-batch")?,
                        "--min-effective-batch",
                    )?;
                }
                "--min-batch-cosine" => {
                    idx += 1;
                    flags.min_batch_cosine = parse_f32(
                        value(args, idx, "--min-batch-cosine")?,
                        "--min-batch-cosine",
                    )?;
                }
                "--max-abs-delta" => {
                    idx += 1;
                    flags.max_abs_delta =
                        parse_f32(value(args, idx, "--max-abs-delta")?, "--max-abs-delta")?;
                }
                "--lens-timeout-secs" => {
                    idx += 1;
                    flags.lens_timeout_secs = parse_u64(
                        value(args, idx, "--lens-timeout-secs")?,
                        "--lens-timeout-secs",
                    )?;
                }
                "--probe" => {
                    idx += 1;
                    flags.probes.push(value(args, idx, "--probe")?.to_string());
                }
                "--probe-file" => {
                    idx += 1;
                    flags
                        .probe_files
                        .push(value(args, idx, "--probe-file")?.into());
                }
                "--worker" => {
                    flags.worker = true;
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected lens scale-audit flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        validate_flags(flags)
    }
}

fn validate_flags(flags: Flags) -> CliResult<Flags> {
    if flags.manifests.is_empty() {
        return Err(CliError::usage(
            "calyx lens scale-audit requires --manifest <path>",
        ));
    }
    if flags.out.as_os_str().is_empty() {
        return Err(CliError::usage(
            "calyx lens scale-audit requires --out <report.json>",
        ));
    }
    if flags.batch_size == 0 || flags.min_effective_batch == 0 {
        return Err(CliError::usage("batch sizes must be > 0"));
    }
    if flags.lens_timeout_secs == 0 {
        return Err(CliError::usage("--lens-timeout-secs must be > 0"));
    }
    if !(0.0..=1.0).contains(&flags.min_batch_cosine) {
        return Err(CliError::usage("--min-batch-cosine must be within 0..1"));
    }
    if !flags.max_abs_delta.is_finite() || flags.max_abs_delta < 0.0 {
        return Err(CliError::usage("--max-abs-delta must be finite and >= 0"));
    }
    Ok(flags)
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn parse_usize(raw: &str, flag: &str) -> CliResult<usize> {
    raw.parse::<usize>()
        .map_err(|error| CliError::usage(format!("{flag} must be an integer: {error}")))
}

fn parse_u64(raw: &str, flag: &str) -> CliResult<u64> {
    raw.parse::<u64>()
        .map_err(|error| CliError::usage(format!("{flag} must be an integer: {error}")))
}

fn parse_f32(raw: &str, flag: &str) -> CliResult<f32> {
    raw.parse::<f32>()
        .map_err(|error| CliError::usage(format!("{flag} must be a float: {error}")))
}
