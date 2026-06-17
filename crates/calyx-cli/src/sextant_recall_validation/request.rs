use std::path::PathBuf;

pub(crate) const DEFAULT_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const DEFAULT_VAULT_SALT: &str = "calyx-ph70-sextant-recall";
const DEFAULT_RERANKER_ENDPOINT: &str = "http://127.0.0.1:8089";
const DEFAULT_RERANKER_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_RERANK_DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub(crate) struct RecallRequest {
    pub(crate) corpus_jsonl: PathBuf,
    pub(crate) queries_jsonl: PathBuf,
    pub(crate) qrels_tsv: PathBuf,
    pub(crate) packed_panel_json: Option<PathBuf>,
    pub(crate) lens_catalog: Option<PathBuf>,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) vault: PathBuf,
    pub(crate) query_limit: usize,
    pub(crate) k: usize,
    pub(crate) min_delta: f64,
    pub(crate) min_fusion_gain: f64,
    pub(crate) reranker_endpoint: String,
    pub(crate) reranker_timeout_ms: u64,
    pub(crate) rerank_depth: usize,
    pub(crate) vault_id: String,
    pub(crate) vault_salt: String,
}

impl RecallRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut request = Self {
            corpus_jsonl: PathBuf::new(),
            queries_jsonl: PathBuf::new(),
            qrels_tsv: PathBuf::new(),
            packed_panel_json: None,
            lens_catalog: None,
            metrics_dir: PathBuf::new(),
            vault: PathBuf::new(),
            query_limit: 50,
            k: 10,
            min_delta: 0.15,
            min_fusion_gain: 0.0,
            reranker_endpoint: DEFAULT_RERANKER_ENDPOINT.to_string(),
            reranker_timeout_ms: DEFAULT_RERANKER_TIMEOUT_MS,
            rerank_depth: DEFAULT_RERANK_DEPTH,
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
                "--packed-panel-json" => {
                    request.packed_panel_json =
                        Some(PathBuf::from(value(args, idx, "--packed-panel-json")?));
                    idx += 2;
                }
                "--lens-catalog" => {
                    request.lens_catalog = Some(PathBuf::from(value(args, idx, "--lens-catalog")?));
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
                "--min-fusion-gain" => {
                    request.min_fusion_gain = parse_f64(args, idx, "--min-fusion-gain")?;
                    idx += 2;
                }
                "--reranker-endpoint" => {
                    request.reranker_endpoint =
                        value(args, idx, "--reranker-endpoint")?.to_string();
                    idx += 2;
                }
                "--reranker-timeout-ms" => {
                    request.reranker_timeout_ms = parse_u64(args, idx, "--reranker-timeout-ms")?;
                    idx += 2;
                }
                "--rerank-depth" => {
                    request.rerank_depth = parse_usize(args, idx, "--rerank-depth")?;
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
        if !self.min_fusion_gain.is_finite() || self.min_fusion_gain < 0.0 {
            return Err(
                "CALYX_FSV_SEXTANT_INVALID_CONFIG: --min-fusion-gain must be finite and non-negative"
                    .into(),
            );
        }
        if !self.reranker_endpoint.starts_with("http://") {
            return Err(
                "CALYX_FSV_SEXTANT_INVALID_CONFIG: --reranker-endpoint must be http://".into(),
            );
        }
        if self.reranker_timeout_ms == 0 {
            return Err(
                "CALYX_FSV_SEXTANT_INVALID_CONFIG: --reranker-timeout-ms must be positive".into(),
            );
        }
        if self.rerank_depth < self.k {
            return Err("CALYX_FSV_SEXTANT_INVALID_CONFIG: --rerank-depth must be >= --k".into());
        }
        Ok(())
    }

    pub(crate) fn real_panel_enabled(&self) -> bool {
        self.packed_panel_json.is_some()
    }
}

fn parse_usize(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    value(args, idx, flag)?
        .parse::<usize>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn parse_u64(args: &[String], idx: usize, flag: &str) -> Result<u64, String> {
    value(args, idx, flag)?
        .parse::<u64>()
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
