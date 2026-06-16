use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct WardGuardRequest {
    pub(crate) scores: PathBuf,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) eval_split: String,
    pub(crate) target_far: f32,
    pub(crate) required_block_rate: f32,
    pub(crate) max_frr: f32,
    pub(crate) alpha: f32,
}

impl WardGuardRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut scores = PathBuf::new();
        let mut metrics_dir = PathBuf::new();
        let mut eval_split = "test".to_string();
        let mut target_far = 0.01_f32;
        let mut required_block_rate = 0.99_f32;
        let mut max_frr = 0.01_f32;
        let mut alpha = 0.05_f32;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--scores" => {
                    scores = PathBuf::from(value(args, idx, "--scores")?);
                    idx += 2;
                }
                "--metrics-dir" => {
                    metrics_dir = PathBuf::from(value(args, idx, "--metrics-dir")?);
                    idx += 2;
                }
                "--eval-split" => {
                    eval_split = value(args, idx, "--eval-split")?.to_string();
                    idx += 2;
                }
                "--target-far" => {
                    target_far = parse_f32(args, idx, "--target-far")?;
                    idx += 2;
                }
                "--required-block-rate" => {
                    required_block_rate = parse_f32(args, idx, "--required-block-rate")?;
                    idx += 2;
                }
                "--max-frr" => {
                    max_frr = parse_f32(args, idx, "--max-frr")?;
                    idx += 2;
                }
                "--alpha" => {
                    alpha = parse_f32(args, idx, "--alpha")?;
                    idx += 2;
                }
                other => return Err(format!("unknown ward guard-validate arg: {other}")),
            }
        }
        let request = Self {
            scores,
            metrics_dir,
            eval_split,
            target_far,
            required_block_rate,
            max_frr,
            alpha,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        if self.scores.as_os_str().is_empty() || self.metrics_dir.as_os_str().is_empty() {
            return Err("ward guard-validate requires --scores and --metrics-dir".to_string());
        }
        if self.eval_split.trim().is_empty() {
            return Err(
                "CALYX_FSV_WARD_INVALID_CONFIG: --eval-split must be non-empty".to_string(),
            );
        }
        in_unit("--target-far", self.target_far)?;
        in_unit("--required-block-rate", self.required_block_rate)?;
        in_unit("--max-frr", self.max_frr)?;
        in_unit("--alpha", self.alpha)?;
        Ok(())
    }
}

fn in_unit(flag: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(format!(
            "CALYX_FSV_WARD_INVALID_CONFIG: {flag} must be finite and within [0, 1]"
        ));
    }
    Ok(())
}

fn parse_f32(args: &[String], idx: usize, flag: &str) -> Result<f32, String> {
    value(args, idx, flag)?
        .parse::<f32>()
        .map_err(|error| format!("CALYX_FSV_WARD_INVALID_CONFIG: invalid {flag}: {error}"))
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}
