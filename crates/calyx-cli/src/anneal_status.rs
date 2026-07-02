use std::collections::BTreeMap;
use std::path::Path;

use calyx_anneal::{AnnealLedgerAction, decode_anneal_ledger_payload, decode_health_value};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::wal::replay_dir;
use calyx_ledger::{EntryKind, LedgerCfStore, decode};

use crate::cf_read::list_sst_files;
use crate::error::{CliError, CliResult};
use crate::ledger_store::AsterLedgerCfStore;

pub(crate) fn status_health(vault: &Path) -> crate::error::CliResult {
    if !vault.is_dir() {
        return Err(CliError::io(format!(
            "--vault path {} is not a directory",
            vault.display()
        )));
    }
    let mut rows = BTreeMap::<Vec<u8>, Vec<u8>>::new();
    read_sst_rows(vault, &mut rows)?;
    read_wal_rows(vault, &mut rows)?;

    if rows.is_empty() {
        println!("ANNEAL_HEALTH empty");
        return Ok(());
    }
    for value in rows.values() {
        let row = decode_health_value(value)?;
        println!("{}: {}", row.kind, row.health);
    }
    Ok(())
}

pub(crate) fn status_faults(vault: &Path, last: usize) -> crate::error::CliResult {
    if last == 0 {
        return Err(CliError::usage("--last must be positive"));
    }
    let store = AsterLedgerCfStore::open(vault)?;
    let mut faults = Vec::new();
    for row in store.scan()? {
        let entry = decode(&row.bytes)?;
        if entry.kind != EntryKind::Anneal {
            continue;
        }
        let anneal = decode_anneal_ledger_payload(&entry.payload)?;
        if anneal.action == AnnealLedgerAction::FaultEvent {
            faults.push(anneal);
        }
    }
    if faults.is_empty() {
        println!("ANNEAL_FAULTS empty");
        return Ok(());
    }
    if last < faults.len() {
        faults.drain(0..faults.len() - last);
    }
    for entry in faults {
        if let Some(fault) = entry.fault {
            println!(
                "FaultEvent ts={} component={} kind={} recommendation={}",
                entry.ts,
                fault.component_label(),
                fault.fault_kind,
                fault.recommendation
            );
        } else {
            println!(
                "FaultEvent ts={} description={}",
                entry.ts, entry.description
            );
        }
    }
    Ok(())
}

pub(crate) fn parse_last(value: &str) -> CliResult<usize> {
    value
        .parse::<usize>()
        .map_err(|error| CliError::usage(format!("invalid --last: {error}")))
}

fn read_sst_rows(vault: &Path, rows: &mut BTreeMap<Vec<u8>, Vec<u8>>) -> crate::error::CliResult {
    for file in list_sst_files(&vault.join("cf").join(ColumnFamily::AnnealHealth.name()))? {
        let reader = SstReader::open(&file)?;
        for row in reader.iter()? {
            rows.insert(row.key, row.value);
        }
    }
    Ok(())
}

fn read_wal_rows(vault: &Path, rows: &mut BTreeMap<Vec<u8>, Vec<u8>>) -> crate::error::CliResult {
    let wal_dir = vault.join("wal");
    if !wal_dir.is_dir() {
        return Ok(());
    }
    let replay = replay_dir(wal_dir)?;
    if let Some(torn) = replay.torn_tail {
        return Err(CliError::runtime(torn.error().to_string()));
    }
    for record in replay.records {
        for row in decode_write_batch(&record.payload)? {
            if row.cf == ColumnFamily::AnnealHealth {
                rows.insert(row.key, row.value);
            }
        }
    }
    Ok(())
}
