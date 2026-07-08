use std::path::PathBuf;

use calyx_registry::LensSpec;

use crate::error::{CliError, CliResult};

use super::DEFAULT_MIN_BITS;
use super::format::VectorFormat;

const A37_ADMISSION_KEY_FLAG: &str = concat!("--a37-admission-", "key");

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
    pub(crate) lens_template_cf_root: Option<PathBuf>,
    pub(crate) lens_template_key: String,
    pub(crate) lens_template_specs: Vec<LensSpec>,
    pub(crate) bits_report: Option<PathBuf>,
    pub(crate) a37_admission_cf_root: Option<PathBuf>,
    pub(crate) a37_admission_key: String,
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
    pub(crate) lens_parallelism: usize,
    pub(crate) worker_gpu_mem_limit_mib: Option<usize>,
    pub(crate) emit_artifacts: bool,
}

impl Args {
    pub(crate) fn parse(raw: &[String]) -> CliResult<Self> {
        let mut rows_jsonl = None;
        let mut out_dir = None;
        let mut dataset = None;
        let mut target_class = None;
        let mut manifests = Vec::new();
        let mut lens_template_cf_root = None;
        let mut lens_template_key = super::template::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut bits_report = None;
        let mut a37_admission_cf_root = None;
        let mut a37_admission_key = "a37_multi_anchor_admission".to_string();
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
        let mut lens_parallelism = 1usize;
        let mut worker_gpu_mem_limit_mib = None;
        let mut emit_artifacts = true;
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
                "--lens-template-cf-root" => lens_template_cf_root = Some(PathBuf::from(next()?)),
                "--lens-template-key" => lens_template_key = next()?,
                "--bits-report" => bits_report = Some(PathBuf::from(next()?)),
                "--a37-admission-cf-root" => {
                    a37_admission_cf_root = Some(PathBuf::from(next()?));
                }
                flag if flag == A37_ADMISSION_KEY_FLAG => {
                    let value = next()?;
                    a37_admission_key = value;
                }
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
                "--lens-parallelism" => lens_parallelism = parse_usize(&next()?, flag)?,
                "--worker-gpu-mem-limit-mib" => {
                    worker_gpu_mem_limit_mib = Some(parse_usize(&next()?, flag)?);
                }
                "--db-only" | "--no-artifacts" => emit_artifacts = false,
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
            lens_template_cf_root,
            lens_template_key,
            lens_template_specs: Vec::new(),
            bits_report,
            a37_admission_cf_root,
            a37_admission_key,
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
            lens_parallelism,
            worker_gpu_mem_limit_mib,
            emit_artifacts,
        };
        args.validate()?;
        Ok(args)
    }

    fn validate(&self) -> CliResult {
        if self.dataset.trim().is_empty() {
            return Err(CliError::usage("--dataset must be non-empty"));
        }
        let worker_mode = self.worker_report.is_some() || self.worker_slot.is_some();
        let has_lens_template = self.lens_template_cf_root.is_some();
        if worker_mode
            && (self.worker_report.is_none()
                || self.worker_slot.is_none()
                || self.manifests.len() != 1)
        {
            return Err(CliError::usage(
                "stream-fbin worker mode requires --worker-report, --worker-slot, and exactly one --manifest",
            ));
        }
        if worker_mode && has_lens_template {
            return Err(CliError::usage(
                "stream-fbin worker mode is file-diagnostic only; DB-native templates run in-process",
            ));
        }
        if !worker_mode && has_lens_template && !self.manifests.is_empty() {
            return Err(CliError::usage(
                "provide either --lens-template-cf-root or --manifest entries, not both",
            ));
        }
        if !worker_mode && self.mode.requires_gate() && !has_lens_template {
            return Err(CliError::usage(
                "gate mode requires --lens-template-cf-root <dir>; --manifest is diagnostic/import-only",
            ));
        }
        if !worker_mode && !has_lens_template && self.manifests.len() < 2 {
            return Err(CliError::usage("provide at least two --manifest entries"));
        }
        if self.lens_template_key.trim().is_empty() {
            return Err(CliError::usage("--lens-template-key must be non-empty"));
        }
        let has_bits_report = self.bits_report.is_some();
        let has_admission_db = self.a37_admission_cf_root.is_some();
        if has_bits_report && has_admission_db {
            return Err(CliError::usage(
                "provide at most one of --bits-report <json> or --a37-admission-cf-root <dir>",
            ));
        }
        if self.mode.requires_gate() && !has_admission_db {
            return Err(CliError::usage(
                "gate mode requires --a37-admission-cf-root <dir>",
            ));
        }
        if has_bits_report && self.mode.requires_gate() {
            return Err(CliError::usage(
                "--bits-report is diagnostic-only; gate mode requires --a37-admission-cf-root <dir>",
            ));
        }
        if self.a37_admission_key.trim().is_empty() {
            return Err(CliError::usage("--a37-admission-key must be non-empty"));
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
        if self.lens_parallelism == 0 {
            return Err(CliError::usage("--lens-parallelism must be > 0"));
        }
        if worker_mode && self.lens_parallelism != 1 {
            return Err(CliError::usage(
                "--lens-parallelism applies to the parent stream-fbin command, not workers",
            ));
        }
        if matches!(self.worker_gpu_mem_limit_mib, Some(0)) {
            return Err(CliError::usage("--worker-gpu-mem-limit-mib must be > 0"));
        }
        Ok(())
    }

    pub(crate) fn diagnostic_bootstrap_without_admission(&self) -> bool {
        !self.mode.requires_gate()
            && self.bits_report.is_none()
            && self.a37_admission_cf_root.is_none()
    }

    pub(crate) fn lens_descriptor_ref(&self, lens_name: &str) -> String {
        match &self.lens_template_cf_root {
            Some(root) => format!(
                "aster-graph-cf:{}:{}:{}",
                root.display(),
                self.lens_template_key,
                lens_name
            ),
            None => format!("manifest:{lens_name}"),
        }
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn parses_worker_passthrough_flags() {
        let raw = [
            "--rows-jsonl",
            "rows.jsonl",
            "--out-dir",
            "out",
            "--dataset",
            "unit",
            "--target-class",
            "1",
            "--lens-template-cf-root",
            "template_cf",
            "--a37-admission-cf-root",
            "a37_cf",
            "--query-count",
            "8",
            "--cost-override-json",
            "costs.json",
            "--embedding-model-id",
            "intfloat/multilingual-e5-base",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();

        let args = Args::parse(&raw).unwrap();

        assert_eq!(
            args.cost_override_json.as_deref(),
            Some(Path::new("costs.json"))
        );
        assert_eq!(
            args.embedding_model_id.as_deref(),
            Some("intfloat/multilingual-e5-base")
        );
    }
}
