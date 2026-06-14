use std::path::{Path, PathBuf};

use calyx_anneal::{AnnealLedgerAction, decode_anneal_ledger_payload};
use calyx_ledger::{EntryKind, LedgerCfStore, decode};
use serde_json::json;

use crate::cf_read::hex_bytes as hex;
use crate::ledger_store::AsterLedgerCfStore;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = ABLogRequest::parse(args)?;
    let entries = read_ab_entries(&request.vault, request.last)?;
    let report = json!({
        "source_of_truth": "Aster ledger CF rows plus WAL replay under <vault>/cf/ledger and <vault>/wal",
        "vault": request.vault.display().to_string(),
        "last": request.last,
        "entries": entries,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct ABLogRequest {
    vault: PathBuf,
    last: usize,
}

impl ABLogRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut vault = None;
        let mut last = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
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
                other => return Err(format!("unknown ab-log arg: {other}")),
            }
        }
        let last = last.unwrap_or(5);
        if last == 0 {
            return Err("--last must be positive".to_string());
        }
        Ok(Self {
            vault: vault.ok_or_else(|| "ab-log requires --vault".to_string())?,
            last,
        })
    }
}

fn read_ab_entries(vault: &Path, last: usize) -> Result<Vec<serde_json::Value>, String> {
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for row in store.scan().map_err(|error| error.to_string())? {
        let entry = decode(&row.bytes).map_err(|error| error.to_string())?;
        if entry.kind != EntryKind::Anneal {
            continue;
        }
        let anneal =
            decode_anneal_ledger_payload(&entry.payload).map_err(|error| error.to_string())?;
        if !matches!(
            anneal.action,
            AnnealLedgerAction::AutotuneAB
                | AnnealLedgerAction::AutotuneAbandoned
                | AnnealLedgerAction::AutotunePromote
        ) {
            continue;
        }
        entries.push(json!({
            "seq": row.seq,
            "entry_hash": hex(&entry.entry_hash),
            "payload_hex": hex(&entry.payload),
            "payload_json": anneal,
        }));
    }
    if last < entries.len() {
        entries.drain(0..entries.len() - last);
    }
    Ok(entries)
}
