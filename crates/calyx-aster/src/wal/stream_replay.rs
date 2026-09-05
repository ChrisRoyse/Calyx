use std::fs::OpenOptions;
use std::path::Path;

use calyx_core::{CalyxErrorCode, Result};

use super::record::{DecodeInto, DecodeStatus, SpanStatus};
use super::{ReplayRecord, TornTail, record, segment, storage_error};

/// Streams every WAL record's payload above `replay_floor_seq` WITHOUT
/// allocating per record: one scratch buffer is reused for every decoded
/// payload, so a scan over a large WAL stays O(largest record) resident instead
/// of churning one owned `Vec` per record — the churn bloated the glibc arena
/// to ~O(total WAL bytes) and OOM-killed reader/`wal-prune` opens (issue #1992).
///
/// Records at or below the floor are header-skipped without reading their
/// payload. `visit` receives each record's `(seq, payload)` borrowing the
/// shared buffer and returns `Ok(false)` to stop the scan early.
pub(crate) fn for_each_record_payload_after(
    dir: impl AsRef<Path>,
    replay_floor_seq: u64,
    mut visit: impl FnMut(u64, &[u8]) -> Result<bool>,
) -> Result<()> {
    let dir = dir.as_ref();
    let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
    let segments = segment::list_segments(dir)?;
    let mut buf = Vec::new();
    for (_, path) in segments {
        let mut file = OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|error| storage_error("open WAL segment for stream replay", error))?;
        let mut offset = 0;
        loop {
            let span = match record::read_logical_span_at(&mut file, offset)
                .map_err(|error| storage_error("decode WAL header", error))?
            {
                SpanStatus::Complete(span) => span,
                SpanStatus::Eof => break,
                SpanStatus::Torn { offset, message } => {
                    return Err(torn_error(&path, offset, message));
                }
            };
            if span.seq <= replay_floor_seq {
                offset = span.end_offset;
                continue;
            }
            match record::decode_into(&mut file, offset, &mut buf)
                .map_err(|error| storage_error("decode WAL record", error))?
            {
                DecodeInto::Complete {
                    seq, end_offset, ..
                } => {
                    offset = end_offset;
                    if !visit(seq, &buf)? {
                        return Ok(());
                    }
                }
                DecodeInto::Eof => break,
                DecodeInto::Torn { offset, message } => {
                    return Err(torn_error(&path, offset, message));
                }
            }
        }
    }
    Ok(())
}

/// Like [`for_each_record_payload_after`] but visits segments newest-first
/// (floor 0, full decode) and stops as soon as `visit` returns `Ok(false)`.
///
/// The ledger point-read retained-WAL fallback wants a bounded set of recent
/// ledger rows, which live in the newest commits; a newest-first scan resolves
/// them after touching only the last segment(s) instead of decoding the whole
/// WAL from seq 0 (issue #1992). Records within a segment are still visited in
/// on-disk (ascending-offset) order.
pub(crate) fn for_each_record_payload_reverse(
    dir: impl AsRef<Path>,
    mut visit: impl FnMut(u64, &[u8]) -> Result<bool>,
) -> Result<()> {
    let dir = dir.as_ref();
    let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
    let segments = segment::list_segments(dir)?;
    let mut buf = Vec::new();
    for (_, path) in segments.iter().rev() {
        let mut file = OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|error| storage_error("open WAL segment for stream replay", error))?;
        let mut offset = 0;
        loop {
            match record::decode_into(&mut file, offset, &mut buf)
                .map_err(|error| storage_error("decode WAL record", error))?
            {
                DecodeInto::Complete {
                    seq, end_offset, ..
                } => {
                    offset = end_offset;
                    if !visit(seq, &buf)? {
                        return Ok(());
                    }
                }
                DecodeInto::Eof => break,
                DecodeInto::Torn { offset, message } => {
                    return Err(torn_error(path, offset, message));
                }
            }
        }
    }
    Ok(())
}

fn torn_error(path: &Path, offset: u64, message: String) -> calyx_core::CalyxError {
    TornTail {
        segment_path: path.to_path_buf(),
        offset,
        code: CalyxErrorCode::AsterTornWal.code(),
        message,
    }
    .error()
}

/// Streams only records newer than durable SST coverage. Records at or below
/// the floor are checked for canonical framing and skipped by their encoded
/// length without reading vector-heavy payloads.
pub(crate) fn stream_records_after(
    dir: impl AsRef<std::path::Path>,
    replay_floor_seq: u64,
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
            let span = match record::read_logical_span_at(&mut file, offset)
                .map_err(|error| storage_error("decode WAL header", error))?
            {
                SpanStatus::Complete(span) => span,
                SpanStatus::Eof => break,
                SpanStatus::Torn { offset, message } => {
                    return Err(TornTail {
                        segment_path: path.clone(),
                        offset,
                        code: CalyxErrorCode::AsterTornWal.code(),
                        message,
                    }
                    .error());
                }
            };
            if span.seq <= replay_floor_seq {
                offset = span.end_offset;
                continue;
            }
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
                        fragmented: decoded.fragmented,
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
