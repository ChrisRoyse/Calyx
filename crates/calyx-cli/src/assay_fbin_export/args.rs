use std::path::PathBuf;

use crate::error::{CliError, CliResult};

use super::DEFAULT_MIN_BITS;

#[derive(Clone, Debug)]
pub(super) struct Args {
    pub(super) corpus_dir: PathBuf,
    pub(super) out_dir: PathBuf,
    pub(super) bits_report: PathBuf,
    pub(super) query_count: usize,
    pub(super) min_bits: f32,
}

impl Args {
    pub(super) fn parse(raw: &[String]) -> CliResult<Self> {
        let mut corpus_dir = None;
        let mut out_dir = None;
        let mut bits_report = None;
        let mut query_count = None;
        let mut min_bits = DEFAULT_MIN_BITS;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--corpus-dir" => corpus_dir = Some(PathBuf::from(next()?)),
                "--out-dir" => out_dir = Some(PathBuf::from(next()?)),
                "--bits-report" => bits_report = Some(PathBuf::from(next()?)),
                "--query-count" => query_count = Some(parse_usize(&next()?, "--query-count")?),
                "--min-bits" => min_bits = parse_f32(&next()?, "--min-bits")?,
                other => {
                    return Err(CliError::usage(format!(
                        "unknown assay export-fbin arg: {other}"
                    )));
                }
            }
        }
        let args = Self {
            corpus_dir: corpus_dir
                .ok_or_else(|| CliError::usage("--corpus-dir <dir> is required"))?,
            out_dir: out_dir.ok_or_else(|| CliError::usage("--out-dir <dir> is required"))?,
            bits_report: bits_report
                .ok_or_else(|| CliError::usage("--bits-report <json> is required"))?,
            query_count: query_count
                .ok_or_else(|| CliError::usage("--query-count <n> is required"))?,
            min_bits,
        };
        validate(&args)?;
        Ok(args)
    }
}

fn validate(args: &Args) -> CliResult {
    if args.query_count == 0 {
        return Err(CliError::usage("--query-count must be > 0"));
    }
    if !args.min_bits.is_finite() || args.min_bits < 0.0 {
        return Err(CliError::usage(
            "--min-bits must be finite and non-negative",
        ));
    }
    Ok(())
}

fn parse_usize(value: &str, flag: &str) -> CliResult<usize> {
    value
        .parse()
        .map_err(|error| CliError::usage(format!("{flag} expects usize: {error}")))
}

fn parse_f32(value: &str, flag: &str) -> CliResult<f32> {
    value
        .parse()
        .map_err(|error| CliError::usage(format!("{flag} expects f32: {error}")))
}
