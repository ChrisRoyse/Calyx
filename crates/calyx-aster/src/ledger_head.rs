use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};
use calyx_ledger::{LedgerHeadAnchor, decode};

use crate::cf::ColumnFamily;
use crate::ledger_view::parse_aster_ledger_seq;
use crate::vault::encode::WriteRow;

const LEDGER_HEAD_DIR: &str = "ledger_head";
const LEDGER_HEAD_FILE: &str = "current.json";

pub fn head_anchor_path(vault: &Path) -> PathBuf {
    vault.join(LEDGER_HEAD_DIR).join(LEDGER_HEAD_FILE)
}

pub fn read_head_anchor(vault: &Path) -> Result<Option<LedgerHeadAnchor>> {
    let path = head_anchor_path(vault);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path)
        .map_err(|error| CalyxError::disk_pressure(format!("read Aster ledger head: {error}")))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| CalyxError::ledger_corrupt(format!("decode Aster ledger head: {error}")))
}

pub(crate) fn write_head_anchor(vault: &Path, anchor: &LedgerHeadAnchor) -> Result<()> {
    if let Some(current) = read_head_anchor(vault)? {
        if anchor.height < current.height {
            return Err(CalyxError::ledger_append_only_violation(format!(
                "Aster ledger head regressed from {} to {}",
                current.height, anchor.height
            )));
        }
        if anchor.height == current.height && anchor.tip_hash != current.tip_hash {
            return Err(CalyxError::ledger_append_only_violation(
                "Aster ledger head changed hash at the same height",
            ));
        }
    }
    let path = head_anchor_path(vault);
    let parent = path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure("Aster ledger head path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        CalyxError::disk_pressure(format!("create Aster ledger head dir: {error}"))
    })?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(anchor).map_err(|error| {
        CalyxError::ledger_corrupt(format!("encode Aster ledger head: {error}"))
    })?;
    {
        let mut file = File::create(&tmp).map_err(|error| {
            CalyxError::disk_pressure(format!("create Aster ledger head temp: {error}"))
        })?;
        file.write_all(&bytes).map_err(|error| {
            CalyxError::disk_pressure(format!("write Aster ledger head temp: {error}"))
        })?;
        file.sync_all().map_err(|error| {
            CalyxError::disk_pressure(format!("sync Aster ledger head: {error}"))
        })?;
    }
    replace_file(&tmp, &path)?;
    sync_parent(&path)
}

pub(crate) fn newest_anchor_from_rows(rows: &[WriteRow]) -> Result<Option<LedgerHeadAnchor>> {
    let mut newest = None;
    for row in rows.iter().filter(|row| row.cf == ColumnFamily::Ledger) {
        let key_seq = parse_aster_ledger_seq(&row.key)?;
        let entry = decode(&row.value)?;
        if key_seq != entry.seq {
            return Err(CalyxError::ledger_corrupt(format!(
                "Aster ledger key seq {key_seq} does not match entry seq {}",
                entry.seq
            )));
        }
        if newest.as_ref().is_none_or(|(seq, _hash)| entry.seq > *seq) {
            newest = Some((entry.seq, entry.entry_hash));
        }
    }
    newest
        .map(|(seq, hash)| {
            let height = seq
                .checked_add(1)
                .ok_or_else(|| CalyxError::ledger_corrupt("Aster ledger head overflow"))?;
            LedgerHeadAnchor::new(height, hash)
        })
        .transpose()
}

fn replace_file(tmp: &Path, path: &Path) -> Result<()> {
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).map_err(|error| {
            CalyxError::disk_pressure(format!("replace Aster ledger head: {error}"))
        })?;
    }
    fs::rename(tmp, path)
        .map_err(|error| CalyxError::disk_pressure(format!("rename Aster ledger head: {error}")))
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure("Aster ledger head path has no parent"))?;
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| CalyxError::disk_pressure(format!("sync Aster ledger head dir: {error}")))
}

#[cfg(not(unix))]
fn sync_parent(path: &Path) -> Result<()> {
    path.parent()
        .ok_or_else(|| CalyxError::disk_pressure("Aster ledger head path has no parent"))?;
    Ok(())
}
