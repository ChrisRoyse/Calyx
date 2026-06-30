use std::fs::OpenOptions;

use calyx_core::{CalyxErrorCode, Result};

use super::record::DecodeStatus;
use super::{ReplayRecord, TornTail, record, segment, storage_error};

pub(crate) fn stream_records(
    dir: impl AsRef<std::path::Path>,
    mut visit: impl FnMut(ReplayRecord) -> Result<()>,
) -> Result<usize> {
    let dir = dir.as_ref();
    let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
    let segments = segment::list_segments(dir)?;
    let mut count = 0;
    for (_, path) in segments {
        let mut file = OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|error| storage_error("open WAL segment for stream replay", error))?;
        let mut offset = 0;
        loop {
            match record::decode_at(&mut file, offset)
                .map_err(|error| storage_error("decode WAL record", error))?
            {
                DecodeStatus::Complete(decoded) => {
                    offset = decoded.end_offset;
                    count += 1;
                    visit(ReplayRecord {
                        seq: decoded.seq,
                        payload: decoded.payload,
                        segment_path: path.clone(),
                        start_offset: decoded.start_offset,
                        end_offset: decoded.end_offset,
                    })?;
                }
                DecodeStatus::Eof => break,
                DecodeStatus::Torn { offset, message } => {
                    return Err(TornTail {
                        segment_path: path.clone(),
                        offset,
                        code: CalyxErrorCode::AsterTornWal.code(),
                        message,
                    }
                    .error());
                }
            }
        }
    }
    Ok(count)
}
