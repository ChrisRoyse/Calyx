use std::path::Path;

use calyx_ledger::{LedgerCfStore, decode};
use serde_json::json;

use crate::cf_read::hex_bytes;
use crate::ledger_store::AsterLedgerCfStore;

pub fn scan_ledger_vault(vault: &Path) -> Result<(), String> {
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    for row in store.scan().map_err(|error| error.to_string())? {
        let entry = decode(&row.bytes).map_err(|error| error.to_string())?;
        let payload = serde_json::from_slice::<serde_json::Value>(&entry.payload)
            .unwrap_or_else(|_| json!({"hex": hex_bytes(&entry.payload)}));
        println!(
            "{}",
            json!({
                "seq": entry.seq,
                "kind": format!("{:?}", entry.kind),
                "payload": payload,
                "entry_hash": hex_bytes(&entry.entry_hash),
                "prev_hash": hex_bytes(&entry.prev_hash),
                "actor": format!("{:?}", entry.actor),
            })
        );
    }
    Ok(())
}
