use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bincode::config;
use calyx_core::{CalyxError, Result};

use super::{LedgerQueryIndex, hex};

const MAGIC: &[u8] = b"calyx_ledger_query_index_v1\0";
const DIRECTORY: &str = "ledger_query_index";
const SUFFIX: &str = ".idx";
const MAX_FILE_BYTES: u64 = 1 << 30;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub(super) fn newest_previous_index(vault: &Path, height: u64) -> Result<Option<PathBuf>> {
    let dir = vault.join(DIRECTORY);
    if !dir.exists() {
        return Ok(None);
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|error| {
        CalyxError::disk_pressure(format!(
            "read ledger query index directory {}: {error}",
            dir.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            CalyxError::disk_pressure(format!("read ledger query index entry: {error}"))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            CalyxError::disk_pressure(format!("stat ledger query index entry: {error}"))
        })?;
        if !file_type.is_file() {
            return Err(CalyxError::ledger_corrupt(format!(
                "ledger query index directory contains non-file {}",
                entry.path().display()
            )));
        }
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            CalyxError::ledger_corrupt("ledger query index filename is not valid UTF-8")
        })?;
        if name.starts_with(".ledger-query-index-") && name.ends_with(".tmp") {
            return Err(CalyxError::ledger_corrupt(format!(
                "ledger query index has an unpublished temp file {}",
                entry.path().display()
            )));
        }
        let raw_height = name
            .strip_suffix(SUFFIX)
            .and_then(|name| name.get(..20))
            .ok_or_else(|| {
                CalyxError::ledger_corrupt(format!(
                    "unrecognized ledger query index filename {name}"
                ))
            })?;
        let candidate_height = raw_height.parse::<u64>().map_err(|error| {
            CalyxError::ledger_corrupt(format!(
                "invalid ledger query index height in {name}: {error}"
            ))
        })?;
        if candidate_height < height {
            candidates.push((candidate_height, entry.path()));
        }
    }
    candidates.sort_by_key(|candidate| candidate.0);
    Ok(candidates.pop().map(|candidate| candidate.1))
}

pub(super) fn index_path(vault: &Path, height: u64, tip_hash: &[u8; 32]) -> PathBuf {
    vault
        .join(DIRECTORY)
        .join(format!("{height:020}-{}{}", hex(tip_hash), SUFFIX))
}

pub(super) fn write_index(path: &Path, index: &LedgerQueryIndex) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        CalyxError::disk_pressure("ledger query index path has no parent directory")
    })?;
    crate::private_fs::create_private_dir_all(parent)?;
    let payload = bincode::serde::encode_to_vec(index, config::standard()).map_err(|error| {
        CalyxError::ledger_corrupt(format!("encode ledger query index: {error}"))
    })?;
    let mut bytes = Vec::with_capacity(MAGIC.len() + 8 + payload.len() + 32);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(blake3::hash(&payload).as_bytes());
    let temp = parent.join(format!(
        ".ledger-query-index-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let mut file = crate::private_fs::open_private_temp_file(&temp)?;
        file.write_all(&bytes).map_err(|error| {
            CalyxError::disk_pressure(format!(
                "write ledger query index temp {}: {error}",
                temp.display()
            ))
        })?;
        file.sync_all().map_err(|error| {
            CalyxError::disk_pressure(format!(
                "sync ledger query index temp {}: {error}",
                temp.display()
            ))
        })?;
        drop(file);
        fs::rename(&temp, path).map_err(|error| {
            CalyxError::disk_pressure(format!(
                "publish ledger query index {} -> {}: {error}",
                temp.display(),
                path.display()
            ))
        })?;
        crate::fsync::sync_parent(path, "ledger query index")?;
        remove_old_generations(parent, path)
    })();
    if result.is_err()
        && temp.exists()
        && let Err(cleanup) = fs::remove_file(&temp)
    {
        tracing::error!(
            path = %temp.display(),
            error = %cleanup,
            "failed to remove unpublished ledger query index temp file"
        );
    }
    result
}

fn remove_old_generations(directory: &Path, keep: &Path) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|error| {
        CalyxError::disk_pressure(format!(
            "read ledger query index directory for cleanup {}: {error}",
            directory.display()
        ))
    })? {
        let path = entry
            .map_err(|error| {
                CalyxError::disk_pressure(format!("read ledger query index cleanup entry: {error}"))
            })?
            .path();
        if path != keep && path.extension().is_some_and(|extension| extension == "idx") {
            fs::remove_file(&path).map_err(|error| {
                CalyxError::disk_pressure(format!(
                    "remove stale ledger query index {}: {error}",
                    path.display()
                ))
            })?;
        }
    }
    crate::fsync::sync_dir(directory, "ledger query index cleanup")
}

pub(super) fn read_index(path: &Path) -> Result<LedgerQueryIndex> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CalyxError::disk_pressure(format!(
            "stat ledger query index {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(CalyxError::ledger_corrupt(format!(
            "ledger query index {} is not a bounded regular file (bytes={})",
            path.display(),
            metadata.len()
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|error| {
            CalyxError::disk_pressure(format!(
                "read ledger query index {}: {error}",
                path.display()
            ))
        })?;
    let header_len = MAGIC.len() + 8;
    if bytes.len() < header_len + 32 || &bytes[..MAGIC.len()] != MAGIC {
        return Err(CalyxError::ledger_corrupt(format!(
            "ledger query index {} has an invalid header",
            path.display()
        )));
    }
    let length = u64::from_be_bytes(
        bytes[MAGIC.len()..header_len]
            .try_into()
            .expect("eight-byte length slice"),
    );
    let length = usize::try_from(length).map_err(|_| {
        CalyxError::ledger_corrupt("ledger query index payload length exceeds usize")
    })?;
    if bytes.len() != header_len + length + 32 {
        return Err(CalyxError::ledger_corrupt(format!(
            "ledger query index {} length mismatch",
            path.display()
        )));
    }
    let payload = &bytes[header_len..header_len + length];
    let checksum = &bytes[header_len + length..];
    if blake3::hash(payload).as_bytes() != checksum {
        return Err(CalyxError::ledger_corrupt(format!(
            "ledger query index {} checksum mismatch",
            path.display()
        )));
    }
    let (index, consumed): (LedgerQueryIndex, usize) =
        bincode::serde::decode_from_slice(payload, config::standard()).map_err(|error| {
            CalyxError::ledger_corrupt(format!(
                "decode ledger query index {}: {error}",
                path.display()
            ))
        })?;
    if consumed != payload.len() {
        return Err(CalyxError::ledger_corrupt(format!(
            "ledger query index {} has trailing payload bytes",
            path.display()
        )));
    }
    index.validate_internal()?;
    Ok(index)
}
