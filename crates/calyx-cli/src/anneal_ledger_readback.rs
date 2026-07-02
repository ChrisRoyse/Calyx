use std::path::PathBuf;

use calyx_anneal::{AnnealLedgerAction, decode_anneal_ledger_payload};
use calyx_ledger::{EntryKind, LedgerCfStore, decode};
use serde_json::json;

use crate::error::CliError;
use crate::{cf_read::hex_bytes, ledger_store::AsterLedgerCfStore};

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = Request::parse(args)?;
    if request.kind != "Anneal" {
        return Err(CliError::usage(
            "readback ledger --kind currently supports only Anneal",
        ));
    }
    let store = AsterLedgerCfStore::open(&request.vault)?;
    let mut matches = Vec::new();
    for row in store.scan()? {
        let raw = decode(&row.bytes)?;
        if raw.kind != EntryKind::Anneal {
            continue;
        }
        let entry = decode_anneal_ledger_payload(&raw.payload)?;
        if entry.action != request.action {
            continue;
        }
        matches.push(json!({
            "seq": raw.seq,
            "kind": raw.kind.as_str(),
            "entry_hash": hex_bytes(&raw.entry_hash),
            "prev_hash": hex_bytes(&raw.prev_hash),
            "payload_hex": hex_bytes(&raw.payload),
            "payload": entry,
        }));
    }
    if request.last < matches.len() {
        matches.drain(0..matches.len() - request.last);
    }
    let readback = json!({
        "source_of_truth": "Aster ledger CF rows plus WAL under vault",
        "vault": request.vault.display().to_string(),
        "kind": request.kind,
        "action": request.action,
        "last": request.last,
        "rows": matches,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| {
            CliError::runtime(format!("serialize ledger readback: {error}"))
        })?
    );
    Ok(())
}

struct Request {
    vault: PathBuf,
    kind: String,
    action: AnnealLedgerAction,
    last: usize,
}

impl Request {
    fn parse(args: &[String]) -> crate::error::CliResult<Self> {
        let mut vault = None;
        let mut kind = None;
        let mut action = None;
        let mut last = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    vault = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--kind" => {
                    kind = args.get(idx + 1).cloned();
                    idx += 2;
                }
                "--action" => {
                    action = args
                        .get(idx + 1)
                        .map(|value| parse_action(value))
                        .transpose()?;
                    idx += 2;
                }
                "--last" => {
                    last = args
                        .get(idx + 1)
                        .map(|value| parse_last(value))
                        .transpose()?;
                    idx += 2;
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unknown readback ledger arg: {other}"
                    )));
                }
            }
        }
        Ok(Self {
            vault: vault
                .ok_or_else(|| CliError::usage("readback ledger requires --vault <dir>"))?,
            kind: kind.ok_or_else(|| CliError::usage("readback ledger requires --kind Anneal"))?,
            action: action
                .ok_or_else(|| CliError::usage("readback ledger requires --action <name>"))?,
            last: last.unwrap_or(3),
        })
    }
}

fn parse_last(value: &str) -> crate::error::CliResult<usize> {
    let last = value
        .parse::<usize>()
        .map_err(|error| CliError::usage(format!("invalid --last: {error}")))?;
    if last == 0 {
        return Err(CliError::usage("--last must be > 0"));
    }
    Ok(last)
}

fn parse_action(value: &str) -> crate::error::CliResult<AnnealLedgerAction> {
    match value {
        "GoodhartPassed" => Ok(AnnealLedgerAction::GoodhartPassed),
        "GoodhartFailed" => Ok(AnnealLedgerAction::GoodhartFailed),
        "Promote" | "promote" => Ok(AnnealLedgerAction::Promote),
        "Revert" | "revert" => Ok(AnnealLedgerAction::Revert),
        "Propose" | "propose" => Ok(AnnealLedgerAction::Propose),
        "LensAdmitted" | "lens_admitted" => Ok(AnnealLedgerAction::LensAdmitted),
        "LensRejected" | "lens_rejected" => Ok(AnnealLedgerAction::LensRejected),
        other => Err(CliError::usage(format!(
            "unsupported Anneal ledger action: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_action_accepts_proposal_ledger_actions() {
        assert_eq!(
            parse_action("Propose").unwrap(),
            AnnealLedgerAction::Propose
        );
        assert_eq!(
            parse_action("LensAdmitted").unwrap(),
            AnnealLedgerAction::LensAdmitted
        );
        assert_eq!(
            parse_action("LensRejected").unwrap(),
            AnnealLedgerAction::LensRejected
        );
    }
}
