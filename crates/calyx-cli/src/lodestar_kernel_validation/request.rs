use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct LodestarKernelRequest {
    pub(crate) corpora_dir: PathBuf,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) min_ratio: f32,
    pub(crate) query_limit: usize,
    pub(crate) top_k: usize,
}

impl LodestarKernelRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut request = Self {
            corpora_dir: PathBuf::new(),
            metrics_dir: PathBuf::new(),
            min_ratio: 0.95,
            query_limit: 500,
            top_k: 10,
        };
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--corpora-dir" => {
                    request.corpora_dir = PathBuf::from(value(args, idx, "--corpora-dir")?);
                    idx += 2;
                }
                "--metrics-dir" => {
                    request.metrics_dir = PathBuf::from(value(args, idx, "--metrics-dir")?);
                    idx += 2;
                }
                "--min-ratio" => {
                    request.min_ratio = parse_f32(args, idx, "--min-ratio")?;
                    idx += 2;
                }
                "--query-limit" => {
                    request.query_limit = parse_usize(args, idx, "--query-limit")?;
                    idx += 2;
                }
                "--top-k" => {
                    request.top_k = parse_usize(args, idx, "--top-k")?;
                    idx += 2;
                }
                other => return Err(format!("unknown lodestar kernel arg: {other}")),
            }
        }
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        if self.corpora_dir.as_os_str().is_empty() || self.metrics_dir.as_os_str().is_empty() {
            return Err(
                "lodestar kernel validation requires --corpora-dir and --metrics-dir".to_string(),
            );
        }
        if !self.min_ratio.is_finite() || self.min_ratio < 0.0 || self.min_ratio > 1.0 {
            return Err(
                "CALYX_FSV_LODESTAR_INVALID_CONFIG: --min-ratio must be finite and within [0, 1]"
                    .to_string(),
            );
        }
        if self.query_limit == 0 {
            return Err(
                "CALYX_FSV_LODESTAR_INVALID_CONFIG: --query-limit must be positive".to_string(),
            );
        }
        if self.top_k == 0 {
            return Err("CALYX_FSV_LODESTAR_INVALID_CONFIG: --top-k must be positive".to_string());
        }
        Ok(())
    }
}

fn parse_usize(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    value(args, idx, flag)?
        .parse::<usize>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn parse_f32(args: &[String], idx: usize, flag: &str) -> Result<f32, String> {
    value(args, idx, flag)?
        .parse::<f32>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}
