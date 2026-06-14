use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, SystemClock, VaultId};
use calyx_lodestar::{
    AsterSummarizeRequest, DEFAULT_ASTER_ASSOC_COLLECTION, RecallTestParams, Scope, ScopeCache,
    SummarizeParams, summarize_vault_as_of, summarize_vault_latest,
};

use crate::cf_read::vault_id_from_base;
use crate::error::{CliError, CliResult};

const DEFAULT_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const DEFAULT_SALT: &[u8] = b"calyx-summarize-cli";

pub(crate) fn run(args: &[String]) -> CliResult {
    let request = Request::parse(args)?;
    let vault_id = request.vault_id(&request.vault)?;
    let vault = AsterVault::open(
        &request.vault,
        vault_id,
        request.salt.unwrap_or_else(|| DEFAULT_SALT.to_vec()),
        VaultOptions::default(),
    )
    .map_err(CliError::from)?;
    let mut cache = ScopeCache::new(32);
    let clock = SystemClock;
    let summarize_request = AsterSummarizeRequest {
        collection: &request.graph,
        scope: request.scope,
        params: Some(request.params),
        recall_params: request.recall,
    };
    let result = if let Some(t) = request.as_of {
        summarize_vault_as_of(&vault, summarize_request, t, &mut cache, &clock)
    } else {
        summarize_vault_latest(&vault, summarize_request, &mut cache, &clock)
    }
    .map_err(CliError::from)?;
    vault.flush().map_err(CliError::from)?;
    if let Some(parent) = request.out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &request.out,
        serde_json::to_vec_pretty(&result).map_err(CliError::from)?,
    )?;
    println!("{result}");
    Ok(())
}

struct Request {
    vault: PathBuf,
    graph: String,
    scope: Scope,
    out: PathBuf,
    as_of: Option<u64>,
    params: SummarizeParams,
    recall: RecallTestParams,
    vault_id: Option<VaultId>,
    salt: Option<Vec<u8>>,
}

impl Request {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut graph = DEFAULT_ASTER_ASSOC_COLLECTION.to_string();
        let mut scope = None;
        let mut out = None;
        let mut as_of = None;
        let mut params = SummarizeParams::default();
        let mut recall = RecallTestParams::default();
        let mut vault_id = None;
        let mut salt = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    vault = Some(PathBuf::from(value(args, idx, "--vault")?));
                    idx += 2;
                }
                "--graph" => {
                    graph = value(args, idx, "--graph")?.to_string();
                    idx += 2;
                }
                "--scope" => {
                    scope = Some(parse_scope(value(args, idx, "--scope")?)?);
                    idx += 2;
                }
                "--out" => {
                    out = Some(PathBuf::from(value(args, idx, "--out")?));
                    idx += 2;
                }
                "--as-of" => {
                    as_of = Some(parse_u64(value(args, idx, "--as-of")?, "--as-of")?);
                    idx += 2;
                }
                "--max-kernel-size" => {
                    params.max_kernel_size = Some(parse_usize(
                        value(args, idx, "--max-kernel-size")?,
                        "--max-kernel-size",
                    )?);
                    idx += 2;
                }
                "--require-grounded" => {
                    params.require_grounded = true;
                    idx += 1;
                }
                "--cache-ttl-secs" => {
                    params.cache_ttl_secs = Some(parse_u64(
                        value(args, idx, "--cache-ttl-secs")?,
                        "--cache-ttl-secs",
                    )?);
                    idx += 2;
                }
                "--anchor-label" => {
                    params.anchor_kind = Some(AnchorKind::Label(
                        value(args, idx, "--anchor-label")?.to_string(),
                    ));
                    idx += 2;
                }
                "--recall-top-k" => {
                    recall.top_k =
                        parse_usize(value(args, idx, "--recall-top-k")?, "--recall-top-k")?;
                    idx += 2;
                }
                "--recall-held-out" => {
                    recall.held_out_fraction =
                        parse_f32(value(args, idx, "--recall-held-out")?, "--recall-held-out")?;
                    idx += 2;
                }
                "--recall-seed" => {
                    recall.rng_seed =
                        parse_u64(value(args, idx, "--recall-seed")?, "--recall-seed")?;
                    idx += 2;
                }
                "--recall-min-ratio" => {
                    recall.min_recall_ratio = parse_f32(
                        value(args, idx, "--recall-min-ratio")?,
                        "--recall-min-ratio",
                    )?;
                    idx += 2;
                }
                "--vault-id" => {
                    vault_id = Some(value(args, idx, "--vault-id")?.parse().map_err(|error| {
                        CliError::usage(format!("invalid --vault-id: {error}"))
                    })?);
                    idx += 2;
                }
                "--salt" => {
                    salt = Some(value(args, idx, "--salt")?.as_bytes().to_vec());
                    idx += 2;
                }
                other => return Err(CliError::usage(format!("unknown summarize arg: {other}"))),
            }
        }
        Ok(Self {
            vault: vault.ok_or_else(|| CliError::usage("summarize requires --vault <dir>"))?,
            graph,
            scope: scope
                .ok_or_else(|| CliError::usage("summarize requires --scope <json|@file>"))?,
            out: out.ok_or_else(|| CliError::usage("summarize requires --out <file>"))?,
            as_of,
            params,
            recall,
            vault_id,
            salt,
        })
    }

    fn vault_id(&self, vault: &Path) -> Result<VaultId, CliError> {
        if let Some(vault_id) = self.vault_id {
            return Ok(vault_id);
        }
        vault_id_from_base(vault)
            .or_else(|_| {
                DEFAULT_VAULT_ID
                    .parse::<VaultId>()
                    .map_err(|error| format!("invalid default summarize vault id: {error}"))
            })
            .map_err(CliError::usage)
    }
}

fn parse_scope(value: &str) -> CliResult<Scope> {
    let text = if let Some(path) = value.strip_prefix('@') {
        fs::read_to_string(path)?
    } else {
        value.to_string()
    };
    serde_json::from_str(&text).map_err(CliError::from)
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> CliResult<&'a str> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn parse_usize(value: &str, flag: &str) -> CliResult<usize> {
    value
        .parse()
        .map_err(|error| CliError::usage(format!("invalid {flag}: {error}")))
}

fn parse_u64(value: &str, flag: &str) -> CliResult<u64> {
    value
        .parse()
        .map_err(|error| CliError::usage(format!("invalid {flag}: {error}")))
}

fn parse_f32(value: &str, flag: &str) -> CliResult<f32> {
    value
        .parse()
        .map_err(|error| CliError::usage(format!("invalid {flag}: {error}")))
}
