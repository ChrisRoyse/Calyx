use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::CxId;
use calyx_sextant::index::{
    DiskAnnBuildBackend, DiskAnnBuildParams, DiskAnnPqBuildParams, DiskAnnSearchParams,
};
use serde::Serialize;

use crate::error::{CliError, CliResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Happy,
    Empty,
    DimMismatch,
    Truncated,
    MissingRaw,
    CorruptPq,
}

#[derive(Clone, Debug)]
pub(super) struct Request {
    pub(super) root: PathBuf,
    pub(super) mode: Mode,
    pub(super) nodes: usize,
    pub(super) dim: usize,
    pub(super) queries: usize,
    pub(super) k: usize,
    pub(super) beamwidth: usize,
    pub(super) ef_search: usize,
    pub(super) rescore_k: usize,
    pub(super) recall_floor: Option<f64>,
    pub(super) pq: Option<DiskAnnPqBuildParams>,
    pub(super) build_backend: DiskAnnBuildBackend,
}

#[derive(Clone)]
pub(super) struct Paths {
    pub(super) graph_path: PathBuf,
    pub(super) raw_dir: PathBuf,
    pub(super) pq_path: PathBuf,
    pub(super) metrics_dir: PathBuf,
}

impl Paths {
    pub(super) fn for_root(root: &Path) -> Self {
        Self {
            graph_path: root.join("idx/slot_00.ann/graph.cda"),
            raw_dir: root.join("cf/slot_00.raw"),
            pq_path: root.join("idx/slot_00.ann/graph.pq"),
            metrics_dir: root.join("metrics"),
        }
    }

    pub(super) fn create(root: &Path) -> CliResult<Self> {
        let paths = Self::for_root(root);
        if let Some(parent) = paths.graph_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir_all(&paths.raw_dir)?;
        fs::create_dir_all(&paths.metrics_dir)?;
        Ok(paths)
    }
}

impl Request {
    pub(super) fn parse(args: &[String]) -> Result<Self, String> {
        let root = required_path(args, "--root")?;
        let mode = match value(args, "--mode").unwrap_or("happy") {
            "happy" => Mode::Happy,
            "empty" => Mode::Empty,
            "dim-mismatch" => Mode::DimMismatch,
            "truncated" => Mode::Truncated,
            "missing-raw" => Mode::MissingRaw,
            "corrupt-pq" => Mode::CorruptPq,
            other => return Err(format!("unknown diskann validation mode: {other}")),
        };
        let nodes = number(args, "--nodes", 1000)?;
        let dim = number(args, "--dim", 64)?;
        let queries = number(args, "--queries", 128)?;
        let k = number(args, "--k", 10)?;
        let recall_floor = recall_floor(args)?;
        if mode == Mode::Happy && recall_floor.is_none() {
            return Err(
                "CALYX_FSV_DISKANN_INVALID_CONFIG: happy mode requires --recall-floor in (0, 1]"
                    .to_string(),
            );
        }
        if mode == Mode::Happy && nodes < 8 {
            return Err(
                "CALYX_FSV_DISKANN_INVALID_CONFIG: --nodes must be at least 8 in happy mode"
                    .to_string(),
            );
        }
        Ok(Self {
            root,
            mode,
            nodes,
            dim,
            queries,
            k,
            beamwidth: number(args, "--beamwidth", 32)?,
            ef_search: number(args, "--ef-search", 128)?.max(k),
            rescore_k: number(args, "--rescore-k", 64)?.max(k),
            recall_floor,
            pq: pq_params(args)?,
            build_backend: build_backend(args)?,
        })
    }
}

pub(super) fn raw_vectors(nodes: usize, dim: usize) -> Vec<(CxId, Vec<f32>)> {
    (0..nodes).map(|id| (cx(id), raw_vector(id, dim))).collect()
}

fn raw_vector(id: usize, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|axis| {
            let anchor = if axis == id % dim { 3.0 } else { 0.0 };
            let wave = (((id * 31 + axis * 17) % 101) as f32 / 50.0) - 1.0;
            anchor + wave * 0.05 + (axis % 7) as f32 * 0.001
        })
        .collect()
}

pub(super) fn approx_rows(raw: &[(CxId, Vec<f32>)]) -> Vec<(CxId, Vec<f32>)> {
    raw.iter()
        .enumerate()
        .map(|(id, (cx_id, vector))| {
            let approx = vector
                .iter()
                .enumerate()
                .map(|(axis, value)| value + (((id + axis * 13) % 11) as f32 - 5.0) * 0.001)
                .collect();
            (*cx_id, approx)
        })
        .collect()
}

pub(super) fn write_raw_sidecar(raw_dir: &Path, raw: &[(CxId, Vec<f32>)]) -> CliResult {
    for (id, (_, vector)) in raw.iter().enumerate() {
        let bytes: Vec<_> = vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        fs::write(raw_dir.join(id.to_string()), bytes)?;
    }
    Ok(())
}

pub(super) fn exact_top_k(raw: &[(CxId, Vec<f32>)], query_id: usize, k: usize) -> Vec<(u32, f32)> {
    let query = &raw[query_id].1;
    let mut exact: Vec<_> = raw
        .iter()
        .enumerate()
        .map(|(id, (_, vector))| (id as u32, distance(query, vector)))
        .collect();
    exact.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    exact.truncate(k.min(exact.len()));
    exact
}

pub(super) fn build_params(request: &Request) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim: request.dim,
        m_max: 16,
        ef_construction: request.ef_search.max(64),
        alpha: 1.2,
    }
}

pub(super) fn search_params(request: &Request) -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: request.beamwidth,
        ef_search: request.ef_search,
        rescore_k: request.rescore_k,
        rescore_from_raw: true,
    }
}

fn distance(a: &[f32], b: &[f32]) -> f32 {
    let (dot, aa, bb) = a
        .iter()
        .zip(b)
        .fold((0.0_f32, 0.0_f32, 0.0_f32), |(dot, aa, bb), (x, y)| {
            (dot + x * y, aa + x * x, bb + y * y)
        });
    if aa == 0.0 || bb == 0.0 {
        1.0
    } else {
        (1.0 - dot / (aa.sqrt() * bb.sqrt())).max(0.0)
    }
}

pub(super) fn percentile(values: &[u128], pct: usize) -> u128 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let idx = (sorted.len() * pct).div_ceil(100).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}

pub(super) fn rank_of(hits: &[(u32, f32)], id: u32) -> usize {
    hits.iter()
        .position(|(hit, _)| *hit == id)
        .map(|idx| idx + 1)
        .unwrap_or(usize::MAX)
}

pub(super) fn cx(idx: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..].copy_from_slice(&(idx as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

pub(super) fn file_len(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|metadata| metadata.len())
}

pub(super) fn dir_bytes(path: &Path) -> CliResult<u64> {
    fs::read_dir(path)?
        .map(|entry| -> CliResult<u64> { Ok(entry?.metadata()?.len()) })
        .sum()
}

pub(super) fn write_json<T: Serialize>(path: &Path, value: &T) -> CliResult {
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::runtime(format!("serialize {}: {error}", path.display())))?;
    Ok(fs::write(path, json)?)
}

fn required_path(args: &[String], flag: &str) -> Result<PathBuf, String> {
    value(args, flag)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} is required"))
}

fn number(args: &[String], flag: &str, default: usize) -> Result<usize, String> {
    optional_number(args, flag)?.map_or(Ok(default), Ok)
}

fn optional_number(args: &[String], flag: &str) -> Result<Option<usize>, String> {
    value(args, flag)
        .map(str::parse)
        .transpose()
        .map_err(|error| format!("{flag}: {error}"))?
        .map_or(Ok(None), |n| {
            if n == 0 {
                Err(format!("{flag} must be positive"))
            } else {
                Ok(Some(n))
            }
        })
}

fn recall_floor(args: &[String]) -> Result<Option<f64>, String> {
    let Some(raw) = value(args, "--recall-floor") else {
        return Ok(None);
    };
    let floor: f64 = raw
        .parse()
        .map_err(|error| format!("--recall-floor: {error}"))?;
    if !(floor.is_finite() && 0.0 < floor && floor <= 1.0) {
        return Err("--recall-floor must be finite in (0, 1]".to_string());
    }
    Ok(Some(floor))
}

fn pq_params(args: &[String]) -> Result<Option<DiskAnnPqBuildParams>, String> {
    let Some(subvectors) = optional_number(args, "--pq-subvectors")? else {
        return Ok(None);
    };
    Ok(Some(DiskAnnPqBuildParams {
        subvectors,
        centroids: optional_number(args, "--pq-centroids")?.unwrap_or(256),
        iterations: optional_number(args, "--pq-iterations")?.unwrap_or(8),
    }))
}

fn build_backend(args: &[String]) -> Result<DiskAnnBuildBackend, String> {
    value(args, "--build-backend")
        .map(str::parse)
        .transpose()
        .map(|backend| backend.unwrap_or(DiskAnnBuildBackend::CpuVamana))
}

fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn parses_recall_floor() {
        let args = strings(&["--root", "vault", "--recall-floor", "0.85"]);
        let request = Request::parse(&args).expect("parse");
        assert_eq!(request.recall_floor, Some(0.85));
    }

    #[test]
    fn happy_mode_requires_positive_recall_floor() {
        let missing = Request::parse(&strings(&["--root", "vault"]))
            .expect_err("happy mode without recall floor");
        assert!(missing.contains("happy mode requires --recall-floor"));

        let zero = Request::parse(&strings(&["--root", "vault", "--recall-floor", "0"]))
            .expect_err("zero recall floor");
        assert!(zero.contains("--recall-floor must be finite in (0, 1]"));
    }

    #[test]
    fn happy_mode_requires_node_seven_for_durable_probe() {
        let too_few = Request::parse(&strings(&[
            "--root",
            "vault",
            "--nodes",
            "7",
            "--recall-floor",
            "0.9",
        ]))
        .expect_err("node 7 probe requires at least eight nodes");
        assert!(too_few.contains("--nodes must be at least 8 in happy mode"));

        let enough = Request::parse(&strings(&[
            "--root",
            "vault",
            "--nodes",
            "8",
            "--recall-floor",
            "0.9",
        ]))
        .expect("eight nodes include node 7");
        assert_eq!(enough.nodes, 8);
    }

    #[test]
    fn rejects_zero_nodes_before_happy_mode_execution() {
        let error = Request::parse(&strings(&[
            "--root",
            "vault",
            "--nodes",
            "0",
            "--recall-floor",
            "0.9",
        ]))
        .expect_err("zero nodes");
        assert!(error.contains("--nodes must be positive"));
    }

    #[test]
    fn rejects_invalid_recall_floor() {
        let args = strings(&["--root", "vault", "--recall-floor", "1.1"]);
        let error = Request::parse(&args).expect_err("recall floor > 1");
        assert!(error.contains("--recall-floor must be finite in (0, 1]"));
    }

    #[test]
    fn parses_pq_params() {
        let args = strings(&[
            "--root",
            "vault",
            "--recall-floor",
            "0.9",
            "--pq-subvectors",
            "4",
            "--pq-centroids",
            "16",
            "--pq-iterations",
            "3",
        ]);
        let request = Request::parse(&args).expect("parse");
        assert_eq!(
            request.pq,
            Some(DiskAnnPqBuildParams {
                subvectors: 4,
                centroids: 16,
                iterations: 3
            })
        );
    }

    #[test]
    fn parses_build_backend() {
        let args = strings(&[
            "--root",
            "vault",
            "--recall-floor",
            "0.9",
            "--build-backend",
            "cuvs-cagra",
        ]);
        let request = Request::parse(&args).expect("parse");
        assert_eq!(request.build_backend, DiskAnnBuildBackend::CuvsCagra);
    }
}
