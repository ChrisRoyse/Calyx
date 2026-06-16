use std::path::PathBuf;

#[derive(Clone, Debug)]
pub(crate) struct OracleSufficiencyRequest {
    pub(crate) corpus_dir: PathBuf,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) cf_root: PathBuf,
    pub(crate) domain: String,
}

impl OracleSufficiencyRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut corpus_dir = PathBuf::new();
        let mut metrics_dir = PathBuf::new();
        let mut cf_root: Option<PathBuf> = None;
        let mut domain = "swebench_lite".to_string();
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
                "--domain" => {
                    domain = value(args, idx, "--domain")?.to_string();
                    idx += 2;
                }
                other => {
                    return Err(format!("unknown oracle sufficiency-validate arg: {other}"));
                }
            }
        }
        let cf_root = cf_root.unwrap_or_else(|| metrics_dir.join("oracle_cf"));
        let request = Self {
            corpus_dir,
            metrics_dir,
            cf_root,
            domain,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        if self.corpus_dir.as_os_str().is_empty() || self.metrics_dir.as_os_str().is_empty() {
            return Err(
                "oracle sufficiency-validate requires --corpus-dir and --metrics-dir".to_string(),
            );
        }
        if self.domain.trim().is_empty() {
            return Err("CALYX_FSV_ORACLE_INVALID_CONFIG: --domain must be non-empty".to_string());
        }
        Ok(())
    }
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}
