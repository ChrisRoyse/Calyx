use std::path::PathBuf;

use super::DEFAULT_MIN_DOCS;

const DEFAULT_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const DEFAULT_VAULT_SALT: &[u8] = b"calyx-ph70-anneal-soak";
const DEFAULT_SAMPLE_INTERVAL: u64 = 10_000;

pub(crate) struct SoakRequest {
    pub(crate) queries: u64,
    pub(crate) sample_interval: u64,
    pub(crate) vault: PathBuf,
    pub(crate) corpus_jsonl: PathBuf,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) min_docs: usize,
    pub(crate) vault_id: String,
    pub(crate) vault_salt: String,
}

impl SoakRequest {
    pub(super) fn parse(args: &[String]) -> Result<Self, String> {
        let mut request = Self {
            queries: 1_000_000,
            sample_interval: DEFAULT_SAMPLE_INTERVAL,
            vault: PathBuf::new(),
            corpus_jsonl: PathBuf::new(),
            metrics_dir: PathBuf::new(),
            min_docs: DEFAULT_MIN_DOCS,
            vault_id: DEFAULT_VAULT_ID.to_string(),
            vault_salt: String::from_utf8(DEFAULT_VAULT_SALT.to_vec()).expect("ascii salt"),
        };
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--queries" => {
                    request.queries = parse_u64(args, idx, "--queries")?;
                    idx += 2;
                }
                "--sample-interval" => {
                    request.sample_interval = parse_u64(args, idx, "--sample-interval")?;
                    idx += 2;
                }
                "--vault" => {
                    request.vault = PathBuf::from(value(args, idx, "--vault")?);
                    idx += 2;
                }
                "--corpus-jsonl" => {
                    request.corpus_jsonl = PathBuf::from(value(args, idx, "--corpus-jsonl")?);
                    idx += 2;
                }
                "--metrics-dir" => {
                    request.metrics_dir = PathBuf::from(value(args, idx, "--metrics-dir")?);
                    idx += 2;
                }
                "--min-docs" => {
                    request.min_docs = parse_usize(args, idx, "--min-docs")?;
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
                other => return Err(format!("unknown soak arg: {other}")),
            }
        }
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        if self.queries == 0 {
            return Err("CALYX_ANNEAL_SOAK_INVALID_CONFIG: --queries must be positive".to_string());
        }
        if self.sample_interval == 0 {
            return Err(
                "CALYX_ANNEAL_SOAK_INVALID_CONFIG: --sample-interval must be positive".to_string(),
            );
        }
        if self.vault.as_os_str().is_empty()
            || self.corpus_jsonl.as_os_str().is_empty()
            || self.metrics_dir.as_os_str().is_empty()
        {
            return Err("soak requires --vault, --corpus-jsonl, and --metrics-dir".to_string());
        }
        Ok(())
    }
}

fn parse_u64(args: &[String], idx: usize, flag: &str) -> Result<u64, String> {
    value(args, idx, flag)?
        .parse::<u64>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn parse_usize(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    value(args, idx, flag)?
        .parse::<usize>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}
