//! Flat on-disk vector file (`.fbin`) of REAL embeddings — the source of truth for
//! partitioned-vault build and search. No vectors are ever synthesised: the builder
//! and bench read genuine embeddings produced by the real embedder (TEI) from a real
//! corpus.
//!
//! Layout (little-endian): magic `CLXVEC01` (8 B) | `u32 dim` | `u64 count` |
//! `f32[count*dim]` row-major. Row `i` is the embedding of corpus row `i`.

use std::fs::File;
use std::path::Path;

use calyx_core::Result;
use memmap2::Mmap;

use crate::error::{CALYX_INDEX_CORRUPT, CALYX_INDEX_IO, sextant_error};

pub const VEC_MAGIC: [u8; 8] = *b"CLXVEC01";
const HEADER_LEN: usize = 8 + 4 + 8;

/// mmap-backed reader over a `.fbin` of real embeddings. Reads are zero-copy slices
/// into the mapping, so build/search never materialise the whole file in heap.
#[derive(Debug)]
pub struct FbinVectors {
    mmap: Mmap,
    dim: usize,
    count: u64,
}

impl FbinVectors {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("open vecfile {}: {e}", path.display()),
            )
        })?;
        let len = file
            .metadata()
            .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("stat vecfile: {e}")))?
            .len();
        if len < HEADER_LEN as u64 {
            return Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                format!("vecfile {} is {len} B, smaller than header", path.display()),
            ));
        }
        // SAFETY: read-only map of a file written atomically by the embedder and not
        // mutated in place while open.
        let mmap = unsafe {
            Mmap::map(&file)
                .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("mmap vecfile: {e}")))?
        };
        if mmap[0..8] != VEC_MAGIC {
            return Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                format!("vecfile bad magic {:02x?}", &mmap[0..8]),
            ));
        }
        let dim = u32::from_le_bytes(mmap[8..12].try_into().expect("4B")) as usize;
        let count = u64::from_le_bytes(mmap[12..20].try_into().expect("8B"));
        if dim == 0 {
            return Err(sextant_error(CALYX_INDEX_CORRUPT, "vecfile dim is zero"));
        }
        let expect = HEADER_LEN as u64 + count * dim as u64 * 4;
        if len != expect {
            return Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                format!(
                    "vecfile {} len {len} != expected {expect} (count {count} x dim {dim} x 4 + {HEADER_LEN})",
                    path.display()
                ),
            ));
        }
        // The f32 region begins at byte 20; mmap base is page-aligned and 20 % 4 == 0,
        // so the region is 4-byte aligned for zero-copy f32 reads.
        if !(mmap.as_ptr() as usize + HEADER_LEN).is_multiple_of(std::mem::align_of::<f32>()) {
            return Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                "vecfile f32 region misaligned for zero-copy read",
            ));
        }
        Ok(Self { mmap, dim, count })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    /// Zero-copy view of row `idx`'s embedding. Panics out of range are converted to
    /// a fail-closed error by callers via `try_row`; this is the hot-path variant.
    pub fn row(&self, idx: u64) -> &[f32] {
        let start = HEADER_LEN + (idx as usize) * self.dim * 4;
        let bytes = &self.mmap[start..start + self.dim * 4];
        // SAFETY: alignment checked in `open`; length is an exact multiple of 4; f32
        // accepts any bit pattern; lifetime tied to the map.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<f32>(), self.dim) }
    }

    /// Bounds-checked row read (fail closed instead of panicking).
    pub fn try_row(&self, idx: u64) -> Result<&[f32]> {
        if idx >= self.count {
            return Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                format!("vecfile row {idx} >= count {}", self.count),
            ));
        }
        Ok(self.row(idx))
    }
}
