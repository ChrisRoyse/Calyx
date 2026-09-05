use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

use calyx_core::Result;

use crate::error::{CALYX_INDEX_CORRUPT, CALYX_INDEX_IO, sextant_error};

use super::{AssignmentSink, assignment_ids_rel};

// Amortize region-file opens across GPU chunks without making peak memory
// proportional to the corpus size.
const MAX_BUFFERED_IDS: usize = 128 * 1024 * 1024 / size_of::<u64>();

pub(super) struct AssignmentBuffer<'a> {
    root: &'a Path,
    sink: AssignmentSink,
    rows_by_region: Vec<Vec<u64>>,
    buffered: usize,
    max_buffered: usize,
}

impl<'a> AssignmentBuffer<'a> {
    pub(super) fn new(root: &'a Path, sink: AssignmentSink, regions: usize) -> Result<Self> {
        Self::with_limit(root, sink, regions, MAX_BUFFERED_IDS)
    }

    fn with_limit(
        root: &'a Path,
        sink: AssignmentSink,
        regions: usize,
        max_buffered: usize,
    ) -> Result<Self> {
        clear_stale_ids(root, sink, regions)?;
        Ok(Self {
            root,
            sink,
            rows_by_region: (0..regions).map(|_| Vec::new()).collect(),
            buffered: 0,
            max_buffered: max_buffered.max(1),
        })
    }

    pub(super) fn push(&mut self, row: u64, region: usize) -> Result<()> {
        let rows = self.rows_by_region.get_mut(region).ok_or_else(|| {
            sextant_error(
                CALYX_INDEX_CORRUPT,
                format!("assignment region {region} exceeds output region count"),
            )
        })?;
        rows.push(row);
        self.buffered += 1;
        if self.buffered >= self.max_buffered {
            self.flush()?;
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<()> {
        self.flush()
    }

    fn flush(&mut self) -> Result<()> {
        if self.buffered == 0 {
            return Ok(());
        }
        for (region, rows) in self.rows_by_region.iter_mut().enumerate() {
            if rows.is_empty() {
                continue;
            }
            let path = self.root.join(assignment_ids_rel(self.sink, region as u32));
            if let Some(parent) = path.parent() {
                calyx_aster::private_fs::create_private_dir_all(parent)?;
            }
            // Owner-only (0600) at creation, preserving the append accumulator
            // across flushes (#1989). `.mode()` only bites on the create, so
            // the readback proof runs once, on the flush that creates the file.
            let existed = path.exists();
            let mut options = OpenOptions::new();
            options.create(true).append(true);
            calyx_aster::private_fs::harden_open_options(&mut options);
            let file = options.open(&path).map_err(|error| {
                sextant_error(
                    CALYX_INDEX_IO,
                    format!("open ids {} for append: {error}", path.display()),
                )
            })?;
            if !existed {
                calyx_aster::private_fs::enforce_private_file_mode(&file, &path)?;
            }
            let mut writer = BufWriter::new(file);
            for row in rows.iter() {
                writer.write_all(&row.to_le_bytes()).map_err(|error| {
                    sextant_error(
                        CALYX_INDEX_IO,
                        format!("write region {region} id {row}: {error}"),
                    )
                })?;
            }
            writer.flush().map_err(|error| {
                sextant_error(
                    CALYX_INDEX_IO,
                    format!("flush ids {}: {error}", path.display()),
                )
            })?;
            *rows = Vec::new();
        }
        self.buffered = 0;
        Ok(())
    }
}

fn clear_stale_ids(root: &Path, sink: AssignmentSink, regions: usize) -> Result<()> {
    for region in 0..regions {
        let path = root.join(assignment_ids_rel(sink, region as u32));
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(sextant_error(
                    CALYX_INDEX_IO,
                    format!("remove stale ids {}: {error}", path.display()),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::partitioned::assignment::read_ids;

    #[test]
    fn repeated_flushes_preserve_per_region_row_order() {
        let root =
            std::env::temp_dir().join(format!("calyx-assignment-buffer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut output =
            AssignmentBuffer::with_limit(&root, AssignmentSink::Provisional, 2, 2).unwrap();

        output.push(0, 1).unwrap();
        output.push(1, 0).unwrap();
        output.push(2, 1).unwrap();
        output.finish().unwrap();

        assert_eq!(
            read_ids(&root.join("idx/assign-initial/region_00000.ids")).unwrap(),
            [1]
        );
        assert_eq!(
            read_ids(&root.join("idx/assign-initial/region_00001.ids")).unwrap(),
            [0, 2]
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
