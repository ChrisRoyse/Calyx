use super::{CompactionCatalog, SstShard};
use crate::cf::ColumnFamily;
use calyx_core::{CalyxError, Result, SlotId};
use std::fs;
use std::path::{Path, PathBuf};

pub fn catalog_from_vault_dir(vault_dir: impl AsRef<Path>) -> Result<CompactionCatalog> {
    let cf_root = vault_dir.as_ref().join("cf");
    let mut shards = Vec::new();
    if !cf_root.exists() {
        return Ok(CompactionCatalog::new(shards));
    }
    for entry in fs::read_dir(&cf_root)
        .map_err(|error| CalyxError::disk_pressure(format!("read compaction CF root: {error}")))?
    {
        let path = entry
            .map_err(|error| {
                CalyxError::disk_pressure(format!("read compaction CF entry: {error}"))
            })?
            .path();
        if !path.is_dir() {
            continue;
        }
        let Some(cf) = parse_cf_dir(&path) else {
            continue;
        };
        for sst in list_ssts(&path)? {
            shards.push(SstShard::new(cf, sst, 0)?);
        }
    }
    shards.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(CompactionCatalog::new(shards))
}

fn list_ssts(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)
        .map_err(|error| CalyxError::disk_pressure(format!("read compaction CF dir: {error}")))?
    {
        let path = entry
            .map_err(|error| {
                CalyxError::disk_pressure(format!("read compaction SST entry: {error}"))
            })?
            .path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn parse_cf_dir(path: &Path) -> Option<ColumnFamily> {
    let name = path.file_name()?.to_string_lossy();
    match name.as_ref() {
        "base" => Some(ColumnFamily::Base),
        "xterm" => Some(ColumnFamily::XTerm),
        "scalars" => Some(ColumnFamily::Scalars),
        "anchors" => Some(ColumnFamily::Anchors),
        "ledger" => Some(ColumnFamily::Ledger),
        "online" => Some(ColumnFamily::Online),
        _ if name.starts_with("slot_") => parse_slot_name(&name),
        _ => None,
    }
}

fn parse_slot_name(name: &str) -> Option<ColumnFamily> {
    let raw = name.ends_with(".raw");
    let slot = name
        .trim_start_matches("slot_")
        .trim_end_matches(".raw")
        .parse::<u16>()
        .ok()?;
    Some(if raw {
        ColumnFamily::slot_raw(SlotId::new(slot))
    } else {
        ColumnFamily::slot(SlotId::new(slot))
    })
}
