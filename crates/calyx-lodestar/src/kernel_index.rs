use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::CxId;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{Kernel, LodestarError, Result};

const FORMAT_VERSION: u32 = 1;

pub trait EmbeddingStore {
    fn embedding(&self, cx_id: CxId) -> Result<Option<Vec<f32>>>;
}

impl EmbeddingStore for BTreeMap<CxId, Vec<f32>> {
    fn embedding(&self, cx_id: CxId) -> Result<Option<Vec<f32>>> {
        Ok(self.get(&cx_id).cloned())
    }
}

pub trait KernelStore {
    fn write_index_bytes(&self, kernel_id: CxId, bytes: &[u8]) -> Result<()>;
    fn read_index_bytes(&self, kernel_id: CxId) -> Result<Option<Vec<u8>>>;
}

#[derive(Clone, Debug)]
pub struct FsKernelStore {
    root: PathBuf,
}

impl FsKernelStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn index_dir(&self, kernel_id: CxId) -> PathBuf {
        self.root
            .join("idx")
            .join("kernel")
            .join(kernel_id.to_string())
    }

    pub fn index_file_path(&self, kernel_id: CxId) -> PathBuf {
        self.index_dir(kernel_id).join("index.json")
    }

    pub fn kernel_file_path(&self, kernel_id: CxId) -> PathBuf {
        self.index_dir(kernel_id).join("kernel.json")
    }
}

impl KernelStore for FsKernelStore {
    fn write_index_bytes(&self, kernel_id: CxId, bytes: &[u8]) -> Result<()> {
        let dir = self.index_dir(kernel_id);
        fs::create_dir_all(&dir).map_err(io_error)?;
        let path = dir.join("index.json");
        let tmp = dir.join("index.json.tmp");
        fs::write(&tmp, bytes).map_err(io_error)?;
        if path.exists() {
            fs::remove_file(&path).map_err(io_error)?;
        }
        fs::rename(&tmp, &path).map_err(io_error)
    }

    fn read_index_bytes(&self, kernel_id: CxId) -> Result<Option<Vec<u8>>> {
        let path = self.index_file_path(kernel_id);
        if !Path::new(&path).exists() {
            return Ok(None);
        }
        fs::read(path).map(Some).map_err(io_error)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelVectorRow {
    pub cx_id: CxId,
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct KernelIndex {
    pub kernel_id: CxId,
    pub dim: usize,
    rows: Vec<KernelVectorRow>,
}

impl KernelIndex {
    pub fn rows(&self) -> &[KernelVectorRow] {
        &self.rows
    }

    pub fn filter_to_nodes(&self, allowed_nodes: &BTreeSet<CxId>) -> Result<Self> {
        let rows = self
            .rows
            .iter()
            .filter(|row| allowed_nodes.contains(&row.cx_id))
            .cloned()
            .collect::<Vec<_>>();
        Self::from_rows(self.kernel_id, rows)
    }

    fn from_rows(kernel_id: CxId, rows: Vec<KernelVectorRow>) -> Result<Self> {
        let dim = validate_rows(&rows)?;
        Ok(Self {
            kernel_id,
            dim,
            rows,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct KernelIndexSnapshot {
    format_version: u32,
    kernel_id: CxId,
    dim: usize,
    rows: Vec<KernelVectorRow>,
}

pub fn build_kernel_index(kernel: &Kernel, embeddings: &dyn EmbeddingStore) -> Result<KernelIndex> {
    if kernel.members.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let rows = kernel
        .members
        .iter()
        .map(|cx_id| {
            let vector = embeddings
                .embedding(*cx_id)?
                .ok_or(LodestarError::KernelEmbeddingMissing { cx_id: *cx_id })?;
            Ok(KernelVectorRow {
                cx_id: *cx_id,
                vector,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    KernelIndex::from_rows(kernel.kernel_id, rows)
}

pub fn kernel_search(
    index: &KernelIndex,
    query_vec: &[f32],
    top_k: usize,
) -> Result<Vec<(CxId, f32)>> {
    if query_vec.len() != index.dim {
        return Err(LodestarError::KernelDimMismatch {
            expected: index.dim,
            actual: query_vec.len(),
        });
    }
    if let Some((offset, _)) = query_vec
        .iter()
        .enumerate()
        .find(|(_, value)| !value.is_finite())
    {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!("query vector has non-finite value at offset {offset}"),
        });
    }
    Ok(top_k_by_score(
        index
            .rows
            .par_iter()
            .map(|row| (row.cx_id, cosine(query_vec, &row.vector)))
            .collect(),
        top_k,
    ))
}

pub fn write_kernel_index(index: &KernelIndex, store: &dyn KernelStore) -> Result<()> {
    let snapshot = KernelIndexSnapshot {
        format_version: FORMAT_VERSION,
        kernel_id: index.kernel_id,
        dim: index.dim,
        rows: index.rows.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&snapshot).map_err(codec_error)?;
    store.write_index_bytes(index.kernel_id, &bytes)
}

pub fn load_kernel_index(kernel_id: CxId, store: &dyn KernelStore) -> Result<KernelIndex> {
    let Some(bytes) = store.read_index_bytes(kernel_id)? else {
        return Err(LodestarError::KernelIndexNotFound { kernel_id });
    };
    let snapshot: KernelIndexSnapshot = serde_json::from_slice(&bytes).map_err(codec_error)?;
    if snapshot.format_version != FORMAT_VERSION {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!("unsupported format version {}", snapshot.format_version),
        });
    }
    if snapshot.kernel_id != kernel_id {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!(
                "snapshot kernel id {} did not match requested {}",
                snapshot.kernel_id, kernel_id
            ),
        });
    }
    let actual_dim = validate_rows(&snapshot.rows)?;
    if snapshot.dim != actual_dim {
        return Err(LodestarError::KernelDimMismatch {
            expected: snapshot.dim,
            actual: actual_dim,
        });
    }
    KernelIndex::from_rows(snapshot.kernel_id, snapshot.rows)
}

fn validate_rows(rows: &[KernelVectorRow]) -> Result<usize> {
    if rows.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let dim = rows[0].vector.len();
    if dim == 0 {
        return Err(LodestarError::KernelInvalidParams {
            detail: "kernel vectors must have non-zero dimension".to_string(),
        });
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.cx_id) {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("duplicate kernel row {}", row.cx_id),
            });
        }
        if row.vector.len() != dim {
            return Err(LodestarError::KernelDimMismatch {
                expected: dim,
                actual: row.vector.len(),
            });
        }
        if let Some((offset, _)) = row
            .vector
            .iter()
            .enumerate()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("row {} has non-finite value at offset {offset}", row.cx_id),
            });
        }
    }
    Ok(dim)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut an = 0.0_f32;
    let mut bn = 0.0_f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        an += x * x;
        bn += y * y;
    }
    if an == 0.0 || bn == 0.0 {
        0.0
    } else {
        dot / (an.sqrt() * bn.sqrt())
    }
}

fn top_k_by_score(mut scored: Vec<(CxId, f32)>, top_k: usize) -> Vec<(CxId, f32)> {
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    scored.truncate(top_k);
    scored
}

fn io_error(err: std::io::Error) -> LodestarError {
    LodestarError::KernelIndexIo {
        detail: err.to_string(),
    }
}

fn codec_error(err: serde_json::Error) -> LodestarError {
    LodestarError::KernelIndexCodec {
        detail: err.to_string(),
    }
}
