//! Time-travel readback commands (PH72 T04 follow-up, issue #689).
//!
//! Two thin, read-only wrappers over the already-shipped, FSV'd
//! `calyx_aster::timetravel` API:
//!
//! * `readback time-index --vault <PATH>` prints the `(millis, seqno)` pairs in
//!   the `time_index` CF in ascending order — the source of truth for the
//!   wall-clock → MVCC-seqno mapping (backed by [`read_all`]).
//! * `readback as-of --vault <PATH> --t-millis <T>` resolves the vault to the
//!   snapshot as of `T` and prints the constellation list visible at that time
//!   (backed by [`AsterVault::as_of`] + [`TimeTravelSnapshot::get_cx`]).
//!
//! Neither command mutates the vault. Failures fail loud with the underlying
//! `CalyxError` code (e.g. `CALYX_TIMETRAVEL_NO_DATA` when `T` precedes the
//! first committed write) so a broken vault is never masked by an empty result.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use calyx_aster::timetravel::read_all;
use calyx_aster::vault::encode::{decode_constellation_base, decode_write_batch};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::replay_dir;
use calyx_core::{Clock, VaultId};
use serde_json::json;

/// `readback time-index --vault <PATH>`: print every `time_index` entry in
/// `(millis, seqno)` order.
pub fn readback_time_index(vault: &Path) -> Result<(), String> {
    let store = open_vault(vault)?;
    let entries = read_all(&store).map_err(|error| error.to_string())?;
    let rows: Vec<_> = entries
        .iter()
        .map(|entry| json!({ "millis": entry.millis, "seqno": entry.seqno }))
        .collect();
    let value = json!({
        "vault": vault.display().to_string(),
        "entry_count": entries.len(),
        "entries": rows,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// `readback as-of --vault <PATH> --t-millis <T>`: resolve the snapshot as of
/// `T` and print the constellations visible at that historical sequence.
pub fn readback_as_of(vault: &Path, t_millis: &str) -> Result<(), String> {
    let t_millis: u64 = t_millis
        .parse()
        .map_err(|error| format!("invalid --t-millis: {error}"))?;
    let store = open_vault(vault)?;
    let snapshot = store.as_of(t_millis).map_err(|error| error.to_string())?;

    // The cx universe is every Base row visible at the vault's latest sequence;
    // probing each at the historical snapshot keeps only those ingested by then.
    let latest = store.latest_seq();
    let base_rows = store
        .scan_cf_at(latest, ColumnFamily::Base)
        .map_err(|error| error.to_string())?;

    let mut present = Vec::new();
    for (_key, value) in &base_rows {
        let cx = decode_constellation_base(value).map_err(|error| error.to_string())?;
        if snapshot.get_cx(cx.cx_id).is_ok() {
            present.push(json!({
                "cx_id": cx.cx_id,
                "created_at": cx.created_at,
                "panel_version": cx.panel_version,
            }));
        }
    }

    let value = json!({
        "vault": vault.display().to_string(),
        "t_millis": t_millis,
        "resolved_seqno": snapshot.seqno(),
        "constellation_count": present.len(),
        "constellations": present,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}

/// Opens the vault read-only, inferring its `VaultId` from a committed Base row
/// (a vault cannot be opened without its id because of the per-vault keyspace
/// guard). Fails loud if the vault has no constellations to infer the id from.
fn open_vault(vault: &Path) -> Result<AsterVault<impl Clock>, String> {
    let vault_id = vault_id_from_base(vault)?;
    AsterVault::open(
        vault,
        vault_id,
        b"calyx-timetravel-readback".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())
}

/// Infers the `VaultId` from the first committed Base-CF constellation. The id
/// is a keyspace prefix, so opening with the wrong id would silently read an
/// empty vault — inferring it from the data on disk avoids that.
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

/// Merges every on-disk SST row for `cf` with the rows still in the WAL, latest
/// write winning — the raw read used only to bootstrap the vault id before the
/// vault is open.
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

/// Lists `*.sst` files in `dir` in commit order (compacted files last per seq).
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
