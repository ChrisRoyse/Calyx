use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};

use crate::cf::ColumnFamily;
use crate::storage_names::{classify_sst, ensure_unambiguous_sst_order, sst_order_key};

pub(super) fn list_base_sst_files(vault: &Path) -> Result<Vec<PathBuf>> {
    let dir = vault.join("cf").join(ColumnFamily::Base.name());
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(&dir)
        .map_err(|error| CalyxError::disk_pressure(format!("list Base SST files: {error}")))?
    {
        let path = entry
            .map_err(|error| CalyxError::disk_pressure(format!("list Base SST files: {error}")))?
            .path();
        if classify_sst(&path)?.is_some() {
            let order = sst_order_key(&path)?.expect("classified SST has order key");
            files.push((order, path));
        }
    }
    // The page index folds rows newest-wins in this order (issue #1138):
    // an ambiguous seq-domain layout must fail closed, not index stale rows.
    ensure_unambiguous_sst_order(files.iter().map(|(_, path)| path.as_path()))?;
    files.sort_by(|(left_order, left_path), (right_order, right_path)| {
        left_order
            .cmp(right_order)
            .then_with(|| left_path.cmp(right_path))
    });
    Ok(files.into_iter().map(|(_, path)| path).collect())
}
