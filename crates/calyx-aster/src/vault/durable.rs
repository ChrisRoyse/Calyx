use super::encode::{WriteRow, decode_write_batch, encode_write_batch};
use crate::cf::ColumnFamily;
use crate::manifest::{ImmutableRef, ManifestStore, VaultManifest, recover_vault};
use crate::sst::{SstReader, write_sst};
use crate::wal::{GroupCommitBatcher, WalOptions, replay_dir};
use calyx_core::{CalyxError, Result, SlotId, SystemClock};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct VaultOptions {
    pub wal_options: WalOptions,
    pub memtable_byte_cap: usize,
}

#[derive(Debug)]
pub(super) struct DurableVault {
    root: PathBuf,
    batcher: GroupCommitBatcher,
}

pub(super) struct RecoveredBatch {
    pub seq: u64,
    pub rows: Vec<WriteRow>,
}

pub(super) struct RecoveredBatches {
    pub batches: Vec<RecoveredBatch>,
    pub last_recovered_seq: u64,
}

impl DurableVault {
    pub(super) fn open(root: impl AsRef<Path>, options: &VaultOptions) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("cf"))
            .map_err(|error| storage_error("create durable CF root", error))?;
        let wal = crate::wal::Wal::open(root.join("wal"), options.wal_options)?;
        let batcher = GroupCommitBatcher::new(
            wal,
            options.wal_options.group_commit_window,
            Arc::new(SystemClock),
        )?;
        Ok(Self { root, batcher })
    }

    pub(super) fn recover_batches(root: impl AsRef<Path>) -> Result<RecoveredBatches> {
        let root = root.as_ref();
        if root.join("CURRENT").exists() {
            let recovery = recover_vault(root)?;
            let mut batches = read_manifested_batches(root, recovery.manifest.durable_seq)?;
            for record in recovery.wal_records {
                batches.push(RecoveredBatch {
                    seq: record.seq,
                    rows: decode_write_batch(&record.payload)?,
                });
            }
            return Ok(RecoveredBatches {
                batches,
                last_recovered_seq: recovery.last_recovered_seq,
            });
        }

        let replay = replay_dir(root.join("wal"))?;
        let last_recovered_seq = replay.records.last().map_or(0, |record| record.seq);
        let batches = replay
            .records
            .iter()
            .map(|record| {
                Ok(RecoveredBatch {
                    seq: record.seq,
                    rows: decode_write_batch(&record.payload)?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(RecoveredBatches {
            batches,
            last_recovered_seq,
        })
    }

    pub(super) fn append_batch(&self, rows: &[WriteRow]) -> Result<u64> {
        let payload = encode_write_batch(rows)?;
        let ack = self.batcher.submit(payload)?;
        Ok(ack.seq)
    }

    pub(super) fn checkpoint_batch(&self, seq: u64, rows: &[WriteRow]) -> Result<()> {
        self.write_rows(seq, rows)?;
        self.write_manifest(seq)
    }

    pub(super) fn flush(&self) -> Result<()> {
        self.batcher.flush_sync()
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    fn write_rows(&self, seq: u64, rows: &[WriteRow]) -> Result<()> {
        let mut by_cf = Vec::<(ColumnFamily, Vec<(usize, &WriteRow)>)>::new();
        for (index, row) in rows.iter().enumerate() {
            if let Some((_, group)) = by_cf.iter_mut().find(|(cf, _)| *cf == row.cf) {
                group.push((index, row));
            } else {
                by_cf.push((row.cf, vec![(index, row)]));
            }
        }
        by_cf.sort_by_key(|(cf, _)| cf.name());
        for (cf, mut rows) in by_cf {
            rows.sort_by(|(_, left), (_, right)| left.key.cmp(&right.key));
            let first_index = rows.first().map_or(0, |(index, _)| *index);
            let dir = self.root.join("cf").join(cf.name());
            fs::create_dir_all(&dir).map_err(|error| storage_error("create CF dir", error))?;
            let path = dir.join(format!("{seq:020}-{first_index:04}.sst"));
            let entries = rows
                .iter()
                .map(|(_, row)| (row.key.as_slice(), row.value.as_slice()));
            write_sst(&path, entries)?;
        }
        Ok(())
    }

    fn write_manifest(&self, seq: u64) -> Result<()> {
        let (panel_ref, codebook_refs) = ensure_manifest_assets(&self.root)?;
        let manifest = VaultManifest::new(seq, seq, panel_ref, codebook_refs)?;
        ManifestStore::open(&self.root).write_current(&manifest)?;
        Ok(())
    }
}

fn read_manifested_batches(root: &Path, durable_seq: u64) -> Result<Vec<RecoveredBatch>> {
    let mut by_seq = BTreeMap::<u64, Vec<(usize, WriteRow)>>::new();
    let cf_root = root.join("cf");
    if durable_seq == 0 || !cf_root.exists() {
        return Ok(Vec::new());
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
        let cf = parse_cf_name(&cf_name)?;
        for file in
            fs::read_dir(cf_dir.path()).map_err(|error| storage_error("read CF dir", error))?
        {
            let path = file
                .map_err(|error| storage_error("read SST entry", error))?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("sst") {
                continue;
            }
            let Some((seq, index)) = durable_sst_identity(&path) else {
                continue;
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

fn durable_sst_identity(path: &Path) -> Option<(u64, usize)> {
    let stem = path.file_stem()?.to_str()?;
    let (seq, index) = stem.split_once('-')?;
    Some((seq.parse().ok()?, index.parse().ok()?))
}

fn parse_cf_name(value: &str) -> Result<ColumnFamily> {
    match value {
        "base" => Ok(ColumnFamily::Base),
        "anchors" => Ok(ColumnFamily::Anchors),
        "ledger" => Ok(ColumnFamily::Ledger),
        "online" => Ok(ColumnFamily::Online),
        "scalars" => Ok(ColumnFamily::Scalars),
        "xterm" => Ok(ColumnFamily::XTerm),
        _ if value.starts_with("slot_") => parse_slot_cf(value),
        _ => Err(CalyxError::aster_corrupt_shard(format!(
            "unknown durable CF directory {value}"
        ))),
    }
}

fn parse_slot_cf(value: &str) -> Result<ColumnFamily> {
    let raw = value.ends_with(".raw");
    let slot_text = value.trim_start_matches("slot_").trim_end_matches(".raw");
    let slot = slot_text.parse::<u16>().map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("invalid slot CF directory {value}: {error}"))
    })?;
    if raw {
        Ok(ColumnFamily::slot_raw(SlotId::new(slot)))
    } else {
        Ok(ColumnFamily::slot(SlotId::new(slot)))
    }
}

fn ensure_manifest_assets(root: &Path) -> Result<(ImmutableRef, Vec<ImmutableRef>)> {
    let panel_path = root.join("panel/current.bin");
    let codebook_path = root.join("codebooks/default.bin");
    let panel_bytes = b"calyx-stage1-panel";
    let codebook_bytes = b"calyx-stage1-codebook";
    write_asset(&panel_path, panel_bytes)?;
    write_asset(&codebook_path, codebook_bytes)?;
    Ok((
        ImmutableRef::from_bytes("panel/current.bin", panel_bytes)?,
        vec![ImmutableRef::from_bytes(
            "codebooks/default.bin",
            codebook_bytes,
        )?],
    ))
}

fn write_asset(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| storage_error("create manifest asset dir", error))?;
    }
    {
        let mut file =
            File::create(path).map_err(|error| storage_error("create manifest asset", error))?;
        file.write_all(bytes)
            .map_err(|error| storage_error("write manifest asset", error))?;
        file.sync_all()
            .map_err(|error| storage_error("fsync manifest asset", error))?;
    }
    Ok(())
}

fn storage_error(context: &str, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}
