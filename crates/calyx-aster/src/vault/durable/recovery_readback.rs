use super::super::encode::WriteRow;
use super::{RecoveredBatch, storage_error};
use crate::compaction::TieringPolicy;
use crate::sst::SstReader;
use crate::storage_names::{SstName, classify_sst, parse_cf_dir_name};
use calyx_core::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn read_manifested_batches(
    root: &Path,
    tiering_policy: Option<&TieringPolicy>,
    durable_seq: u64,
) -> Result<Vec<RecoveredBatch>> {
    let mut by_seq = BTreeMap::<u64, Vec<(usize, WriteRow)>>::new();
    if durable_seq == 0 {
        return Ok(Vec::new());
    }
    for cf_root in tiered_cf_roots(root, tiering_policy) {
        if !cf_root.exists() {
            continue;
        }
        for entry in fs::read_dir(&cf_root).map_err(|error| storage_error("read CF root", error))? {
            let cf_dir = entry.map_err(|error| storage_error("read CF entry", error))?;
            if !cf_dir
                .file_type()
                .map_err(|error| storage_error("stat CF entry", error))?
                .is_dir()
            {
                continue;
            }
            let cf_name = cf_dir.file_name().to_string_lossy().to_string();
            let cf = parse_cf_dir_name(&cf_name)?;
            for file in
                fs::read_dir(cf_dir.path()).map_err(|error| storage_error("read CF dir", error))?
            {
                let path = file
                    .map_err(|error| storage_error("read SST entry", error))?
                    .path();
                let Some(name) = classify_sst(&path)? else {
                    continue;
                };
                let (seq, index) = match name {
                    SstName::DurableBatch { seq, index } => (seq, index),
                    SstName::Compacted { seq } => (seq, 0),
                    // Router memtable flushes are recovered by
                    // `CfRouter::load_existing`, not by durable readback.
                    SstName::Router { .. } => continue,
                };
                if seq > durable_seq {
                    continue;
                }
                let reader = SstReader::open(&path)?;
                for (row_offset, row) in reader.iter()?.into_iter().enumerate() {
                    by_seq.entry(seq).or_default().push((
                        index + row_offset,
                        WriteRow {
                            cf,
                            key: row.key,
                            value: row.value,
                        },
                    ));
                }
            }
        }
    }

    Ok(by_seq
        .into_iter()
        .map(|(seq, mut rows)| {
            rows.sort_by_key(|(index, _)| *index);
            RecoveredBatch {
                seq,
                rows: rows.into_iter().map(|(_, row)| row).collect(),
            }
        })
        .collect())
}

pub(in crate::vault) fn tiered_cf_roots(
    root: &Path,
    tiering_policy: Option<&TieringPolicy>,
) -> Vec<PathBuf> {
    let mut roots = vec![root.join("cf")];
    if let Some(policy) = tiering_policy {
        for tier_root in policy.tier_roots() {
            let cf_root = tier_root.join("cf");
            if !roots.contains(&cf_root) {
                roots.push(cf_root);
            }
        }
    }
    roots
}
