use std::fs::{self, File};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use calyx_core::Result;
use serde::{Deserialize, Serialize};

use crate::error::{
    CALYX_INDEX_CORRUPT, CALYX_INDEX_DIM_MISMATCH, CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO,
    sextant_error,
};

mod cpu;
mod gpu;
mod validate;

const PQ_MAGIC: [u8; 8] = *b"CLXPQ001";
const PQ_VERSION: u32 = 1;
const HEADER_BYTES: usize = 40;

/// At or below this measured range, CUDA context cost exceeds CPU Lloyd work.
pub const DISKANN_PQ_SMALL_CORPUS_ROWS: usize = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskAnnPqBuildExecution {
    Auto,
    CpuReference,
    CudaRequired,
}

impl DiskAnnPqBuildExecution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::CpuReference => "cpu-reference",
            Self::CudaRequired => "cuda-required",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskAnnPqBuildParams {
    pub subvectors: usize,
    pub centroids: usize,
    pub iterations: usize,
}

impl Default for DiskAnnPqBuildParams {
    fn default() -> Self {
        Self {
            subvectors: 16,
            centroids: 256,
            iterations: 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskAnnPqBuildDiagnostics {
    pub backend: String,
    pub requested_execution: String,
    pub strict_gpu_required: bool,
    pub small_corpus_cpu_max_rows: usize,
    pub row_count: usize,
    pub dim: usize,
    pub subvectors: usize,
    pub centroids: usize,
    pub iterations: usize,
    pub pinned_staging: bool,
    pub resident_corpus: bool,
    pub chunk_rows: usize,
    pub chunks_per_pass: usize,
    pub subspace_upload_reuse: bool,
    pub cagra_device_reuse: bool,
    pub cagra_device_reuse_reason: String,
    pub corpus_uploads: usize,
    pub h2d_transfers: usize,
    pub d2h_transfers: usize,
    pub corpus_bytes_uploaded: u64,
    pub codebook_bytes_uploaded: usize,
    pub codebook_bytes_read: usize,
    pub codes_bytes_read: usize,
    pub assignment_kernel_launches: usize,
    pub accumulation_kernel_launches: usize,
    pub centroid_kernel_launches: usize,
    pub memset_operations: usize,
    pub peak_device_bytes: usize,
    pub peak_pinned_host_bytes: usize,
    pub staging_us: u128,
    pub training_us: u128,
    pub encoding_us: u128,
    pub total_us: u128,
}

impl DiskAnnPqBuildDiagnostics {
    fn sidecar_read(index: &DiskAnnPqIndex) -> Self {
        Self {
            backend: "sidecar-v1-read".to_string(),
            requested_execution: "sidecar-read".to_string(),
            strict_gpu_required: false,
            small_corpus_cpu_max_rows: DISKANN_PQ_SMALL_CORPUS_ROWS,
            row_count: index.node_count,
            dim: index.dim,
            subvectors: index.subvectors,
            centroids: index.centroids,
            iterations: index.iterations,
            pinned_staging: false,
            resident_corpus: false,
            chunk_rows: 0,
            chunks_per_pass: 0,
            subspace_upload_reuse: false,
            cagra_device_reuse: false,
            cagra_device_reuse_reason: "sidecar read performs no build".to_string(),
            corpus_uploads: 0,
            h2d_transfers: 0,
            d2h_transfers: 0,
            corpus_bytes_uploaded: 0,
            codebook_bytes_uploaded: 0,
            codebook_bytes_read: 0,
            codes_bytes_read: 0,
            assignment_kernel_launches: 0,
            accumulation_kernel_launches: 0,
            centroid_kernel_launches: 0,
            memset_operations: 0,
            peak_device_bytes: 0,
            peak_pinned_host_bytes: 0,
            staging_us: 0,
            training_us: 0,
            encoding_us: 0,
            total_us: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiskAnnPqIndex {
    dim: usize,
    node_count: usize,
    subvectors: usize,
    centroids: usize,
    subdim: usize,
    iterations: usize,
    codebook: Vec<f32>,
    codes: Vec<u8>,
    diagnostics: DiskAnnPqBuildDiagnostics,
}

pub(super) struct BuildOutput {
    codebook: Vec<f32>,
    codes: Vec<u8>,
    diagnostics: DiskAnnPqBuildDiagnostics,
}

#[derive(Debug)]
pub struct DiskAnnPqQuery<'a> {
    lut: Vec<f32>,
    subvectors: usize,
    centroids: usize,
    codes: &'a [u8],
}

impl DiskAnnPqIndex {
    pub fn build(rows: &[(u32, Vec<f32>)], params: DiskAnnPqBuildParams) -> Result<Self> {
        Self::build_with_execution(rows, params, DiskAnnPqBuildExecution::Auto)
    }

    pub fn build_with_execution(
        rows: &[(u32, Vec<f32>)],
        params: DiskAnnPqBuildParams,
        execution: DiskAnnPqBuildExecution,
    ) -> Result<Self> {
        validate::rows(rows, params)?;
        let dim = rows[0].1.len();
        let node_count = rows.len();
        let centroids = params.centroids.min(node_count);
        let effective = match execution {
            DiskAnnPqBuildExecution::Auto if node_count <= DISKANN_PQ_SMALL_CORPUS_ROWS => {
                DiskAnnPqBuildExecution::CpuReference
            }
            DiskAnnPqBuildExecution::Auto => DiskAnnPqBuildExecution::CudaRequired,
            explicit => explicit,
        };
        let output = match effective {
            DiskAnnPqBuildExecution::CpuReference => cpu::build(rows, params, execution),
            DiskAnnPqBuildExecution::CudaRequired => gpu::build(rows, params, execution),
            DiskAnnPqBuildExecution::Auto => unreachable!("auto execution was resolved"),
        }?;
        Ok(Self {
            dim,
            node_count,
            subvectors: params.subvectors,
            centroids,
            subdim: dim / params.subvectors,
            iterations: params.iterations,
            codebook: output.codebook,
            codes: output.codes,
            diagnostics: output.diagnostics,
        })
    }

    pub fn read_if_exists(path: &Path) -> Result<Option<Self>> {
        path.is_file().then(|| Self::read(path)).transpose()
    }

    pub fn read(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|error| io("open pq sidecar", error))?
            .read_to_end(&mut bytes)
            .map_err(|error| io("read pq sidecar", error))?;
        if bytes.len() < HEADER_BYTES {
            return Err(corrupt(format!(
                "pq sidecar {} is {} B, shorter than header",
                path.display(),
                bytes.len()
            )));
        }
        if bytes[0..8] != PQ_MAGIC {
            return Err(corrupt(format!("pq sidecar {} bad magic", path.display())));
        }
        let version = le_u32(&bytes, 8);
        if version != PQ_VERSION {
            return Err(corrupt(format!("pq sidecar version {version}")));
        }
        let dim = le_u32(&bytes, 12) as usize;
        let node_count = le_u64(&bytes, 16) as usize;
        let subvectors = le_u32(&bytes, 24) as usize;
        let centroids = le_u32(&bytes, 28) as usize;
        let subdim = le_u32(&bytes, 32) as usize;
        let iterations = le_u32(&bytes, 36) as usize;
        validate::header(dim, node_count, subvectors, centroids, subdim, iterations)?;
        let codebook_floats = subvectors
            .checked_mul(centroids)
            .and_then(|value| value.checked_mul(subdim))
            .ok_or_else(|| corrupt("pq codebook size overflow"))?;
        let codebook_bytes = codebook_floats
            .checked_mul(4)
            .ok_or_else(|| corrupt("pq codebook byte size overflow"))?;
        let codes_bytes = node_count
            .checked_mul(subvectors)
            .ok_or_else(|| corrupt("pq code byte size overflow"))?;
        let expected = HEADER_BYTES + codebook_bytes + codes_bytes;
        if bytes.len() != expected {
            return Err(corrupt(format!(
                "pq sidecar {} len {} != expected {expected}",
                path.display(),
                bytes.len()
            )));
        }
        let codes_start = HEADER_BYTES + codebook_bytes;
        let mut codebook = Vec::with_capacity(codebook_floats);
        for chunk in bytes[HEADER_BYTES..codes_start].chunks_exact(4) {
            let value = f32::from_le_bytes(chunk.try_into().expect("4B"));
            if !value.is_finite() {
                return Err(corrupt("pq codebook contains non-finite centroid"));
            }
            codebook.push(value);
        }
        let codes = bytes[codes_start..].to_vec();
        if codes.iter().any(|code| *code as usize >= centroids) {
            return Err(corrupt("pq sidecar contains out-of-range code"));
        }
        let mut index = Self {
            dim,
            node_count,
            subvectors,
            centroids,
            subdim,
            iterations,
            codebook,
            codes,
            diagnostics: empty_diagnostics(),
        };
        index.diagnostics = DiskAnnPqBuildDiagnostics::sidecar_read(&index);
        Ok(index)
    }

    pub fn write_atomic(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            calyx_aster::private_fs::create_private_dir_all(parent)?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        // Owner-only (0600) at creation (#1989); existing sync_all + rename
        // below keep the PQ sidecar's durability unchanged.
        let mut file = calyx_aster::private_fs::open_private_temp_file(&tmp)?;
        file.write_all(&PQ_MAGIC)
            .and_then(|_| file.write_all(&PQ_VERSION.to_le_bytes()))
            .and_then(|_| file.write_all(&(self.dim as u32).to_le_bytes()))
            .and_then(|_| file.write_all(&(self.node_count as u64).to_le_bytes()))
            .and_then(|_| file.write_all(&(self.subvectors as u32).to_le_bytes()))
            .and_then(|_| file.write_all(&(self.centroids as u32).to_le_bytes()))
            .and_then(|_| file.write_all(&(self.subdim as u32).to_le_bytes()))
            .and_then(|_| file.write_all(&(self.iterations as u32).to_le_bytes()))
            .map_err(|error| io("write pq header", error))?;
        for value in &self.codebook {
            file.write_all(&value.to_le_bytes())
                .map_err(|error| io("write pq codebook", error))?;
        }
        file.write_all(&self.codes)
            .map_err(|error| io("write pq codes", error))?;
        file.sync_all()
            .map_err(|error| io("fsync pq sidecar", error))?;
        drop(file);
        fs::rename(&tmp, path).map_err(|error| io("publish pq sidecar", error))
    }

    pub fn query<'a>(&'a self, query: &[f32]) -> Result<DiskAnnPqQuery<'a>> {
        if query.len() != self.dim {
            return Err(sextant_error(
                CALYX_INDEX_DIM_MISMATCH,
                format!("pq query dim {} expected {}", query.len(), self.dim),
            ));
        }
        if query.iter().any(|value| !value.is_finite()) {
            return Err(invalid("pq query contains non-finite component"));
        }
        let mut lut = vec![0.0; self.subvectors * self.centroids];
        for subvector in 0..self.subvectors {
            let offset = subvector * self.subdim;
            let values = &query[offset..offset + self.subdim];
            for centroid in 0..self.centroids {
                lut[subvector * self.centroids + centroid] =
                    l2_sq(values, self.centroid(subvector, centroid));
            }
        }
        Ok(DiskAnnPqQuery {
            lut,
            subvectors: self.subvectors,
            centroids: self.centroids,
            codes: &self.codes,
        })
    }

    pub fn ram_bytes(&self) -> usize {
        self.codes.len() + self.codebook.len() * size_of::<f32>()
    }

    pub fn node_count(&self) -> usize {
        self.node_count
    }

    pub fn subvectors(&self) -> usize {
        self.subvectors
    }

    pub fn centroids(&self) -> usize {
        self.centroids
    }

    pub fn build_params(&self) -> DiskAnnPqBuildParams {
        DiskAnnPqBuildParams {
            subvectors: self.subvectors,
            centroids: self.centroids,
            iterations: self.iterations,
        }
    }

    pub fn build_diagnostics(&self) -> &DiskAnnPqBuildDiagnostics {
        &self.diagnostics
    }

    pub fn codebook(&self) -> &[f32] {
        &self.codebook
    }

    pub fn codes(&self) -> &[u8] {
        &self.codes
    }

    fn centroid(&self, subvector: usize, centroid: usize) -> &[f32] {
        let at = (subvector * self.centroids + centroid) * self.subdim;
        &self.codebook[at..at + self.subdim]
    }
}

impl DiskAnnPqQuery<'_> {
    pub fn distance_l2(&self, id: u32) -> Result<f32> {
        let row = id as usize;
        let code_offset = row
            .checked_mul(self.subvectors)
            .ok_or_else(|| invalid("pq code offset overflow"))?;
        if code_offset + self.subvectors > self.codes.len() {
            return Err(invalid(format!("pq missing codes for node {id}")));
        }
        let mut sum = 0.0;
        for subvector in 0..self.subvectors {
            let code = self.codes[code_offset + subvector] as usize;
            if code >= self.centroids {
                return Err(corrupt(format!("pq code {code} >= {}", self.centroids)));
            }
            sum += self.lut[subvector * self.centroids + code];
        }
        Ok(sum)
    }
}

pub fn default_pq_sidecar(graph_path: &Path) -> PathBuf {
    graph_path.with_extension("pq")
}

pub(super) fn initial_codebook(
    rows: &[(u32, Vec<f32>)],
    subvectors: usize,
    centroids: usize,
) -> Vec<f32> {
    let dim = rows[0].1.len();
    let subdim = dim / subvectors;
    let mut codebook = Vec::with_capacity(subvectors * centroids * subdim);
    for subvector in 0..subvectors {
        let offset = subvector * subdim;
        for centroid in 0..centroids {
            let row = &rows[centroid * rows.len() / centroids].1;
            codebook.extend_from_slice(&row[offset..offset + subdim]);
        }
    }
    codebook
}

pub(super) fn l2_sq(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let delta = x - y;
            delta * delta
        })
        .sum()
}

fn empty_diagnostics() -> DiskAnnPqBuildDiagnostics {
    DiskAnnPqBuildDiagnostics {
        backend: String::new(),
        requested_execution: String::new(),
        strict_gpu_required: false,
        small_corpus_cpu_max_rows: DISKANN_PQ_SMALL_CORPUS_ROWS,
        row_count: 0,
        dim: 0,
        subvectors: 0,
        centroids: 0,
        iterations: 0,
        pinned_staging: false,
        resident_corpus: false,
        chunk_rows: 0,
        chunks_per_pass: 0,
        subspace_upload_reuse: false,
        cagra_device_reuse: false,
        cagra_device_reuse_reason: String::new(),
        corpus_uploads: 0,
        h2d_transfers: 0,
        d2h_transfers: 0,
        corpus_bytes_uploaded: 0,
        codebook_bytes_uploaded: 0,
        codebook_bytes_read: 0,
        codes_bytes_read: 0,
        assignment_kernel_launches: 0,
        accumulation_kernel_launches: 0,
        centroid_kernel_launches: 0,
        memset_operations: 0,
        peak_device_bytes: 0,
        peak_pinned_host_bytes: 0,
        staging_us: 0,
        training_us: 0,
        encoding_us: 0,
        total_us: 0,
    }
}

fn le_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4B"))
}

fn le_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8B"))
}

pub(super) fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_INVALID_PARAMS,
        format!("diskann pq invalid params: {detail}"),
    )
}

pub(super) fn corrupt(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_CORRUPT, format!("diskann pq corrupt: {detail}"))
}

fn io(stage: &str, error: std::io::Error) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_IO, format!("diskann pq {stage}: {error}"))
}
