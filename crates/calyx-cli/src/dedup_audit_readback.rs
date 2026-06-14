use std::path::Path;
use std::str::FromStr;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::{ReversalToken, dedup_audit, dedup_undo};
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::CxId;
use serde_json::json;

use crate::cf_read::{hex_bytes, latest_cf_rows, vault_id_from_base};

pub fn readback_dedup_audit(vault: &Path, cx_id: &str) -> crate::error::CliResult {
    let cx_id = CxId::from_str(cx_id).map_err(|error| format!("invalid --cx-id: {error}"))?;
    let vault_id = vault_id_from_base(vault)?;
    let store = AsterVault::open(
        vault,
        vault_id,
        b"calyx-dedup-audit-readback".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let report = dedup_audit(&store, cx_id).map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

pub fn readback_dedup_undo(vault: &Path, token: &str) -> crate::error::CliResult {
    let token: ReversalToken =
        serde_json::from_str(token).map_err(|error| format!("invalid --token: {error}"))?;
    let vault_id = vault_id_from_base(vault)?;
    let store = AsterVault::open(
        vault,
        vault_id,
        b"calyx-dedup-audit-readback".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let before = latest_cf_rows(vault, ColumnFamily::Base)?;
    let restored = dedup_undo(&store, &token).map_err(|error| error.to_string())?;
    store.flush().map_err(|error| error.to_string())?;
    let after = latest_cf_rows(vault, ColumnFamily::Base)?;
    let value = json!({
        "vault": vault.display().to_string(),
        "restored": restored,
        "base_rows_before": before.len(),
        "base_rows_after": after.len(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}

pub fn readback_cx_list(vault: &Path) -> crate::error::CliResult {
    let rows = latest_cf_rows(vault, ColumnFamily::Base)?;
    let mut values = Vec::new();
    for (key, value) in rows {
        let cx = decode_constellation_base(&value).map_err(|error| error.to_string())?;
        values.push(json!({
            "key_hex": hex_bytes(&key),
            "cx_id": cx.cx_id,
            "created_at": cx.created_at,
            "panel_version": cx.panel_version,
            "base_hex": hex_bytes(&value),
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&values).map_err(|error| error.to_string())?
    );
    Ok(())
}
