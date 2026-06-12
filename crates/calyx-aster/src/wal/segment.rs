//! WAL segment naming helpers.
//!
//! Listing fails closed: a `*.wal` file with a non-canonical name or a gap in
//! the segment index sequence is a typed error, never silently excluded from
//! replay (silent exclusion would drop committed writes and could regress the
//! active segment index onto an existing file).

use crate::storage_names::wal_segment_index;
use calyx_core::{CalyxError, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn segment_path(dir: &Path, index: u64) -> PathBuf {
    dir.join(format!("{index:020}.wal"))
}

pub(super) fn list_segments(dir: &Path) -> Result<Vec<(u64, PathBuf)>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    for entry in fs::read_dir(dir).map_err(|error| super::storage_error("list WAL", error))? {
        let entry = entry.map_err(|error| super::storage_error("list WAL", error))?;
        let path = entry.path();
        if let Some(index) = wal_segment_index(&path)? {
            segments.push((index, path));
        }
    }
    segments.sort_by_key(|(index, _)| *index);
    validate_contiguous(&segments)?;
    Ok(segments)
}

/// Segments are created by rotation (`index + 1`) and only ever deleted from
/// the tail during torn-record truncation, so the on-disk indexes must be
/// contiguous. A gap means a segment file is missing and replay would
/// silently skip committed writes.
fn validate_contiguous(segments: &[(u64, PathBuf)]) -> Result<()> {
    for pair in segments.windows(2) {
        if pair[1].0 != pair[0].0 + 1 {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "WAL segment indexes are not contiguous: {} is followed by {}; \
                 a segment file is missing from replay",
                pair[0].1.display(),
                pair[1].1.display()
            )));
        }
    }
    Ok(())
}
