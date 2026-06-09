use std::path::Path;

use calyx_ledger::{LedgerCfStore, decode};
use serde_json::json;

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

fn hex_bytes(bytes: &[u8]) -> String {
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
