use std::path::Path;

use calyx_anneal::{FileWardTauStore, WardTauStore};
use calyx_core::SlotId;

pub(crate) fn readback_ward_tau(vault: &Path, slot: &str) -> crate::error::CliResult {
    if !vault.is_dir() {
        return Err(format!("--vault path {} is not a directory", vault.display()).into());
    }
    let slot_id = slot
        .parse::<SlotId>()
        .map_err(|error| format!("invalid --slot: {error}"))?;
    let store = FileWardTauStore::open(vault).map_err(|error| error.to_string())?;
    let Some(row) = store
        .readback()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|row| row.slot_id == slot_id)
    else {
        println!(
            "WARD_TAU slot_{} absent file={}",
            slot_id.get(),
            store.path().display()
        );
        return Ok(());
    };
    println!(
        "WARD_TAU slot_{} tau={:.6} far={:.6} frr={:.6} updated_at={} file={}",
        row.slot_id.get(),
        row.tau,
        row.far,
        row.frr,
        row.updated_at,
        store.path().display()
    );
    Ok(())
}
