use std::path::Path;

use calyx_ledger::{
    LedgerCfStore, LedgerRow, VerifyResult, decode, tombstone_from_entry, verify_chain,
};
use serde_json::json;

use crate::cf_read::hex_bytes;
use crate::error::{CliError, CliResult};
use crate::ledger_store::AsterLedgerCfStore;

pub fn scan_ledger_vault(vault: &Path) -> crate::error::CliResult {
    let store = AsterLedgerCfStore::open(vault)?;
    for row in store.scan()? {
        println!("{}", ledger_row_json(&row)?);
    }
    Ok(())
}

pub fn tail_ledger_vault(vault: &Path, last: usize) -> crate::error::CliResult {
    let store = AsterLedgerCfStore::open(vault)?;
    let rows = store.scan()?;
    match verify_chain(&store, 0..rows.len() as u64)? {
        VerifyResult::Intact { count } => {
            println!(
                "{}",
                json!({"verify_chain": "Intact", "verified_count": count})
            );
        }
        other => {
            return Err(CliError::runtime(format!(
                "ledger chain is not intact: {other:?}"
            )));
        }
    }
    for row in rows.iter().skip(rows.len().saturating_sub(last)) {
        println!("{}", ledger_row_json(row)?);
    }
    Ok(())
}

fn ledger_row_json(row: &LedgerRow) -> CliResult<serde_json::Value> {
    let entry = decode(&row.bytes)?;
    let payload = match tombstone_from_entry(&entry) {
        Ok(Some(tombstone)) => tombstone.as_json_value(),
        Ok(None) | Err(_) => serde_json::from_slice::<serde_json::Value>(&entry.payload)
            .unwrap_or_else(|_| json!({"hex": hex_bytes(&entry.payload)})),
    };
    Ok(json!({
        "seq": entry.seq,
        "kind": format!("{:?}", entry.kind),
        "payload": payload,
        "entry_hash": hex_bytes(&entry.entry_hash),
        "prev_hash": hex_bytes(&entry.prev_hash),
        "actor": format!("{:?}", entry.actor),
    }))
}
