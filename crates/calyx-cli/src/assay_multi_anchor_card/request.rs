use std::path::PathBuf;

use calyx_assay::{DEFAULT_MAX_REDUNDANCY, DEFAULT_MIN_MARGINAL_BITS};

use super::model::DbReportRef;
use super::{CODE_INVALID_CONFIG, CODE_OUTPUT_EXISTS};

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) reports: Vec<PathBuf>,
    pub(crate) db_reports: Vec<DbReportRef>,
    pub(crate) out_dir: PathBuf,
    pub(crate) cf_root: PathBuf,
    pub(crate) association_key: String,
    pub(crate) min_lenses: usize,
    pub(crate) min_marginal_bits: f32,
    pub(crate) max_redundancy: f32,
    pub(crate) mode: Mode,
    pub(crate) emit_artifacts: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Gate,
    Diagnostic,
}

impl Mode {
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

impl Request {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut reports = Vec::new();
        let mut db_reports = Vec::new();
        let mut out_dir = PathBuf::new();
        let mut cf_root = None;
        let mut association_key = "a37_multi_anchor_admission".to_string();
        let mut min_lenses = 10_usize;
        let mut min_marginal_bits = DEFAULT_MIN_MARGINAL_BITS;
        let mut max_redundancy = DEFAULT_MAX_REDUNDANCY;
        let mut mode = Mode::Gate;
        let mut emit_artifacts = true;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--report" => {
                    reports.push(PathBuf::from(value(args, idx, "--report")?));
                    idx += 2;
                }
                "--assay-cf-report" => {
                    db_reports.push(DbReportRef {
                        cf_root: PathBuf::from(nth_value(args, idx, 1, "--assay-cf-report")?),
                        domain: nth_value(args, idx, 2, "--assay-cf-report")?.to_string(),
                        target_class: nth_value(args, idx, 3, "--assay-cf-report")?
                            .parse::<usize>()
                            .map_err(|_| {
                                format!(
                                    "{CODE_INVALID_CONFIG}: --assay-cf-report target_class must be an unsigned integer"
                                )
                            })?,
                    });
                    idx += 4;
                }
                "--out-dir" => {
                    out_dir = PathBuf::from(value(args, idx, "--out-dir")?);
                    idx += 2;
                }
                "--cf-root" => {
                    cf_root = Some(PathBuf::from(value(args, idx, "--cf-root")?));
                    idx += 2;
                }
                "--association-key" => {
                    association_key = value(args, idx, "--association-key")?.to_string();
                    idx += 2;
                }
                "--min-lenses" => {
                    min_lenses = parse_usize(args, idx, "--min-lenses")?;
                    idx += 2;
                }
                "--min-marginal-bits" => {
                    min_marginal_bits = parse_f32(args, idx, "--min-marginal-bits")?;
                    idx += 2;
                }
                "--max-redundancy" => {
                    max_redundancy = parse_f32(args, idx, "--max-redundancy")?;
                    idx += 2;
                }
                "--mode" => {
                    mode = parse_mode(value(args, idx, "--mode")?)?;
                    idx += 2;
                }
                "--diagnostic" | "--baseline" => {
                    mode = Mode::Diagnostic;
                    idx += 1;
                }
                "--db-only" | "--no-artifacts" => {
                    emit_artifacts = false;
                    idx += 1;
                }
                other => return Err(format!("{CODE_INVALID_CONFIG}: unknown arg {other}")),
            }
        }
        let cf_root = match cf_root {
            Some(path) => path,
            None if !out_dir.as_os_str().is_empty() => out_dir.join("association_cf"),
            None => {
                return Err(format!(
                    "{CODE_INVALID_CONFIG}: --cf-root is required when --db-only omits --out-dir"
                ));
            }
        };
        let request = Self {
            reports,
            db_reports,
            out_dir,
            cf_root,
            association_key,
            min_lenses,
            min_marginal_bits,
            max_redundancy,
            mode,
            emit_artifacts,
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn ensure_fresh_output(&self) -> Result<(), String> {
        if self.emit_artifacts && self.out_dir.exists() {
            return Err(format!(
                "{CODE_OUTPUT_EXISTS}: out_dir already exists: {}",
                self.out_dir.display()
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        if self.reports.len() + self.db_reports.len() < 2 {
            return Err(format!(
                "{CODE_INVALID_CONFIG}: multi-anchor card requires at least two --report or --assay-cf-report inputs"
            ));
        }
        if self.emit_artifacts && self.out_dir.as_os_str().is_empty() {
            return Err(format!("{CODE_INVALID_CONFIG}: --out-dir is required"));
        }
        if self.cf_root.as_os_str().is_empty() {
            return Err(format!(
                "{CODE_INVALID_CONFIG}: --cf-root must be non-empty"
            ));
        }
        if self.association_key.trim().is_empty() {
            return Err(format!(
                "{CODE_INVALID_CONFIG}: --association-key must be non-empty"
            ));
        }
        for report in &self.db_reports {
            if report.cf_root.as_os_str().is_empty() {
                return Err(format!(
                    "{CODE_INVALID_CONFIG}: --assay-cf-report cf_root must be non-empty"
                ));
            }
            if report.domain.trim().is_empty() {
                return Err(format!(
                    "{CODE_INVALID_CONFIG}: --assay-cf-report domain must be non-empty"
                ));
            }
        }
        if self.min_lenses == 0 {
            return Err(format!("{CODE_INVALID_CONFIG}: --min-lenses must be > 0"));
        }
        if !self.min_marginal_bits.is_finite() || self.min_marginal_bits < 0.0 {
            return Err(format!(
                "{CODE_INVALID_CONFIG}: --min-marginal-bits must be finite and non-negative"
            ));
        }
        if !self.max_redundancy.is_finite() || !(0.0..=1.0).contains(&self.max_redundancy) {
            return Err(format!(
                "{CODE_INVALID_CONFIG}: --max-redundancy must be finite and within [0,1]"
            ));
        }
        Ok(())
    }
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    nth_value(args, idx, 1, flag)
}

fn nth_value<'a>(
    args: &'a [String],
    idx: usize,
    offset: usize,
    flag: &str,
) -> Result<&'a str, String> {
    args.get(idx + offset)
        .map(String::as_str)
        .ok_or_else(|| format!("{CODE_INVALID_CONFIG}: {flag} requires a value"))
}

fn parse_usize(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    value(args, idx, flag)?
        .parse::<usize>()
        .map_err(|_| format!("{CODE_INVALID_CONFIG}: {flag} must be an unsigned integer"))
}

fn parse_f32(args: &[String], idx: usize, flag: &str) -> Result<f32, String> {
    value(args, idx, flag)?
        .parse::<f32>()
        .map_err(|_| format!("{CODE_INVALID_CONFIG}: {flag} must be a finite float"))
}

fn parse_mode(value: &str) -> Result<Mode, String> {
    match value {
        "gate" => Ok(Mode::Gate),
        "diagnostic" | "baseline" => Ok(Mode::Diagnostic),
        other => Err(format!(
            "{CODE_INVALID_CONFIG}: --mode must be gate or diagnostic, got {other}"
        )),
    }
}
