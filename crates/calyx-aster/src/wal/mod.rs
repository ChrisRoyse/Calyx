//! Write-ahead log storage for Aster.

mod batch;
mod record;
mod segment;

use calyx_core::{CalyxError, CalyxErrorCode, Result};
use record::DecodeStatus;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use batch::GroupCommitBatcher;

/// Default group-commit window for PH05.
pub const DEFAULT_GROUP_COMMIT_WINDOW: Duration = Duration::from_millis(2);

/// WAL writer configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalOptions {
    /// Maximum bytes in one segment before the next append rotates.
    pub max_segment_bytes: u64,
    /// Upper bound for coalescing near-following requests into one fsync.
    pub group_commit_window: Duration,
}

impl Default for WalOptions {
    fn default() -> Self {
        Self {
            max_segment_bytes: 64 * 1024 * 1024,
            group_commit_window: DEFAULT_GROUP_COMMIT_WINDOW,
        }
    }
}

/// Fsync-backed append acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendAck {
    pub seq: u64,
    pub segment_path: PathBuf,
    pub start_offset: u64,
    pub end_offset: u64,
}

/// A replayed WAL record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayRecord {
    pub seq: u64,
    pub payload: Vec<u8>,
    pub segment_path: PathBuf,
    pub start_offset: u64,
    pub end_offset: u64,
}

/// Torn WAL tail discarded during replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TornTail {
    pub segment_path: PathBuf,
    pub offset: u64,
    pub code: &'static str,
    pub message: String,
}

impl TornTail {
    /// Converts the recovery observation to the catalogued Calyx error.
    pub fn error(&self) -> CalyxError {
        CalyxErrorCode::AsterTornWal.error(format!(
            "{} at byte {}: {}",
            self.segment_path.display(),
            self.offset,
            self.message
        ))
    }
}

/// WAL replay result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayOutcome {
    pub records: Vec<ReplayRecord>,
    pub torn_tail: Option<TornTail>,
}

/// Durable WAL writer.
#[derive(Debug)]
pub struct Wal {
    dir: PathBuf,
    options: WalOptions,
    active_index: u64,
    file: File,
    next_seq: u64,
}

impl Wal {
    /// Opens a WAL directory, replaying and truncating any torn tail first.
    pub fn open(dir: impl AsRef<Path>, options: WalOptions) -> Result<Self> {
        batch::validate_window(options.group_commit_window)?;
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir).map_err(|error| storage_error("create WAL directory", error))?;
        let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
        let replay = replay_dir_locked(&dir)?;
        let next_seq = replay.records.last().map_or(1, |record| record.seq + 1);
        let segments =
            segment::list_segments(&dir).map_err(|error| storage_error("list WAL", error))?;
        let active_index = segments.last().map_or(0, |(index, _)| *index);
        let active_path = segment::segment_path(&dir, active_index);
        let file = open_append_file(&active_path)?;

        Ok(Self {
            dir,
            options,
            active_index,
            file,
            next_seq,
        })
    }

    /// Appends one record and fsyncs it.
    pub fn append(&mut self, payload: &[u8]) -> Result<AppendAck> {
        let mut acks = self.append_batch(&[payload])?;
        Ok(acks.remove(0))
    }

    /// Appends a batch and fsyncs once before acknowledging the records.
    pub fn append_batch(&mut self, payloads: &[&[u8]]) -> Result<Vec<AppendAck>> {
        if payloads.is_empty() {
            return Ok(Vec::new());
        }

        let _lock = crate::file_lock::FileLockGuard::acquire(&self.dir.join(".append.lock"))?;
        self.refresh_after_external_appends()?;
        let mut acks = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let seq = self.next_seq;
            let bytes = record::encode(seq, payload)
                .map_err(|error| storage_error("encode WAL record", error))?;
            self.rotate_if_needed(bytes.len() as u64)?;
            let start_offset = self.seek_end()?;
            self.file
                .write_all(&bytes)
                .map_err(|error| storage_error("append WAL record", error))?;
            let end_offset = start_offset + bytes.len() as u64;
            acks.push(AppendAck {
                seq,
                segment_path: self.active_path(),
                start_offset,
                end_offset,
            });
            self.next_seq += 1;
        }

        self.file
            .sync_data()
            .map_err(|error| storage_error("fsync WAL batch", error))?;
        Ok(acks)
    }

    fn refresh_after_external_appends(&mut self) -> Result<()> {
        let replay = replay_dir_locked(&self.dir)?;
        self.next_seq = replay.records.last().map_or(1, |record| record.seq + 1);
        let segments =
            segment::list_segments(&self.dir).map_err(|error| storage_error("list WAL", error))?;
        let active_index = segments.last().map_or(0, |(index, _)| *index);
        if active_index != self.active_index {
            self.active_index = active_index;
            self.file = open_append_file(&self.active_path())?;
        }
        Ok(())
    }

    fn rotate_if_needed(&mut self, incoming_bytes: u64) -> Result<()> {
        let offset = self.seek_end()?;
        if offset == 0 || offset + incoming_bytes <= self.options.max_segment_bytes {
            return Ok(());
        }

        self.file
            .sync_all()
            .map_err(|error| storage_error("fsync WAL segment before rotation", error))?;
        self.active_index += 1;
        self.file = open_append_file(&self.active_path())?;
        Ok(())
    }

    fn seek_end(&mut self) -> Result<u64> {
        self.file
            .seek(SeekFrom::End(0))
            .map_err(|error| storage_error("seek WAL segment", error))
    }

    fn active_path(&self) -> PathBuf {
        segment::segment_path(&self.dir, self.active_index)
    }
}

/// Replays a WAL directory, truncating the first torn segment tail if present.
pub fn replay_dir(dir: impl AsRef<Path>) -> Result<ReplayOutcome> {
    let dir = dir.as_ref();
    let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
    replay_dir_locked(dir)
}

fn replay_dir_locked(dir: &Path) -> Result<ReplayOutcome> {
    let segments = segment::list_segments(dir).map_err(|error| storage_error("list WAL", error))?;
    let mut records = Vec::new();

    for (position, (_, path)) in segments.iter().enumerate() {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| storage_error("open WAL segment for replay", error))?;
        let mut offset = 0;

        loop {
            match record::decode_at(&mut file, offset)
                .map_err(|error| storage_error("decode WAL record", error))?
            {
                DecodeStatus::Complete(decoded) => {
                    offset = decoded.end_offset;
                    records.push(ReplayRecord {
                        seq: decoded.seq,
                        payload: decoded.payload,
                        segment_path: path.clone(),
                        start_offset: decoded.start_offset,
                        end_offset: decoded.end_offset,
                    });
                }
                DecodeStatus::Eof => break,
                DecodeStatus::Torn { offset, message } => {
                    file.set_len(offset)
                        .map_err(|error| storage_error("truncate torn WAL tail", error))?;
                    file.sync_data()
                        .map_err(|error| storage_error("fsync truncated WAL tail", error))?;
                    remove_later_segments(&segments[position + 1..])?;
                    return Ok(ReplayOutcome {
                        records,
                        torn_tail: Some(TornTail {
                            segment_path: path.clone(),
                            offset,
                            code: CalyxErrorCode::AsterTornWal.code(),
                            message,
                        }),
                    });
                }
            }
        }
    }

    Ok(ReplayOutcome {
        records,
        torn_tail: None,
    })
}

fn open_append_file(path: &Path) -> Result<File> {
    let existed = path.exists();
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| storage_error("open WAL segment", error))?;
    if !existed {
        sync_parent(path)?;
    }
    Ok(file)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure("WAL segment path has no parent"))?;
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| storage_error("fsync WAL directory", error))
}

#[cfg(not(unix))]
fn sync_parent(path: &Path) -> Result<()> {
    path.parent()
        .ok_or_else(|| CalyxError::disk_pressure("WAL segment path has no parent"))?;
    Ok(())
}

fn remove_later_segments(segments: &[(u64, PathBuf)]) -> Result<()> {
    for (_, path) in segments {
        fs::remove_file(path)
            .map_err(|error| storage_error("remove discarded WAL segment", error))?;
    }
    Ok(())
}

fn storage_error(context: &str, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests;
