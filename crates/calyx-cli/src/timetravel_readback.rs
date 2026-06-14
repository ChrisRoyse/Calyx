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

use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::timetravel::read_all;
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::Clock;
use serde_json::json;

use crate::cf_read::vault_id_from_base;

/// `readback time-index --vault <PATH>`: print every `time_index` entry in
/// `(millis, seqno)` order.
pub fn readback_time_index(vault: &Path) -> crate::error::CliResult {
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
pub fn readback_as_of(vault: &Path, t_millis: &str) -> crate::error::CliResult {
    let t_millis: u64 = t_millis
        .parse()
        .map_err(|error| format!("invalid --t-millis: {error}"))?;
    let store = open_vault(vault)?;
    // Readback is an integrity surface: validate the physical time-index CF
    // before printing a snapshot so malformed keys are never masked.
    read_all(&store).map_err(|error| error.to_string())?;
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
