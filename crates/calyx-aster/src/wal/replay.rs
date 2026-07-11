use super::record::DecodeStatus;
use super::{ReplayOutcome, ReplayRecord, TornTail, record, segment, storage_error};
use calyx_core::{CalyxErrorCode, Result};
use std::fs::{File, OpenOptions};
use std::path::Path;

/// Replays a WAL directory, truncating a torn physical tail if present.
pub fn replay_dir(dir: impl AsRef<Path>) -> Result<ReplayOutcome> {
    replay_dir_after(dir, 0)
}

/// Replays WAL records after a durable checkpoint sequence.
///
/// Records at or below `replay_floor_seq` are already represented by durable
/// SST/manifest state, so recovery validates their headers and seeks over their
/// payloads without re-reading and re-decoding old vector-heavy batches.
pub fn replay_dir_after(dir: impl AsRef<Path>, replay_floor_seq: u64) -> Result<ReplayOutcome> {
    let dir = dir.as_ref();
    let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
    replay_dir_locked_after(dir, replay_floor_seq)
}

pub(super) fn replay_dir_locked(dir: &Path) -> Result<ReplayOutcome> {
    replay_dir_locked_after(dir, 0)
}

pub(super) fn replay_dir_locked_after(dir: &Path, replay_floor_seq: u64) -> Result<ReplayOutcome> {
    let segments = segment::list_segments(dir)?;
    let mut records = Vec::new();

    for (position, (_, path)) in segments.iter().enumerate() {
        let has_later_segments = position + 1 < segments.len();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| storage_error("open WAL segment for replay", error))?;
        let mut offset = 0;

        loop {
            let header = match record::read_header_at(&mut file, offset)
                .map_err(|error| storage_error("decode WAL header", error))?
            {
                record::HeaderStatus::Complete(header) => header,
                record::HeaderStatus::Eof => break,
                record::HeaderStatus::Torn { offset, message } => {
                    return resolve_torn_tail(
                        &file,
                        path,
                        offset,
                        message,
                        records,
                        has_later_segments,
                    );
                }
            };
            if header.seq <= replay_floor_seq {
                offset = header.end_offset;
                continue;
            }
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
                    return resolve_torn_tail(
                        &file,
                        path,
                        offset,
                        message,
                        records,
                        has_later_segments,
                    );
                }
            }
        }
    }

    Ok(ReplayOutcome {
        records,
        torn_tail: None,
    })
}

fn resolve_torn_tail(
    file: &File,
    segment_path: &Path,
    offset: u64,
    message: String,
    records: Vec<ReplayRecord>,
    has_later_segments: bool,
) -> Result<ReplayOutcome> {
    let torn_tail = TornTail {
        segment_path: segment_path.to_path_buf(),
        offset,
        code: CalyxErrorCode::AsterTornWal.code(),
        message,
    };
    if has_later_segments {
        return Err(torn_tail.error());
    }

    file.set_len(offset)
        .map_err(|error| storage_error("truncate torn WAL tail", error))?;
    file.sync_data()
        .map_err(|error| storage_error("fsync truncated WAL tail", error))?;
    Ok(ReplayOutcome {
        records,
        torn_tail: Some(torn_tail),
    })
}
