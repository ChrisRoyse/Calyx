use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{AnnealLedgerAction, decode_anneal_ledger_payload};
use calyx_ledger::{EntryKind, LedgerCfStore, decode};
use serde_json::{Value, json};

use crate::ledger_store::AsterLedgerCfStore;

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let request = ReportRequest::parse(args)?;
    if request.scope != "forge" {
        return Err("autotune-report currently supports --scope forge".to_string());
    }
    let cache = read_cache(&request.cache)?;
    let promotions = read_promotions(&request.vault, request.last)?;
    let report = json!({
        "scope": request.scope,
        "cache": request.cache.display().to_string(),
        "vault": request.vault.display().to_string(),
        "last": request.last,
        "cache_bytes": cache.bytes,
        "cache_entries": cache.entries,
        "recent_promotions": promotions,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct ReportRequest {
    scope: String,
    cache: PathBuf,
    vault: PathBuf,
    last: usize,
}

impl ReportRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut scope = None;
        let mut cache = None;
        let mut vault = None;
        let mut last = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--scope" => {
                    scope = args.get(idx + 1).cloned();
                    idx += 2;
                }
                "--cache" => {
                    cache = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--vault" => {
                    vault = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--last" => {
                    last = Some(
                        args.get(idx + 1)
                            .ok_or_else(|| "--last requires a value".to_string())?
                            .parse::<usize>()
                            .map_err(|error| format!("invalid --last: {error}"))?,
                    );
                    idx += 2;
                }
                other => return Err(format!("unknown autotune-report arg: {other}")),
            }
        }
        let last = last.unwrap_or(5);
        if last == 0 {
            return Err("--last must be positive".to_string());
        }
        Ok(Self {
            scope: scope.ok_or_else(|| "autotune-report requires --scope".to_string())?,
            cache: cache.ok_or_else(|| "autotune-report requires --cache".to_string())?,
            vault: vault.ok_or_else(|| "autotune-report requires --vault".to_string())?,
            last,
        })
    }
}

struct CacheReport {
    bytes: usize,
    entries: Value,
}

fn read_cache(path: &Path) -> Result<CacheReport, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("read cache {}: {error}", path.display()))?;
    let json: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse cache {}: {error}", path.display()))?;
    Ok(CacheReport {
        bytes: bytes.len(),
        entries: json.get("entries").cloned().unwrap_or_else(|| json!([])),
    })
}

fn read_promotions(vault: &Path, last: usize) -> Result<Vec<Value>, String> {
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    let mut promotions = Vec::new();
    for row in store.scan().map_err(|error| error.to_string())? {
        let entry = decode(&row.bytes).map_err(|error| error.to_string())?;
        if entry.kind != EntryKind::Anneal {
            continue;
        }
        let anneal =
            decode_anneal_ledger_payload(&entry.payload).map_err(|error| error.to_string())?;
        if anneal.action == AnnealLedgerAction::AutotunePromote
            && anneal.artifact_id.starts_with("forge:")
        {
            promotions.push(json!({
                "seq": row.seq,
                "entry_hash": hex(&entry.entry_hash),
                "payload_hex": hex(&entry.payload),
                "payload_json": anneal,
            }));
        }
    }
    if last < promotions.len() {
        promotions.drain(0..promotions.len() - last);
    }
    Ok(promotions)
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
