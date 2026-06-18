use std::path::Path;

use calyx_core::Result;

use super::gen_row;
use crate::index::vecfile::{FbinVectors, I8BinVectors};

/// Source of the vectors a partitioned vault is built from. The real production
/// path reads genuine embeddings from disk. Synthetic rows exist only for
/// builder-logic unit tests and must never back a recall or FSV claim.
pub trait VectorSource: Sync {
    fn dim(&self) -> usize;
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn row(&self, idx: u64) -> Vec<f32>;
}

/// Real float32 embeddings memory-mapped from Calyx `.fbin`.
pub struct FbinSource {
    vectors: FbinVectors,
}

impl FbinSource {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            vectors: FbinVectors::open(path)?,
        })
    }
}

impl VectorSource for FbinSource {
    fn dim(&self) -> usize {
        self.vectors.dim()
    }
    fn len(&self) -> u64 {
        self.vectors.count()
    }
    fn row(&self, idx: u64) -> Vec<f32> {
        self.vectors.row(idx).to_vec()
    }
}

/// Real signed-int8 BigANN vectors, normalized to Calyx's cosine geometry.
pub struct I8BinSource {
    vectors: I8BinVectors,
    normalize: bool,
}

impl I8BinSource {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            vectors: I8BinVectors::open(path)?,
            normalize: true,
        })
    }

    pub fn open_raw(path: &Path) -> Result<Self> {
        Ok(Self {
            vectors: I8BinVectors::open(path)?,
            normalize: false,
        })
    }
}

impl VectorSource for I8BinSource {
    fn dim(&self) -> usize {
        self.vectors.dim()
    }
    fn len(&self) -> u64 {
        self.vectors.count()
    }
    fn row(&self, idx: u64) -> Vec<f32> {
        if self.normalize {
            self.vectors.row_f32_normalized(idx)
        } else {
            self.vectors.row_f32_raw(idx)
        }
    }
}

/// Deterministic synthetic rows. Builder-logic unit tests only.
pub struct SyntheticSource {
    pub seed: u64,
    pub dim: usize,
    pub n_cx: u64,
}

impl VectorSource for SyntheticSource {
    fn dim(&self) -> usize {
        self.dim
    }
    fn len(&self) -> u64 {
        self.n_cx
    }
    fn row(&self, idx: u64) -> Vec<f32> {
        gen_row(self.seed, idx, self.dim)
    }
}
