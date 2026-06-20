use std::path::PathBuf;

use crate::assay_corpus_build::request::CorpusBuildRequest;
use crate::error::{CliError, CliResult};

use super::DEFAULT_MIN_BITS;
use super::format::VectorFormat;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StreamMode {
    Gate,
    Diagnostic,
}

impl StreamMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Gate => "gate",
            Self::Diagnostic => "diagnostic",
        }
    }

    pub(crate) fn requires_gate(self) -> bool {
        matches!(self, Self::Gate)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Args {
    pub(crate) rows_jsonl: PathBuf,
    pub(crate) out_dir: PathBuf,
    pub(crate) dataset: String,
    pub(crate) target_class: usize,
    pub(crate) manifests: Vec<PathBuf>,
    pub(crate) bits_report: PathBuf,
    pub(crate) query_count: usize,
    pub(crate) limit_per_class: Option<usize>,
    pub(crate) batch_size: usize,
    pub(crate) cost_override_json: Option<PathBuf>,
    pub(crate) embedding_model_id: Option<String>,
    pub(crate) min_bits: f32,
    pub(crate) vector_format: VectorFormat,
    pub(crate) mode: StreamMode,
    pub(crate) worker_report: Option<PathBuf>,
    pub(crate) worker_slot: Option<usize>,
}

impl Args {
    pub(crate) fn parse(raw: &[String]) -> CliResult<Self> {
        let mut rows_jsonl = None;
        let mut out_dir = None;
        let mut dataset = None;
        let mut target_class = None;
        let mut manifests = Vec::new();
        let mut bits_report = None;
        let mut query_count = None;
        let mut limit_per_class = None;
        let mut batch_size = 16usize;
        let mut cost_override_json = None;
        let mut embedding_model_id = None;
        let mut min_bits = DEFAULT_MIN_BITS;
        let mut vector_format = VectorFormat::default();
        let mut mode = StreamMode::Gate;
        let mut worker_report = None;
        let mut worker_slot = None;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--rows-jsonl" => rows_jsonl = Some(PathBuf::from(next()?)),
                "--out-dir" => out_dir = Some(PathBuf::from(next()?)),
                "--dataset" => dataset = Some(next()?),
                "--target-class" => target_class = Some(parse_usize(&next()?, flag)?),
                "--manifest" => manifests.push(PathBuf::from(next()?)),
                "--bits-report" => bits_report = Some(PathBuf::from(next()?)),
                "--query-count" => query_count = Some(parse_usize(&next()?, flag)?),
                "--limit-per-class" => limit_per_class = Some(parse_usize(&next()?, flag)?),
                "--batch-size" => batch_size = parse_usize(&next()?, flag)?,
                "--cost-override-json" => cost_override_json = Some(PathBuf::from(next()?)),
                "--embedding-model-id" => embedding_model_id = Some(next()?),
                "--min-bits" => min_bits = parse_f32(&next()?, flag)?,
                "--vector-format" => vector_format = VectorFormat::parse(&next()?)?,
                "--mode" => mode = parse_mode(&next()?)?,
                "--diagnostic" | "--baseline" => mode = StreamMode::Diagnostic,
                "--worker-report" => worker_report = Some(PathBuf::from(next()?)),
                "--worker-slot" => worker_slot = Some(parse_usize(&next()?, flag)?),
                other => {
                    return Err(CliError::usage(format!(
                        "unknown assay stream-fbin arg: {other}"
                    )));
                }
            }
        }
        let args = Self {
            rows_jsonl: rows_jsonl
                .ok_or_else(|| CliError::usage("--rows-jsonl <jsonl> is required"))?,
            out_dir: out_dir.ok_or_else(|| CliError::usage("--out-dir <dir> is required"))?,
            dataset: dataset.ok_or_else(|| CliError::usage("--dataset <name> is required"))?,
            target_class: target_class
                .ok_or_else(|| CliError::usage("--target-class <n> is required"))?,
            manifests,
            bits_report: bits_report
                .ok_or_else(|| CliError::usage("--bits-report <json> is required"))?,
            query_count: query_count
                .ok_or_else(|| CliError::usage("--query-count <n> is required"))?,
            limit_per_class,
            batch_size,
            cost_override_json,
            embedding_model_id,
            min_bits,
            vector_format,
            mode,
            worker_report,
            worker_slot,
        };
        args.validate()?;
        Ok(args)
    }

    pub(crate) fn corpus_request(&self) -> CorpusBuildRequest {
        CorpusBuildRequest {
            rows_jsonl: self.rows_jsonl.clone(),
            out_dir: self.out_dir.clone(),
            dataset: self.dataset.clone(),
            target_class: self.target_class,
            manifests: self.manifests.clone(),
            limit_per_class: self.limit_per_class,
            batch_size: self.batch_size,
            cost_override_json: self.cost_override_json.clone(),
            embedding_model_id: self.embedding_model_id.clone(),
            worker_report: None,
        }
    }

    fn validate(&self) -> CliResult {
        if self.dataset.trim().is_empty() {
            return Err(CliError::usage("--dataset must be non-empty"));
        }
        let worker_mode = self.worker_report.is_some() || self.worker_slot.is_some();
        if worker_mode
            && (self.worker_report.is_none()
                || self.worker_slot.is_none()
                || self.manifests.len() != 1)
        {
            return Err(CliError::usage(
                "stream-fbin worker mode requires --worker-report, --worker-slot, and exactly one --manifest",
            ));
        }
        if !worker_mode && self.manifests.len() < 2 {
            return Err(CliError::usage("provide at least two --manifest entries"));
        }
        if self.query_count == 0 || self.batch_size == 0 {
            return Err(CliError::usage(
                "--query-count and --batch-size must be > 0",
            ));
        }
        if matches!(self.limit_per_class, Some(0)) {
            return Err(CliError::usage("--limit-per-class must be > 0"));
        }
        if !self.min_bits.is_finite() || self.min_bits < 0.0 {
            return Err(CliError::usage(
                "--min-bits must be finite and non-negative",
            ));
        }
        Ok(())
    }
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

fn parse_mode(value: &str) -> CliResult<StreamMode> {
    match value {
        "gate" => Ok(StreamMode::Gate),
        "diagnostic" | "baseline" => Ok(StreamMode::Diagnostic),
        other => Err(CliError::usage(format!(
            "--mode must be gate or diagnostic, got {other}"
        ))),
    }
}
