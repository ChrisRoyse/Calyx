//! `calyx resource-drill` — manual FSV driver for `resource_status` (issue #592).
//!
//! Drives the real write path (WAL + MVCC + CF router memtable) with
//! deterministic synthetic rows so every status field is hand-computable,
//! printing the full status BEFORE, AFTER, and FINAL (post-release) so state
//! transitions are visible byte-for-byte.

use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::mvcc::Freshness;
use calyx_aster::vault::VaultOptions;

use crate::resource_status::{open_resource_vault, vram_status_from_vault};

const DRILL_VALUE_FILL: u8 = 0xA5;

pub(crate) struct ResourceDrillArgs {
    pub ops: u64,
    pub value_bytes: usize,
    pub memtable_cap: usize,
    pub pin_max_age_ms: u64,
}

pub(crate) fn run_resource_drill(vault: &Path, args: ResourceDrillArgs) -> crate::error::CliResult {
    if args.value_bytes == 0 {
        return Err("--value-bytes must be positive".to_string().into());
    }
    if args.memtable_cap == 0 {
        return Err("--memtable-cap must be positive".to_string().into());
    }
    let options = VaultOptions {
        memtable_byte_cap: args.memtable_cap,
        ..VaultOptions::default()
    };
    let store = open_resource_vault(vault, options)?;

    print_status(&store, vault, "BEFORE")?;

    let pinned = store.pin_reader(Freshness::FreshDerived, args.pin_max_age_ms);
    println!(
        "RESOURCE_DRILL PIN lease_id={} pinned_seq={} max_age_ms={}",
        pinned.lease().id(),
        pinned.lease().pinned_seq(),
        args.pin_max_age_ms
    );

    for op in 0..args.ops {
        store
            .write_cf(
                ColumnFamily::Base,
                op.to_be_bytes().to_vec(),
                vec![DRILL_VALUE_FILL; args.value_bytes],
            )
            .map_err(|error| format!("write op {op}: {error}"))?;
    }
    println!(
        "RESOURCE_DRILL WROTE ops={} value_bytes={} latest_seq={}",
        args.ops,
        args.value_bytes,
        store.latest_seq()
    );

    print_status(&store, vault, "AFTER")?;

    let released = store.release_reader(pinned.lease().id());
    println!(
        "RESOURCE_DRILL RELEASE lease_id={} released={}",
        pinned.lease().id(),
        released
    );

    print_status(&store, vault, "FINAL")?;
    Ok(())
}

fn print_status(
    store: &calyx_aster::vault::AsterVault,
    vault: &Path,
    phase: &str,
) -> std::result::Result<(), String> {
    let vram = vram_status_from_vault(vault)?;
    let status = store
        .resource_status(vault, vram)
        .map_err(|error| error.to_string())?;
    println!(
        "RESOURCE_DRILL {phase} {}",
        serde_json::to_string(&status).map_err(|error| error.to_string())?
    );
    Ok(())
}
