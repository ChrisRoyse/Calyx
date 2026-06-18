use std::fs::{self, File};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use calyx_core::Result;

use crate::error::{
    CALYX_INDEX_CORRUPT, CALYX_INDEX_DIM_MISMATCH, CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO,
    sextant_error,
};

const PQ_MAGIC: [u8; 8] = *b"CLXPQ001";
const PQ_VERSION: u32 = 1;
const HEADER_BYTES: usize = 40;

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
        validate_rows(rows, params)?;
        let dim = rows[0].1.len();
        let node_count = rows.len();
        let subvectors = params.subvectors;
        let centroids = params.centroids.min(node_count);
        let subdim = dim / subvectors;
        let mut codebook = Vec::with_capacity(subvectors * centroids * subdim);
        for subvector in 0..subvectors {
            train_subspace(
                rows,
                subvector * subdim,
                subdim,
                centroids,
                params.iterations,
                &mut codebook,
            );
        }
        let mut index = Self {
            dim,
            node_count,
            subvectors,
            centroids,
            subdim,
            iterations: params.iterations,
            codebook,
            codes: vec![0; node_count * subvectors],
        };
        index.encode_rows(rows)?;
        Ok(index)
    }

    pub fn read_if_exists(path: &Path) -> Result<Option<Self>> {
        if !path.is_file() {
            return Ok(None);
        }
        Self::read(path).map(Some)
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
        validate_header(dim, node_count, subvectors, centroids, subdim, iterations)?;
        let codebook_floats = subvectors
            .checked_mul(centroids)
            .and_then(|v| v.checked_mul(subdim))
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
        let codebook_start = HEADER_BYTES;
        let codes_start = codebook_start + codebook_bytes;
        let mut codebook = Vec::with_capacity(codebook_floats);
        for chunk in bytes[codebook_start..codes_start].chunks_exact(4) {
            let value = f32::from_le_bytes(chunk.try_into().expect("4B"));
            if !value.is_finite() {
                return Err(corrupt("pq codebook contains non-finite centroid"));
            }
            codebook.push(value);
        }
        Ok(Self {
            dim,
            node_count,
            subvectors,
            centroids,
            subdim,
            iterations,
            codebook,
            codes: bytes[codes_start..].to_vec(),
        })
    }

    pub fn write_atomic(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| io("create pq parent", error))?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let mut file = File::create(&tmp).map_err(|error| io("create pq tmp", error))?;
        file.write_all(&PQ_MAGIC)
            .map_err(|error| io("write pq magic", error))?;
        file.write_all(&PQ_VERSION.to_le_bytes())
            .map_err(|error| io("write pq version", error))?;
        file.write_all(&(self.dim as u32).to_le_bytes())
            .map_err(|error| io("write pq dim", error))?;
        file.write_all(&(self.node_count as u64).to_le_bytes())
            .map_err(|error| io("write pq node_count", error))?;
        file.write_all(&(self.subvectors as u32).to_le_bytes())
            .map_err(|error| io("write pq subvectors", error))?;
        file.write_all(&(self.centroids as u32).to_le_bytes())
            .map_err(|error| io("write pq centroids", error))?;
        file.write_all(&(self.subdim as u32).to_le_bytes())
            .map_err(|error| io("write pq subdim", error))?;
        file.write_all(&(self.iterations as u32).to_le_bytes())
            .map_err(|error| io("write pq iterations", error))?;
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
        if query.iter().any(|v| !v.is_finite()) {
            return Err(invalid("pq query contains non-finite component"));
        }
        let mut lut = vec![0.0; self.subvectors * self.centroids];
        for subvector in 0..self.subvectors {
            let offset = subvector * self.subdim;
            let q = &query[offset..offset + self.subdim];
            for centroid in 0..self.centroids {
                let c = self.centroid(subvector, centroid);
                lut[subvector * self.centroids + centroid] = l2_sq(q, c);
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

    fn encode_rows(&mut self, rows: &[(u32, Vec<f32>)]) -> Result<()> {
        for (idx, (id, vector)) in rows.iter().enumerate() {
            if *id as usize != idx {
                return Err(invalid(format!("pq row id {id} expected dense id {idx}")));
            }
            for subvector in 0..self.subvectors {
                let offset = subvector * self.subdim;
                let code = self.nearest(subvector, &vector[offset..offset + self.subdim]);
                self.codes[idx * self.subvectors + subvector] = code as u8;
            }
        }
        Ok(())
    }

    fn nearest(&self, subvector: usize, values: &[f32]) -> usize {
        (0..self.centroids)
            .min_by(|&a, &b| {
                let da = l2_sq(values, self.centroid(subvector, a));
                let db = l2_sq(values, self.centroid(subvector, b));
                da.total_cmp(&db).then_with(|| a.cmp(&b))
            })
            .unwrap_or(0)
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

fn validate_rows(rows: &[(u32, Vec<f32>)], params: DiskAnnPqBuildParams) -> Result<()> {
    if rows.is_empty() {
        return Err(invalid("pq requires at least one row"));
    }
    let dim = rows[0].1.len();
    if dim == 0 {
        return Err(invalid("pq dim must be positive"));
    }
    if params.subvectors == 0 || params.centroids == 0 || params.iterations == 0 {
        return Err(invalid(
            "pq subvectors, centroids, and iterations must be positive",
        ));
    }
    if params.centroids > 256 {
        return Err(invalid("pq centroids must be <= 256 for u8 codes"));
    }
    if params.subvectors > dim || !dim.is_multiple_of(params.subvectors) {
        return Err(invalid(format!(
            "pq dim {dim} must be divisible by subvectors {}",
            params.subvectors
        )));
    }
    for (idx, (id, vector)) in rows.iter().enumerate() {
        if *id as usize != idx {
            return Err(invalid(format!("pq row id {id} expected dense id {idx}")));
        }
        if vector.len() != dim {
            return Err(sextant_error(
                CALYX_INDEX_DIM_MISMATCH,
                format!("pq vector {id} dim {} expected {dim}", vector.len()),
            ));
        }
        if vector.iter().any(|v| !v.is_finite()) {
            return Err(invalid(format!("pq vector {id} contains non-finite value")));
        }
    }
    Ok(())
}

fn validate_header(
    dim: usize,
    node_count: usize,
    subvectors: usize,
    centroids: usize,
    subdim: usize,
    iterations: usize,
) -> Result<()> {
    if dim == 0
        || node_count == 0
        || subvectors == 0
        || centroids == 0
        || subdim == 0
        || iterations == 0
    {
        return Err(corrupt("pq header contains zero field"));
    }
    if centroids > 256 {
        return Err(corrupt("pq centroids exceed u8 code space"));
    }
    if subvectors.checked_mul(subdim) != Some(dim) {
        return Err(corrupt(format!(
            "pq header subvectors {subvectors} * subdim {subdim} != dim {dim}"
        )));
    }
    Ok(())
}

fn train_subspace(
    rows: &[(u32, Vec<f32>)],
    offset: usize,
    subdim: usize,
    centroids: usize,
    iterations: usize,
    out: &mut Vec<f32>,
) {
    let n = rows.len();
    let start = out.len();
    for centroid in 0..centroids {
        let row = &rows[centroid * n / centroids].1;
        out.extend_from_slice(&row[offset..offset + subdim]);
    }
    let mut sums = vec![0.0; centroids * subdim];
    let mut counts = vec![0_usize; centroids];
    for _ in 0..iterations {
        sums.fill(0.0);
        counts.fill(0);
        for (_, vector) in rows {
            let values = &vector[offset..offset + subdim];
            let nearest = nearest_in_codebook(values, &out[start..], centroids, subdim);
            counts[nearest] += 1;
            let sum_at = nearest * subdim;
            for (dst, src) in sums[sum_at..sum_at + subdim].iter_mut().zip(values) {
                *dst += *src;
            }
        }
        for (centroid, count) in counts.iter().copied().enumerate().take(centroids) {
            if count == 0 {
                continue;
            }
            let dst_at = start + centroid * subdim;
            let sum_at = centroid * subdim;
            for axis in 0..subdim {
                out[dst_at + axis] = sums[sum_at + axis] / count as f32;
            }
        }
    }
}

fn nearest_in_codebook(values: &[f32], codebook: &[f32], centroids: usize, subdim: usize) -> usize {
    (0..centroids)
        .min_by(|&a, &b| {
            let ca = &codebook[a * subdim..a * subdim + subdim];
            let cb = &codebook[b * subdim..b * subdim + subdim];
            l2_sq(values, ca).total_cmp(&l2_sq(values, cb))
        })
        .unwrap_or(0)
}

fn l2_sq(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

fn le_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4B"))
}

fn le_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8B"))
}

fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_INVALID_PARAMS,
        format!("diskann pq invalid params: {detail}"),
    )
}

fn corrupt(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_CORRUPT, format!("diskann pq corrupt: {detail}"))
}

fn io(stage: &str, error: std::io::Error) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_IO, format!("diskann pq {stage}: {error}"))
}
