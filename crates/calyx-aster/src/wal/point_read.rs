use std::fs::OpenOptions;
use std::path::Path;

use calyx_core::{CalyxError, CalyxErrorCode, Result};

use super::record::DecodeStatus;
use super::{ReplayRecord, TornTail, record, storage_error};

/// Reads and validates one physical WAL record by exact segment and byte range.
pub(crate) fn read_record_at(
    segment_path: impl AsRef<Path>,
    seq: u64,
    start_offset: u64,
    end_offset: u64,
) -> Result<ReplayRecord> {
    let path = segment_path.as_ref();
    let dir = path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure("WAL segment path has no parent"))?;
    let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| storage_error("open WAL segment for point read", error))?;
    match record::decode_at(&mut file, start_offset)
        .map_err(|error| storage_error("decode WAL record", error))?
    {
        DecodeStatus::Complete(decoded) => {
            if decoded.seq != seq
                || decoded.start_offset != start_offset
                || decoded.end_offset != end_offset
            {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "WAL record {} decoded as seq {} range {}..{} instead of seq {seq} range {start_offset}..{end_offset}",
                    path.display(),
                    decoded.seq,
                    decoded.start_offset,
                    decoded.end_offset
                )));
            }
            Ok(ReplayRecord {
                seq: decoded.seq,
                payload: decoded.payload,
                segment_path: path.to_path_buf(),
                start_offset: decoded.start_offset,
                end_offset: decoded.end_offset,
            })
        }
        DecodeStatus::Eof => Err(CalyxError::aster_corrupt_shard(format!(
            "WAL record {seq} at {}:{}..{} is beyond EOF",
            path.display(),
            start_offset,
            end_offset
        ))),
        DecodeStatus::Torn { offset, message } => Err(TornTail {
            segment_path: path.to_path_buf(),
            offset,
            code: CalyxErrorCode::AsterTornWal.code(),
            message,
        }
        .error()),
    }
}
