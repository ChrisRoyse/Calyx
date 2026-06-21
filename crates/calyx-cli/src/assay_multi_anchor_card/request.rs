use std::path::PathBuf;

use calyx_assay::{DEFAULT_MAX_REDUNDANCY, DEFAULT_MIN_MARGINAL_BITS};

use super::{CODE_INVALID_CONFIG, CODE_OUTPUT_EXISTS};

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) reports: Vec<PathBuf>,
    pub(crate) out_dir: PathBuf,
    pub(crate) min_lenses: usize,
    pub(crate) min_marginal_bits: f32,
    pub(crate) max_redundancy: f32,
    pub(crate) mode: Mode,
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
        let mut out_dir = PathBuf::new();
        let mut min_lenses = 10_usize;
        let mut min_marginal_bits = DEFAULT_MIN_MARGINAL_BITS;
        let mut max_redundancy = DEFAULT_MAX_REDUNDANCY;
        let mut mode = Mode::Gate;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--report" => {
                    reports.push(PathBuf::from(value(args, idx, "--report")?));
                    idx += 2;
                }
                "--out-dir" => {
                    out_dir = PathBuf::from(value(args, idx, "--out-dir")?);
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
                other => return Err(format!("{CODE_INVALID_CONFIG}: unknown arg {other}")),
            }
        }
        let request = Self {
            reports,
            out_dir,
            min_lenses,
            min_marginal_bits,
            max_redundancy,
            mode,
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn ensure_fresh_output(&self) -> Result<(), String> {
        if self.out_dir.exists() {
            return Err(format!(
                "{CODE_OUTPUT_EXISTS}: out_dir already exists: {}",
                self.out_dir.display()
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        if self.reports.len() < 2 {
            return Err(format!(
                "{CODE_INVALID_CONFIG}: multi-anchor card requires at least two --report inputs"
            ));
        }
        if self.out_dir.as_os_str().is_empty() {
            return Err(format!("{CODE_INVALID_CONFIG}: --out-dir is required"));
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
    args.get(idx + 1)
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
