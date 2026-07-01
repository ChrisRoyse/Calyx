use super::insert_ledger_bytes;
use crate::cf::ledger_key;
use crate::sst::level::SstLevel;
use crate::sst::{SstLookupMetadata, SstReader};
use crate::storage_names::{SstName, classify_sst, sst_order_key};
use calyx_core::{CalyxError, Result as CalyxResult};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(super) fn read_sst_ledger_rows(
    ledger_dirs: &[PathBuf],
    wanted: &BTreeSet<u64>,
    rows: &mut BTreeMap<u64, Vec<u8>>,
) -> CalyxResult<()> {
    read_sst_ledger_rows_from_candidates(
        ledger_dirs,
        wanted,
        rows,
        probable_ledger_sst_candidates,
    )?;
    let unresolved = unresolved_seqs(wanted, rows);
    if !unresolved.is_empty() {
        read_sst_ledger_rows_from_candidates(
            ledger_dirs,
            &unresolved,
            rows,
            key_range_ledger_sst_candidates,
        )?;
    }
    let unresolved = unresolved_seqs(wanted, rows);
    if !unresolved.is_empty() {
        read_sst_ledger_rows_from_candidates(
            ledger_dirs,
            &unresolved,
            rows,
            named_ledger_sst_candidates,
        )?;
    }
    let unresolved = unresolved_seqs(wanted, rows);
    if !unresolved.is_empty() {
        read_sst_ledger_rows_from_candidates(
            ledger_dirs,
            &unresolved,
            rows,
            complete_ledger_sst_candidates,
        )?;
    }
    Ok(())
}

pub(super) fn unresolved_seqs(
    wanted: &BTreeSet<u64>,
    rows: &BTreeMap<u64, Vec<u8>>,
) -> BTreeSet<u64> {
    wanted
        .iter()
        .copied()
        .filter(|seq| !rows.contains_key(seq))
        .collect()
}

fn read_sst_ledger_rows_from_candidates(
    ledger_dirs: &[PathBuf],
    wanted: &BTreeSet<u64>,
    rows: &mut BTreeMap<u64, Vec<u8>>,
    candidates: fn(&[PathBuf], &BTreeSet<u64>) -> CalyxResult<Vec<PathBuf>>,
) -> CalyxResult<()> {
    let level = SstLevel::from_oldest_first_with_lookup(candidates(ledger_dirs, wanted)?)?;
    for seq in wanted {
        let key = ledger_key(*seq);
        for value in level.values_for_key(&key)? {
            insert_ledger_bytes(rows, *seq, value)?;
        }
    }
    Ok(())
}

fn probable_ledger_sst_candidates(
    ledger_dirs: &[PathBuf],
    wanted: &BTreeSet<u64>,
) -> CalyxResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for dir in ledger_dirs {
        for seq in wanted {
            push_ledger_sst_candidate(&dir.join(format!("{seq:020}.sst")), &mut files)?;
            push_ledger_sst_candidate(&dir.join(format!("{seq:020}-0000.sst")), &mut files)?;
        }
    }
    sorted_unique_paths(files)
}

fn named_ledger_sst_candidates(
    ledger_dirs: &[PathBuf],
    wanted: &BTreeSet<u64>,
) -> CalyxResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for dir in ledger_dirs {
        if !dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(dir).map_err(|error| {
            CalyxError::disk_pressure(format!("read ledger CF dir {}: {error}", dir.display()))
        })? {
            let path = entry
                .map_err(|error| {
                    CalyxError::disk_pressure(format!("read ledger SST entry: {error}"))
                })?
                .path();
            let Some(name) = classify_sst(&path)? else {
                continue;
            };
            let seq = match name {
                SstName::Router { seq } | SstName::DurableBatch { seq, .. } => seq,
                SstName::Compacted { .. } => continue,
            };
            if !wanted.contains(&seq) {
                continue;
            }
            let order = sst_order_key(&path)?.ok_or_else(|| {
                CalyxError::aster_corrupt_shard(format!(
                    "classified ledger SST {} has no order key",
                    path.display()
                ))
            })?;
            files.push((order, path));
        }
    }
    sorted_unique_paths(files)
}

fn key_range_ledger_sst_candidates(
    ledger_dirs: &[PathBuf],
    wanted: &BTreeSet<u64>,
) -> CalyxResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for dir in ledger_dirs {
        for seq in wanted {
            if let Some(path) = primary_ledger_sst_by_key_range(dir, *seq)? {
                push_ledger_sst_candidate(&path, &mut files)?;
            }
        }
    }
    sorted_unique_paths(files)
}

fn primary_ledger_sst_by_key_range(dir: &Path, seq: u64) -> CalyxResult<Option<PathBuf>> {
    let key = ledger_key(seq);
    let mut low = 0_u64;
    let mut high = seq;
    let mut candidate = None;
    while low <= high {
        let mid = low + (high - low) / 2;
        let path = dir.join(format!("{mid:020}-0000.sst"));
        if !path.try_exists().map_err(|error| {
            CalyxError::disk_pressure(format!("stat {}: {error}", path.display()))
        })? {
            if mid == 0 {
                break;
            }
            high = mid - 1;
            continue;
        }
        let lookup = ledger_sst_lookup_metadata(&path)?;
        if lookup.first_key.as_slice() <= key.as_slice() {
            candidate = Some((path, lookup));
            low = mid.saturating_add(1);
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    let Some((path, lookup)) = candidate else {
        return Ok(None);
    };
    if key.as_slice() >= lookup.first_key.as_slice() && key.as_slice() <= lookup.last_key.as_slice()
    {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

fn ledger_sst_lookup_metadata(path: &Path) -> CalyxResult<SstLookupMetadata> {
    SstReader::open(path)?.lookup_metadata().ok_or_else(|| {
        CalyxError::aster_corrupt_shard(format!("ledger SST {} has no keys", path.display()))
    })
}

fn sorted_unique_paths(
    mut files: Vec<(crate::storage_names::SstOrderKey, PathBuf)>,
) -> CalyxResult<Vec<PathBuf>> {
    files.sort_by(|(left_order, left_path), (right_order, right_path)| {
        left_order
            .cmp(right_order)
            .then_with(|| left_path.cmp(right_path))
    });
    let mut paths = files.into_iter().map(|(_, path)| path).collect::<Vec<_>>();
    paths.dedup();
    Ok(paths)
}

fn complete_ledger_sst_candidates(
    ledger_dirs: &[PathBuf],
    _wanted: &BTreeSet<u64>,
) -> CalyxResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for dir in ledger_dirs {
        if !dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(dir).map_err(|error| {
            CalyxError::disk_pressure(format!("read ledger CF dir {}: {error}", dir.display()))
        })? {
            let path = entry
                .map_err(|error| {
                    CalyxError::disk_pressure(format!("read ledger SST entry: {error}"))
                })?
                .path();
            let Some(_) = classify_sst(&path)? else {
                continue;
            };
            let order = sst_order_key(&path)?.ok_or_else(|| {
                CalyxError::aster_corrupt_shard(format!(
                    "classified ledger SST {} has no order key",
                    path.display()
                ))
            })?;
            files.push((order, path));
        }
    }
    sorted_unique_paths(files)
}

fn push_ledger_sst_candidate(
    path: &Path,
    files: &mut Vec<(crate::storage_names::SstOrderKey, PathBuf)>,
) -> CalyxResult<()> {
    if !path
        .try_exists()
        .map_err(|error| CalyxError::disk_pressure(format!("stat {}: {error}", path.display())))?
    {
        return Ok(());
    }
    let Some(name) = classify_sst(path)? else {
        return Ok(());
    };
    match name {
        SstName::Router { .. } | SstName::DurableBatch { .. } => {}
        SstName::Compacted { .. } => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "targeted ledger point read reached compacted ledger SST {}; add a compacted ledger row index before using point verification on compacted ledger layouts",
                path.display()
            )));
        }
    }
    let order = sst_order_key(path)?.ok_or_else(|| {
        CalyxError::aster_corrupt_shard(format!(
            "classified ledger SST {} has no order key",
            path.display()
        ))
    })?;
    files.push((order, path.to_path_buf()));
    Ok(())
}
