use std::path::PathBuf;

pub(crate) const DEFAULT_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const DEFAULT_VAULT_SALT: &str = "calyx-ph70-sextant-recall";

#[derive(Clone, Debug)]
pub(crate) struct RecallRequest {
    pub(crate) corpus_jsonl: PathBuf,
    pub(crate) queries_jsonl: PathBuf,
    pub(crate) qrels_tsv: PathBuf,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) vault: PathBuf,
    pub(crate) query_limit: usize,
    pub(crate) k: usize,
    pub(crate) min_delta: f64,
    pub(crate) vault_id: String,
    pub(crate) vault_salt: String,
}

impl RecallRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut request = Self {
            corpus_jsonl: PathBuf::new(),
            queries_jsonl: PathBuf::new(),
            qrels_tsv: PathBuf::new(),
            metrics_dir: PathBuf::new(),
            vault: PathBuf::new(),
            query_limit: 50,
            k: 10,
            min_delta: 0.15,
            vault_id: DEFAULT_VAULT_ID.to_string(),
            vault_salt: DEFAULT_VAULT_SALT.to_string(),
        };
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--corpus-jsonl" => {
                    request.corpus_jsonl = PathBuf::from(value(args, idx, "--corpus-jsonl")?);
                    idx += 2;
                }
                "--queries-jsonl" => {
                    request.queries_jsonl = PathBuf::from(value(args, idx, "--queries-jsonl")?);
                    idx += 2;
                }
                "--qrels" => {
                    request.qrels_tsv = PathBuf::from(value(args, idx, "--qrels")?);
                    idx += 2;
                }
                "--metrics-dir" => {
                    request.metrics_dir = PathBuf::from(value(args, idx, "--metrics-dir")?);
                    idx += 2;
                }
                "--vault" => {
                    request.vault = PathBuf::from(value(args, idx, "--vault")?);
                    idx += 2;
                }
                "--query-limit" => {
                    request.query_limit = parse_usize(args, idx, "--query-limit")?;
                    idx += 2;
                }
                "--k" => {
                    request.k = parse_usize(args, idx, "--k")?;
                    idx += 2;
                }
                "--min-delta" => {
                    request.min_delta = parse_f64(args, idx, "--min-delta")?;
                    idx += 2;
                }
                "--vault-id" => {
                    request.vault_id = value(args, idx, "--vault-id")?.to_string();
                    idx += 2;
                }
                "--salt" => {
                    request.vault_salt = value(args, idx, "--salt")?.to_string();
                    idx += 2;
                }
                other => return Err(format!("unknown sextant recall arg: {other}")),
            }
        }
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        if self.corpus_jsonl.as_os_str().is_empty()
            || self.queries_jsonl.as_os_str().is_empty()
            || self.qrels_tsv.as_os_str().is_empty()
            || self.metrics_dir.as_os_str().is_empty()
            || self.vault.as_os_str().is_empty()
        {
            return Err(
                "sextant recall requires --corpus-jsonl, --queries-jsonl, --qrels, --metrics-dir, and --vault"
                    .to_string(),
            );
        }
        if self.query_limit == 0 {
            return Err("CALYX_FSV_SEXTANT_INVALID_CONFIG: --query-limit must be positive".into());
        }
        if self.k == 0 {
            return Err("CALYX_FSV_SEXTANT_INVALID_CONFIG: --k must be positive".into());
        }
        if !self.min_delta.is_finite() || self.min_delta < 0.0 {
            return Err(
                "CALYX_FSV_SEXTANT_INVALID_CONFIG: --min-delta must be finite and non-negative"
                    .into(),
            );
        }
        Ok(())
    }
}

fn parse_usize(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    value(args, idx, flag)?
        .parse::<usize>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn parse_f64(args: &[String], idx: usize, flag: &str) -> Result<f64, String> {
    value(args, idx, flag)?
        .parse::<f64>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}
