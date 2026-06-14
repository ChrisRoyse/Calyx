use std::path::Path;

use calyx_ledger::{
    LedgerCfStore, LedgerRow, VerifyResult, decode, tombstone_from_entry, verify_chain,
};
use serde_json::json;

use crate::cf_read::hex_bytes;
use crate::ledger_store::AsterLedgerCfStore;

pub fn scan_ledger_vault(vault: &Path) -> crate::error::CliResult {
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    for row in store.scan().map_err(|error| error.to_string())? {
        println!("{}", ledger_row_json(&row)?);
    }
    Ok(())
}

pub fn tail_ledger_vault(vault: &Path, last: usize) -> crate::error::CliResult {
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    let rows = store.scan().map_err(|error| error.to_string())?;
    match verify_chain(&store, 0..rows.len() as u64).map_err(|error| error.to_string())? {
        VerifyResult::Intact { count } => {
            println!(
                "{}",
                json!({"verify_chain": "Intact", "verified_count": count})
            );
        }
        other => return Err(format!("ledger chain is not intact: {other:?}").into()),
    }
    for row in rows.iter().skip(rows.len().saturating_sub(last)) {
        println!("{}", ledger_row_json(row)?);
    }
    Ok(())
}

fn ledger_row_json(row: &LedgerRow) -> Result<serde_json::Value, String> {
    let entry = decode(&row.bytes).map_err(|error| error.to_string())?;
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
