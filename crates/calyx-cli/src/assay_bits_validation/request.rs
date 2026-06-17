use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct AssayBitsRequest {
    pub(crate) corpus_dir: PathBuf,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) cf_root: PathBuf,
    pub(crate) min_bits: f32,
    pub(crate) max_corr: f32,
    pub(crate) target_class: usize,
    pub(crate) domain: String,
    /// Optional sidecar of measured per-lens resource cost. When present, the
    /// engine emits signal-density (`bits / VRAM-MB`, `bits / ms`) and requires
    /// a cost entry for every corpus lens.
    pub(crate) cost_json: Option<PathBuf>,
    /// Optional fixed panel resource budget. When present, `--cost-json` is
    /// required and admission uses the resource-aware panel packer.
    pub(crate) panel_budget_json: Option<PathBuf>,
}

impl AssayBitsRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut corpus_dir = PathBuf::new();
        let mut metrics_dir = PathBuf::new();
        let mut cf_root: Option<PathBuf> = None;
        let mut min_bits = 0.05_f32;
        let mut max_corr = 0.6_f32;
        let mut target_class = 0_usize;
        let mut domain = "ag_news".to_string();
        let mut cost_json: Option<PathBuf> = None;
        let mut panel_budget_json: Option<PathBuf> = None;
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
                "--min-bits" => {
                    min_bits = parse_f32(args, idx, "--min-bits")?;
                    idx += 2;
                }
                "--max-corr" => {
                    max_corr = parse_f32(args, idx, "--max-corr")?;
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
                "--cost-json" => {
                    cost_json = Some(PathBuf::from(value(args, idx, "--cost-json")?));
                    idx += 2;
                }
                "--panel-budget-json" => {
                    panel_budget_json =
                        Some(PathBuf::from(value(args, idx, "--panel-budget-json")?));
                    idx += 2;
                }
                other => return Err(format!("unknown assay bits-validate arg: {other}")),
            }
        }
        let cf_root = cf_root.unwrap_or_else(|| metrics_dir.join("assay_cf"));
        let request = Self {
            corpus_dir,
            metrics_dir,
            cf_root,
            min_bits,
            max_corr,
            target_class,
            domain,
            cost_json,
            panel_budget_json,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        if self.corpus_dir.as_os_str().is_empty() || self.metrics_dir.as_os_str().is_empty() {
            return Err("assay bits-validate requires --corpus-dir and --metrics-dir".to_string());
        }
        if !self.min_bits.is_finite() || self.min_bits < 0.0 {
            return Err(
                "CALYX_FSV_ASSAY_INVALID_CONFIG: --min-bits must be finite and non-negative"
                    .to_string(),
            );
        }
        if !self.max_corr.is_finite() || !(0.0..=1.0).contains(&self.max_corr) {
            return Err(
                "CALYX_FSV_ASSAY_INVALID_CONFIG: --max-corr must be finite and within [0, 1]"
                    .to_string(),
            );
        }
        if self.domain.trim().is_empty() {
            return Err("CALYX_FSV_ASSAY_INVALID_CONFIG: --domain must be non-empty".to_string());
        }
        if self.panel_budget_json.is_some() && self.cost_json.is_none() {
            return Err(
                "CALYX_FSV_ASSAY_INVALID_CONFIG: --panel-budget-json requires --cost-json"
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
