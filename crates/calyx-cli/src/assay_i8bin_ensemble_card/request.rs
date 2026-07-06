use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct I8binEnsembleRequest {
    pub(crate) plan: Option<PathBuf>,
    pub(crate) plan_cf_root: Option<PathBuf>,
    pub(crate) plan_key: String,
    pub(crate) rows_jsonl: PathBuf,
    pub(crate) labels_cf_root: Option<PathBuf>,
    pub(crate) labels_key: String,
    pub(crate) stream_report: Option<PathBuf>,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) cf_root: PathBuf,
    pub(crate) target_class: usize,
    pub(crate) domain: String,
    pub(crate) sample_rows: usize,
    pub(crate) signature_rows: Option<usize>,
    pub(crate) min_lenses: usize,
    pub(crate) min_marginal_bits: f32,
    pub(crate) max_redundancy: f32,
    pub(crate) nmi_bins: usize,
    pub(crate) mode: A37CardMode,
    pub(crate) emit_artifacts: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum A37CardMode {
    Gate,
    Diagnostic,
}

impl A37CardMode {
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

impl I8binEnsembleRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut plan = None;
        let mut plan_cf_root = None;
        let mut plan_key = crate::partitioned_bench::rrf_plan::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut rows_jsonl = PathBuf::new();
        let mut labels_cf_root = None;
        let mut labels_key = super::label_store::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut stream_report = None;
        let mut metrics_dir = PathBuf::new();
        let mut cf_root = None;
        let mut target_class = 1_usize;
        let mut domain = "i8bin_ensemble_card".to_string();
        let mut sample_rows = 10_000_usize;
        let mut signature_rows = None;
        let mut min_lenses = calyx_assay::DEFAULT_GATE_PANEL_LENSES;
        let mut min_marginal_bits = calyx_assay::DEFAULT_MIN_MARGINAL_BITS;
        let mut max_redundancy = calyx_assay::DEFAULT_MAX_REDUNDANCY;
        let mut nmi_bins = 10_usize;
        let mut mode = A37CardMode::Gate;
        let mut emit_artifacts = true;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--plan" => {
                    plan = Some(PathBuf::from(value(args, idx, "--plan")?));
                    idx += 2;
                }
                "--plan-cf-root" => {
                    plan_cf_root = Some(PathBuf::from(value(args, idx, "--plan-cf-root")?));
                    idx += 2;
                }
                "--plan-key" | "--plan-association-key" => {
                    plan_key = value(args, idx, "--plan-key")?.to_string();
                    idx += 2;
                }
                "--rows-jsonl" => {
                    rows_jsonl = PathBuf::from(value(args, idx, "--rows-jsonl")?);
                    idx += 2;
                }
                "--labels-cf-root" => {
                    labels_cf_root = Some(PathBuf::from(value(args, idx, "--labels-cf-root")?));
                    idx += 2;
                }
                "--labels-key" | "--label-key" | "--labels-association-key" => {
                    labels_key = value(args, idx, "--labels-key")?.to_string();
                    idx += 2;
                }
                "--stream-report" => {
                    stream_report = Some(PathBuf::from(value(args, idx, "--stream-report")?));
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
                "--sample-rows" => {
                    sample_rows = parse_usize(args, idx, "--sample-rows")?;
                    idx += 2;
                }
                "--signature-rows" => {
                    signature_rows = parse_signature_rows(value(args, idx, "--signature-rows")?)?;
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
                "--nmi-bins" => {
                    nmi_bins = parse_usize(args, idx, "--nmi-bins")?;
                    idx += 2;
                }
                "--mode" => {
                    mode = parse_mode(value(args, idx, "--mode")?)?;
                    idx += 2;
                }
                "--diagnostic" | "--baseline" => {
                    mode = A37CardMode::Diagnostic;
                    idx += 1;
                }
                "--db-only" | "--no-artifacts" => {
                    emit_artifacts = false;
                    idx += 1;
                }
                other => {
                    return Err(format!(
                        "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: unknown arg {other}"
                    ));
                }
            }
        }
        let cf_root = match cf_root {
            Some(path) => path,
            None if !metrics_dir.as_os_str().is_empty() => metrics_dir.join("assay_cf"),
            None => {
                return Err(
                    "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --cf-root is required when --db-only omits --metrics-dir"
                        .to_string(),
                );
            }
        };
        let request = Self {
            plan,
            plan_cf_root,
            plan_key,
            rows_jsonl,
            labels_cf_root,
            labels_key,
            stream_report,
            metrics_dir,
            cf_root,
            target_class,
            domain,
            sample_rows,
            signature_rows,
            min_lenses,
            min_marginal_bits,
            max_redundancy,
            nmi_bins,
            mode,
            emit_artifacts,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        let has_plan_file = self.plan.is_some();
        let has_plan_db = self.plan_cf_root.is_some();
        if has_plan_file == has_plan_db {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: provide exactly one of --plan <json> or --plan-cf-root <aster-dir>"
                    .to_string(),
            );
        }
        if self.plan_key.trim().is_empty() {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --plan-key must be non-empty"
                    .to_string(),
            );
        }
        let has_rows_file = !self.rows_jsonl.as_os_str().is_empty();
        let has_labels_db = self.labels_cf_root.is_some();
        if has_rows_file == has_labels_db {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: provide exactly one of --rows-jsonl <rows.jsonl> or --labels-cf-root <aster-dir>"
                    .to_string(),
            );
        }
        if has_labels_db && self.labels_key.trim().is_empty() {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --labels-key must be non-empty"
                    .to_string(),
            );
        }
        if self.mode.requires_gate() && !has_labels_db {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: gate mode requires --labels-cf-root; --rows-jsonl labels are diagnostic/import-only"
                    .to_string(),
            );
        }
        if self.emit_artifacts && self.metrics_dir.as_os_str().is_empty() {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --metrics-dir is required unless --db-only is set"
                    .to_string(),
            );
        }
        if self.cf_root.as_os_str().is_empty() {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --cf-root must be non-empty"
                    .to_string(),
            );
        }
        if self.domain.trim().is_empty() {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --domain must be non-empty".to_string(),
            );
        }
        if self.sample_rows == 0 {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --sample-rows must be > 0".to_string(),
            );
        }
        if self.nmi_bins < 2 {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --nmi-bins must be >= 2".to_string(),
            );
        }
        if !self.min_marginal_bits.is_finite() || self.min_marginal_bits < 0.0 {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --min-marginal-bits must be finite and non-negative"
                    .to_string(),
            );
        }
        if !self.max_redundancy.is_finite() || !(0.0..=1.0).contains(&self.max_redundancy) {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --max-redundancy must be finite and within [0,1]"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(crate) fn ensure_fresh_outputs(&self) -> Result<(), String> {
        if self.emit_artifacts && self.metrics_dir.exists() {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_OUTPUT_EXISTS: metrics_dir already exists: {}",
                self.metrics_dir.display()
            ));
        }
        if self.emit_artifacts && self.cf_root.exists() {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_OUTPUT_EXISTS: cf_root already exists: {}",
                self.cf_root.display()
            ));
        }
        Ok(())
    }
}

fn parse_signature_rows(value: &str) -> Result<Option<usize>, String> {
    if value == "all" {
        return Ok(None);
    }
    value.parse::<usize>().map(Some).map_err(|error| {
        format!("CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: invalid --signature-rows: {error}")
    })
}

fn parse_mode(value: &str) -> Result<A37CardMode, String> {
    match value {
        "gate" => Ok(A37CardMode::Gate),
        "diagnostic" | "baseline" => Ok(A37CardMode::Diagnostic),
        other => Err(format!(
            "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: --mode must be gate or diagnostic, got {other}"
        )),
    }
}

fn parse_usize(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    value(args, idx, flag)?.parse::<usize>().map_err(|error| {
        format!("CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: invalid {flag}: {error}")
    })
}

fn parse_f32(args: &[String], idx: usize, flag: &str) -> Result<f32, String> {
    value(args, idx, flag)?.parse::<f32>().map_err(|error| {
        format!("CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: invalid {flag}: {error}")
    })
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1).map(String::as_str).ok_or_else(|| {
        format!("CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG: {flag} requires a value")
    })
}
