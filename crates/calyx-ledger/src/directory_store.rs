use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};

use crate::append::{LedgerCfStore, LedgerRow};
use crate::head_anchor::{LedgerHeadAnchor, read_anchor_file};

const ROW_EXT: &str = "ledger";

/// Disk-backed row store used for manual FSV before Aster group-commit wiring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryLedgerStore {
    root: PathBuf,
}

impl DirectoryLedgerStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .map_err(|error| CalyxError::disk_pressure(format!("create ledger CF dir: {error}")))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn row_path(&self, seq: u64) -> PathBuf {
        self.root.join(format!("{seq:016x}.{ROW_EXT}"))
    }

    fn anchor_path(&self) -> PathBuf {
        self.root.join("_head_anchor.json")
    }
}

impl LedgerCfStore for DirectoryLedgerStore {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        let mut rows = Vec::new();
        for entry in fs::read_dir(&self.root)
            .map_err(|error| CalyxError::disk_pressure(format!("read ledger CF dir: {error}")))?
        {
            let path = entry
                .map_err(|error| {
                    CalyxError::disk_pressure(format!("read ledger CF entry: {error}"))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some(ROW_EXT) {
                continue;
            }
            let seq = parse_row_seq(&path)?;
            let bytes = fs::read(&path)
                .map_err(|error| CalyxError::disk_pressure(format!("read ledger row: {error}")))?;
            rows.push(LedgerRow { seq, bytes });
        }
        rows.sort_by_key(|row| row.seq);
        Ok(rows)
    }

    fn put_new(&mut self, seq: u64, bytes: &[u8]) -> Result<()> {
        let path = self.row_path(seq);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| match error.kind() {
                io::ErrorKind::AlreadyExists => {
                    append_only_violation(format!("ledger row {} already exists", path.display()))
                }
                _ => CalyxError::disk_pressure(format!("create ledger row: {error}")),
            })?;
        file.write_all(bytes)
            .map_err(|error| CalyxError::disk_pressure(format!("write ledger row: {error}")))?;
        file.sync_all()
            .map_err(|error| CalyxError::disk_pressure(format!("sync ledger row: {error}")))?;
        Ok(())
    }

    fn head_anchor(&self) -> Result<Option<LedgerHeadAnchor>> {
        read_anchor_file(&self.anchor_path())
    }

    fn put_head_anchor(&mut self, anchor: &LedgerHeadAnchor) -> Result<()> {
        let path = self.anchor_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                CalyxError::disk_pressure(format!("create ledger head anchor dir: {error}"))
            })?;
        }
        if let Some(current) = read_anchor_file(&path)?
            && anchor.height < current.height
        {
            return Err(append_only_violation(format!(
                "ledger head anchor regressed from {} to {}",
                current.height, anchor.height
            )));
        }
        let bytes = serde_json::to_vec(anchor).map_err(|error| {
            CalyxError::ledger_corrupt(format!("encode ledger head anchor: {error}"))
        })?;
        fs::write(&path, bytes).map_err(|error| {
            CalyxError::disk_pressure(format!("write ledger head anchor: {error}"))
        })
    }
}

fn parse_row_seq(path: &Path) -> Result<u64> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CalyxError::ledger_corrupt("ledger row has invalid file name"))?;
    u64::from_str_radix(stem, 16)
        .map_err(|error| CalyxError::ledger_corrupt(format!("ledger row seq parse: {error}")))
}

fn append_only_violation(message: impl Into<String>) -> CalyxError {
    CalyxError::ledger_append_only_violation(message)
}
