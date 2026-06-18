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
const I8BIN_HEADER_LEN: usize = 8;

#[derive(Debug)]
pub enum DenseVectorFile {
    Fbin(FbinVectors),
    I8Bin(I8BinVectors),
}

impl DenseVectorFile {
    pub fn open(path: &Path) -> Result<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("fbin") => Ok(Self::Fbin(FbinVectors::open(path)?)),
            Some("i8bin") => Ok(Self::I8Bin(I8BinVectors::open(path)?)),
            _ => Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                format!("unsupported vector file extension for {}", path.display()),
            )),
        }
    }

    pub fn dim(&self) -> usize {
        match self {
            Self::Fbin(file) => file.dim(),
            Self::I8Bin(file) => file.dim(),
        }
    }

    pub fn count(&self) -> u64 {
        match self {
            Self::Fbin(file) => file.count(),
            Self::I8Bin(file) => file.count(),
        }
    }

    pub fn row_f32(&self, idx: u64) -> Vec<f32> {
        match self {
            Self::Fbin(file) => file.row(idx).to_vec(),
            Self::I8Bin(file) => file.row_f32_normalized(idx),
        }
    }

    pub fn row_f32_raw(&self, idx: u64) -> Vec<f32> {
        match self {
            Self::Fbin(file) => file.row(idx).to_vec(),
            Self::I8Bin(file) => file.row_f32_raw(idx),
        }
    }
}

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

/// mmap-backed reader for BigANN-style `.i8bin` signed-int8 vectors.
///
/// Layout (little-endian): `u32 count | u32 dim | i8[count*dim]` row-major.
/// Rows are normalized when converted to f32 so Calyx's cosine DiskANN path and
/// brute-force readback operate on the same byte-derived vectors.
#[derive(Debug)]
pub struct I8BinVectors {
    mmap: Mmap,
    dim: usize,
    count: u64,
}

impl I8BinVectors {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("open i8bin {}: {e}", path.display()),
            )
        })?;
        let len = file
            .metadata()
            .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("stat i8bin: {e}")))?
            .len();
        if len < I8BIN_HEADER_LEN as u64 {
            return Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                format!("i8bin {} is {len} B, smaller than header", path.display()),
            ));
        }
        // SAFETY: read-only map of an immutable dataset file.
        let mmap = unsafe {
            Mmap::map(&file)
                .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("mmap i8bin: {e}")))?
        };
        let count = u32::from_le_bytes(mmap[0..4].try_into().expect("4B")) as u64;
        let dim = u32::from_le_bytes(mmap[4..8].try_into().expect("4B")) as usize;
        if dim == 0 {
            return Err(sextant_error(CALYX_INDEX_CORRUPT, "i8bin dim is zero"));
        }
        let expect = I8BIN_HEADER_LEN as u64 + count * dim as u64;
        if len != expect {
            return Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                format!(
                    "i8bin {} len {len} != expected {expect} (count {count} x dim {dim} + {I8BIN_HEADER_LEN})",
                    path.display()
                ),
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

    pub fn row_i8(&self, idx: u64) -> &[i8] {
        let start = I8BIN_HEADER_LEN + (idx as usize) * self.dim;
        let bytes = &self.mmap[start..start + self.dim];
        // SAFETY: i8 has alignment 1 and accepts every byte pattern.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<i8>(), self.dim) }
    }

    pub fn row_f32_normalized(&self, idx: u64) -> Vec<f32> {
        let mut out = self
            .row_i8(idx)
            .iter()
            .map(|value| f32::from(*value))
            .collect::<Vec<_>>();
        let norm = out.iter().map(|value| value * value).sum::<f32>().sqrt();
        if norm > 0.0 {
            for value in &mut out {
                *value /= norm;
            }
        }
        out
    }

    pub fn row_f32_raw(&self, idx: u64) -> Vec<f32> {
        self.row_i8(idx)
            .iter()
            .map(|value| f32::from(*value))
            .collect()
    }
}

#[derive(Debug)]
pub struct I32BinMatrix {
    mmap: Mmap,
    width: usize,
    count: u64,
}

impl I32BinMatrix {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| {
            sextant_error(
                CALYX_INDEX_IO,
                format!("open i32bin {}: {e}", path.display()),
            )
        })?;
        let len = file
            .metadata()
            .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("stat i32bin: {e}")))?
            .len();
        if len < I8BIN_HEADER_LEN as u64 {
            return Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                format!("i32bin {} is {len} B, smaller than header", path.display()),
            ));
        }
        // SAFETY: read-only map of an immutable dataset file.
        let mmap = unsafe {
            Mmap::map(&file)
                .map_err(|e| sextant_error(CALYX_INDEX_IO, format!("mmap i32bin: {e}")))?
        };
        let count = u32::from_le_bytes(mmap[0..4].try_into().expect("4B")) as u64;
        let width = u32::from_le_bytes(mmap[4..8].try_into().expect("4B")) as usize;
        if width == 0 {
            return Err(sextant_error(CALYX_INDEX_CORRUPT, "i32bin width is zero"));
        }
        let expect = I8BIN_HEADER_LEN as u64 + count * width as u64 * 4;
        if len != expect {
            return Err(sextant_error(
                CALYX_INDEX_CORRUPT,
                format!(
                    "i32bin {} len {len} != expected {expect} (count {count} x width {width} x 4 + {I8BIN_HEADER_LEN})",
                    path.display()
                ),
            ));
        }
        Ok(Self { mmap, width, count })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn row(&self, idx: u64) -> Vec<i32> {
        let start = I8BIN_HEADER_LEN + (idx as usize) * self.width * 4;
        self.mmap[start..start + self.width * 4]
            .chunks_exact(4)
            .map(|chunk| i32::from_le_bytes(chunk.try_into().expect("4B")))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn i8bin_rows_are_read_from_header_and_normalized() {
        let path = std::env::temp_dir().join(format!(
            "calyx-i8bin-{}-{}.i8bin",
            std::process::id(),
            "normalize"
        ));
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&[-3_i8 as u8, 0, 4, 1, 2, 2]);
        fs::write(&path, bytes).expect("write i8bin");

        let file = I8BinVectors::open(&path).expect("open i8bin");

        assert_eq!(file.count(), 2);
        assert_eq!(file.dim(), 3);
        assert_eq!(file.row_f32_raw(0), [-3.0, 0.0, 4.0]);
        assert_close(&file.row_f32_normalized(0), &[-0.6, 0.0, 0.8]);
        assert_close(
            &DenseVectorFile::open(&path).unwrap().row_f32(1),
            &[1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0],
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn i32bin_matrix_reads_header_and_rows() {
        let path = std::env::temp_dir().join(format!(
            "calyx-i32bin-{}-{}.i32bin",
            std::process::id(),
            "truth"
        ));
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        for value in [9_i32, 8, 7, 6, 5, 4] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fs::write(&path, bytes).expect("write i32bin");

        let file = I32BinMatrix::open(&path).expect("open i32bin");

        assert_eq!(file.count(), 2);
        assert_eq!(file.width(), 3);
        assert_eq!(file.row(0), [9, 8, 7]);
        assert_eq!(file.row(1), [6, 5, 4]);
        let _ = fs::remove_file(path);
    }

    fn assert_close(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual - expected).abs() < 0.00001);
        }
    }
}
