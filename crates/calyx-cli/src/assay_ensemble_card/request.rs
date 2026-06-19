use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct EnsembleCardRequest {
    pub(crate) corpus_dir: PathBuf,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) cf_root: PathBuf,
    pub(crate) target_class: usize,
    pub(crate) domain: String,
    pub(crate) min_lenses: usize,
    pub(crate) min_marginal_bits: f32,
    pub(crate) max_redundancy: f32,
}

impl EnsembleCardRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut corpus_dir = PathBuf::new();
        let mut metrics_dir = PathBuf::new();
        let mut cf_root: Option<PathBuf> = None;
        let mut target_class = 0_usize;
        let mut domain = "assay_ensemble".to_string();
        let mut min_lenses = calyx_assay::DEFAULT_GATE_PANEL_LENSES;
        let mut min_marginal_bits = calyx_assay::DEFAULT_MIN_MARGINAL_BITS;
        let mut max_redundancy = calyx_assay::DEFAULT_MAX_REDUNDANCY;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--corpus-dir" => {
                    corpus_dir = PathBuf::from(value(args, idx, "--corpus-dir")?);
                    idx += 2;
                }
                "--metrics-dir" => {
                    metrics_dir = PathBuf::from(value(args, idx, "--metrics-dir")?);
                    idx += 2;
                }
                "--cf-root" => {
                    cf_root = Some(PathBuf::from(value(args, idx, "--cf-root")?));
                    idx += 2;
                }
                "--target-class" => {
                    target_class = parse_usize(args, idx, "--target-class")?;
                    idx += 2;
                }
                "--domain" => {
                    domain = value(args, idx, "--domain")?.to_string();
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
                other => return Err(format!("unknown assay ensemble-card arg: {other}")),
            }
        }
        let cf_root = cf_root.unwrap_or_else(|| metrics_dir.join("assay_cf"));
        let request = Self {
            corpus_dir,
            metrics_dir,
            cf_root,
            target_class,
            domain,
            min_lenses,
            min_marginal_bits,
            max_redundancy,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        if self.corpus_dir.as_os_str().is_empty() || self.metrics_dir.as_os_str().is_empty() {
            return Err("assay ensemble-card requires --corpus-dir and --metrics-dir".to_string());
        }
        if self.domain.trim().is_empty() {
            return Err("CALYX_FSV_ASSAY_INVALID_CONFIG: --domain must be non-empty".to_string());
        }
        if !self.min_marginal_bits.is_finite() || self.min_marginal_bits < 0.0 {
            return Err(
                "CALYX_FSV_ASSAY_INVALID_CONFIG: --min-marginal-bits must be finite and non-negative"
                    .to_string(),
            );
        }
        if !self.max_redundancy.is_finite() || !(0.0..=1.0).contains(&self.max_redundancy) {
            return Err(
                "CALYX_FSV_ASSAY_INVALID_CONFIG: --max-redundancy must be finite and within [0,1]"
                    .to_string(),
            );
        }
        Ok(())
    }
}

fn parse_usize(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    value(args, idx, flag)?
        .parse::<usize>()
        .map_err(|error| format!("CALYX_FSV_ASSAY_INVALID_CONFIG: invalid {flag}: {error}"))
}

fn parse_f32(args: &[String], idx: usize, flag: &str) -> Result<f32, String> {
    value(args, idx, flag)?
        .parse::<f32>()
        .map_err(|error| format!("CALYX_FSV_ASSAY_INVALID_CONFIG: invalid {flag}: {error}"))
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}
