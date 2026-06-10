use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::{ReversalToken, dedup_audit, dedup_undo};
use calyx_aster::sst::SstReader;
use calyx_aster::vault::encode::{decode_constellation_base, decode_write_batch};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::replay_dir;
use calyx_core::{CxId, VaultId};
use serde_json::json;

pub fn readback_dedup_audit(vault: &Path, cx_id: &str) -> Result<(), String> {
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

pub fn readback_dedup_undo(vault: &Path, token: &str) -> Result<(), String> {
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

pub fn readback_cx_list(vault: &Path) -> Result<(), String> {
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

fn vault_id_from_base(vault: &Path) -> Result<VaultId, String> {
    latest_cf_rows(vault, ColumnFamily::Base)?
        .into_values()
        .next()
        .map(|bytes| {
            decode_constellation_base(&bytes)
                .map(|cx| cx.vault_id)
                .map_err(|error| error.to_string())
        })
        .transpose()?
        .ok_or_else(|| "cannot infer vault id: base CF has no rows".to_string())
}

fn latest_cf_rows(vault: &Path, cf: ColumnFamily) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
    let mut rows = BTreeMap::new();
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            rows.insert(row.key, row.value);
        }
    }
    let replay = replay_dir(vault.join("wal")).map_err(|error| error.to_string())?;
    for record in replay.records {
        for row in decode_write_batch(&record.payload).map_err(|error| error.to_string())? {
            if row.cf == cf {
                rows.insert(row.key, row.value);
            }
        }
    }
    Ok(rows)
}

fn list_sst_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            files.push(path);
        }
    }
    files.sort_by(|left, right| sst_order(left).cmp(&sst_order(right)).then(left.cmp(right)));
    Ok(files)
}

fn sst_order(path: &Path) -> (u64, usize) {
    let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return (0, 0);
    };
    if let Some(seq) = stem.strip_prefix("compacted-") {
        return (seq.parse().unwrap_or(0), usize::MAX);
    }
    let Some((seq, index)) = stem.split_once('-') else {
        return (0, 0);
    };
    (seq.parse().unwrap_or(0), index.parse().unwrap_or(0))
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
