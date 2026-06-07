use super::encode::{WriteRow, decode_write_batch, encode_write_batch};
use crate::manifest::{ImmutableRef, ManifestStore, VaultManifest};
use crate::sst::write_sst;
use crate::wal::{GroupCommitBatcher, WalOptions, replay_dir};
use calyx_core::{CalyxError, Result, SystemClock};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct VaultOptions {
    pub wal_options: WalOptions,
}

#[derive(Debug)]
pub(super) struct DurableVault {
    root: PathBuf,
    batcher: GroupCommitBatcher,
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

    pub(super) fn replay_batches(root: impl AsRef<Path>) -> Result<Vec<Vec<WriteRow>>> {
        let replay = replay_dir(root.as_ref().join("wal"))?;
        replay
            .records
            .iter()
            .map(|record| decode_write_batch(&record.payload))
            .collect()
    }

    pub(super) fn write_batch(&self, rows: &[WriteRow]) -> Result<u64> {
        let payload = encode_write_batch(rows)?;
        let ack = self.batcher.submit(payload)?;
        self.write_rows(ack.seq, rows)?;
        self.write_manifest(ack.seq)?;
        Ok(ack.seq)
    }

    pub(super) fn flush(&self) -> Result<()> {
        self.batcher.flush_sync()
    }

    fn write_rows(&self, seq: u64, rows: &[WriteRow]) -> Result<()> {
        for (index, row) in rows.iter().enumerate() {
            let dir = self.root.join("cf").join(row.cf.name());
            fs::create_dir_all(&dir).map_err(|error| storage_error("create CF dir", error))?;
            let path = dir.join(format!("{seq:020}-{index:04}.sst"));
            write_sst(&path, [(row.key.as_slice(), row.value.as_slice())])?;
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
