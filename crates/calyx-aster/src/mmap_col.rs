//! Read-only mmap accessor for cold/columnar Aster bytes.
//!
//! ZFS note: on SST/column datasets, operators should prefer
//! `primarycache=metadata` to avoid double-caching the same cold column in both
//! process RSS and ZFS ARC.

use calyx_core::{CalyxError, Result};
#[cfg(unix)]
use memmap2::{Advice, UncheckedAdvice};
use memmap2::{Mmap, MmapOptions};
use std::fs::File;
use std::mem::{align_of, size_of};
use std::path::{Path, PathBuf};

pub const CALYX_NOT_FOUND: &str = "CALYX_NOT_FOUND";
pub const CALYX_IO_ERROR: &str = "CALYX_IO_ERROR";
pub const CALYX_BOUNDS_EXCEEDED: &str = "CALYX_BOUNDS_EXCEEDED";

/// Read-only mmap over an immutable cold column file.
///
/// The backing file must not be truncated while mapped; Unix kernels can raise
/// SIGBUS on later access when a live mapping outlives the backing bytes.
#[derive(Debug)]
pub struct MmapColumn {
    mmap: Mmap,
    path: PathBuf,
    file_len: usize,
}

impl MmapColumn {
    /// Opens a non-empty file read-only and maps it into the process address space.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => not_found(format!("{} not found", path.display())),
            _ => io_error(format!("open {}: {error}", path.display())),
        })?;
        let len = file
            .metadata()
            .map_err(|error| io_error(format!("metadata {}: {error}", path.display())))?
            .len();
        let file_len = usize::try_from(len).map_err(|_| {
            bounds_exceeded(format!("{} length {len} exceeds usize", path.display()))
        })?;
        if file_len == 0 {
            return Err(not_found(format!("{} is empty", path.display())));
        }
        // SAFETY: the mapping is read-only, the file is not mutated by this
        // type, and all public slice accessors bounds-check against file_len.
        let mmap = unsafe {
            MmapOptions::new()
                .map(&file)
                .map_err(|error| io_error(format!("mmap {}: {error}", path.display())))?
        };
        Ok(Self {
            mmap,
            path: path.to_path_buf(),
            file_len,
        })
    }

    pub fn read_slice(&self, offset: usize, len: usize) -> Result<&[u8]> {
        let end = self.checked_end(offset, len)?;
        self.mmap
            .get(offset..end)
            .ok_or_else(|| bounds_exceeded(self.bounds_message(offset, len)))
    }

    pub fn read_f32_slice(&self, offset: usize, count: usize) -> Result<&[f32]> {
        let byte_len = count
            .checked_mul(size_of::<f32>())
            .ok_or_else(|| bounds_exceeded("f32 slice byte length overflow"))?;
        if !offset.is_multiple_of(align_of::<f32>()) {
            return Err(bounds_exceeded(format!(
                "f32 offset {offset} is not {}-byte aligned",
                align_of::<f32>()
            )));
        }
        let bytes = self.read_slice(offset, byte_len)?;
        if !(bytes.as_ptr() as usize).is_multiple_of(align_of::<f32>()) {
            return Err(bounds_exceeded("mmap base is not f32 aligned"));
        }
        // SAFETY: byte_len is count * size_of::<f32>(), offset and pointer
        // alignment are verified, and the returned slice is tied to &self.
        Ok(unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<f32>(), count) })
    }

    pub fn prefetch(&self, offset: usize, len: usize) {
        self.advise_range(offset, len, PageAdvice::WillNeed);
    }

    pub fn drop_pages(&self, offset: usize, len: usize) {
        self.advise_range(offset, len, PageAdvice::DontNeed);
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.mmap
    }

    pub fn file_len(&self) -> usize {
        self.file_len
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn checked_end(&self, offset: usize, len: usize) -> Result<usize> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| bounds_exceeded(self.bounds_message(offset, len)))?;
        if end > self.file_len {
            return Err(bounds_exceeded(self.bounds_message(offset, len)));
        }
        Ok(end)
    }

    fn bounds_message(&self, offset: usize, len: usize) -> String {
        format!(
            "{} range offset={} len={} exceeds file_len={}",
            self.path.display(),
            offset,
            len,
            self.file_len
        )
    }

    #[cfg(unix)]
    fn advise_range(&self, offset: usize, len: usize, advice: PageAdvice) {
        if self.checked_end(offset, len).is_err() {
            return;
        }
        match advice {
            PageAdvice::WillNeed => {
                let _ = self.mmap.advise_range(Advice::WillNeed, offset, len);
            }
            PageAdvice::DontNeed => {
                // SAFETY: the mapping is read-only and file-backed. This is a
                // best-effort page-cache hint after our public range check.
                let _ = unsafe {
                    self.mmap
                        .unchecked_advise_range(UncheckedAdvice::DontNeed, offset, len)
                };
            }
        }
    }

    #[cfg(not(unix))]
    fn advise_range(&self, offset: usize, len: usize, _advice: PageAdvice) {
        let _ = self.checked_end(offset, len);
    }
}

#[derive(Debug, Clone, Copy)]
enum PageAdvice {
    WillNeed,
    DontNeed,
}

fn not_found(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_NOT_FOUND,
        message: message.into(),
        remediation: "create a non-empty cold column file before opening it",
    }
}

fn io_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_IO_ERROR,
        message: message.into(),
        remediation: "inspect the OS file error and storage path",
    }
}

fn bounds_exceeded(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_BOUNDS_EXCEEDED,
        message: message.into(),
        remediation: "read within the mapped column length and alignment",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn read_slice_returns_known_bytes() {
        let dir = test_dir("bytes");
        let path = dir.join("column.bin");
        let expected = (0..1024)
            .map(|value| (value % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&path, &expected).unwrap();

        let column = MmapColumn::open(&path).unwrap();

        assert_eq!(column.file_len(), 1024);
        assert_eq!(column.read_slice(0, 1024).unwrap(), expected);
        cleanup(dir);
    }

    #[test]
    fn read_f32_slice_returns_known_values() {
        let dir = test_dir("f32");
        let path = dir.join("f32.bin");
        let expected = [1.0_f32, 2.0, 3.0, 4.0];
        let bytes = expected
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect::<Vec<_>>();
        fs::write(&path, bytes).unwrap();

        let column = MmapColumn::open(&path).unwrap();

        assert_eq!(column.read_f32_slice(0, expected.len()).unwrap(), expected);
        cleanup(dir);
    }

    #[test]
    fn bounds_and_alignment_fail_closed() {
        let dir = test_dir("bounds");
        let path = dir.join("column.bin");
        fs::write(&path, vec![0_u8; 1024]).unwrap();
        let column = MmapColumn::open(&path).unwrap();

        let bounds = column.read_slice(1020, 8).unwrap_err();
        let unaligned = column.read_f32_slice(3, 1).unwrap_err();

        assert_eq!(bounds.code, CALYX_BOUNDS_EXCEEDED);
        assert_eq!(unaligned.code, CALYX_BOUNDS_EXCEEDED);
        cleanup(dir);
    }

    #[test]
    fn missing_and_empty_files_are_not_found() {
        let dir = test_dir("missing");
        let missing = MmapColumn::open(&dir.join("missing.bin")).unwrap_err();
        let empty_path = dir.join("empty.bin");
        fs::write(&empty_path, []).unwrap();
        let empty = MmapColumn::open(&empty_path).unwrap_err();

        assert_eq!(missing.code, CALYX_NOT_FOUND);
        assert_eq!(empty.code, CALYX_NOT_FOUND);
        cleanup(dir);
    }

    #[test]
    fn page_advice_is_best_effort() {
        let dir = test_dir("advice");
        let path = dir.join("column.bin");
        fs::write(&path, vec![1_u8; 4096]).unwrap();
        let column = MmapColumn::open(&path).unwrap();

        column.prefetch(0, 1024);
        column.drop_pages(0, 1024);
        column.prefetch(4090, 16);
        column.drop_pages(4090, 16);
        cleanup(dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-mmap-column-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).unwrap();
    }
}
