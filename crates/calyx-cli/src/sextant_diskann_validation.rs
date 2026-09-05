mod issue604;
mod support;

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::time::Instant;

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::index::{
    DiskAnnPqBuildParams, DiskAnnPqSearchBuild, DiskAnnSearch, SextantIndex,
};
use serde::Serialize;

use crate::error::CliError;
use support::{
    Mode, Paths, Request, approx_rows, build_params, cx, exact_top_k, file_len, percentile,
    rank_of, raw_vectors, search_params, write_json, write_legacy_raw_sidecar_dir,
    write_raw_sidecar,
};

const SLOT: SlotId = SlotId::new(0);
#[derive(Serialize)]
struct Summary {
    mode: String,
    build_backend: String,
    root: String,
    graph_path: String,
    raw_dir: String,
    pq_path: Option<String>,
    metrics_dir: String,
    node_count: usize,
    dim: usize,
    query_count: usize,
    k: usize,
    beamwidth: usize,
    ef_search: usize,
    rescore_k: usize,
    recall_floor: Option<f64>,
    recall_at_10_avg: f64,
    recall_at_10_min: f64,
    p50_us: u128,
    p99_us: u128,
    exact_query_node7_rank: usize,
    exact_query_node7_distance: f32,
    trait_top_rank: usize,
    trait_top_cx_id: String,
    graph_bytes: u64,
    raw_file_count: usize,
    raw_bytes_total: u64,
    pq_bytes: Option<u64>,
    pq_ram_bytes: Option<usize>,
    pq_subvectors: Option<usize>,
    pq_centroids: Option<usize>,
    pq_build_diagnostics: Option<calyx_sextant::index::DiskAnnPqBuildDiagnostics>,
    hits_tsv: String,
}

#[derive(Serialize)]
struct EdgeReport {
    mode: String,
    build_backend: String,
    root: String,
    graph_path: String,
    before_graph_exists: bool,
    after_graph_exists: bool,
    before_graph_bytes: Option<u64>,
    after_graph_bytes: Option<u64>,
    expected_error: String,
    observed_error: String,
    observed_message: String,
}

#[derive(Serialize)]
struct LegacyDirReport {
    mode: String,
    build_backend: String,
    root: String,
    raw_dir: String,
    raw_is_dir: bool,
    node_count: usize,
    node7_rank: usize,
    hits: usize,
}

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    if issue604::is_issue604(args) {
        return issue604::run(args);
    }
    let request = Request::parse(args).map_err(CliError::usage)?;
    match request.mode {
        Mode::Happy => run_happy(&request),
        Mode::Empty => run_empty_edge(&request),
        Mode::DimMismatch => run_dim_mismatch_edge(&request),
        Mode::Truncated => run_truncated_edge(&request),
        Mode::TruncatedRaw => run_truncated_raw_edge(&request),
        Mode::LegacyDir => run_legacy_dir_edge(&request),
        Mode::CorruptPq => run_corrupt_pq_edge(&request),
    }
}

fn run_happy(request: &Request) -> crate::error::CliResult {
    let paths = Paths::create(&request.root)?;
    let raw = raw_vectors(request.nodes, request.dim);
    let approx = approx_rows(&raw);
    write_raw_sidecar(&paths.raw_dir, &raw)?;
    let index = build_index(request, &paths, &approx)?;
    let mut latencies = Vec::with_capacity(request.queries);
    let mut recalls = Vec::with_capacity(request.queries);
    let mut hits_tsv = String::from("query_id\trank\tnode_id\tdistance\texact_top10\n");
    for q in 0..request.queries {
        let query_id = (q * 17 + 7) % request.nodes;
        let exact = exact_top_k(&raw, query_id, request.k);
        let exact_ids: BTreeSet<_> = exact.iter().map(|(id, _)| *id).collect();
        let started = Instant::now();
        let hits = index.search_ids(&raw[query_id].1, request.k, &search_params(request))?;
        latencies.push(started.elapsed().as_micros());
        let got_ids: BTreeSet<_> = hits.iter().map(|(id, _)| *id).collect();
        let overlap = got_ids.intersection(&exact_ids).count();
        recalls.push(overlap as f64 / exact_ids.len() as f64);
        for (rank, (node_id, distance)) in hits.iter().enumerate() {
            hits_tsv.push_str(&format!(
                "{query_id}\t{}\t{node_id}\t{distance:.8}\t{}\n",
                rank + 1,
                exact_ids.contains(node_id)
            ));
        }
    }
    let node7 = index.search_ids(&raw[7].1, request.k, &search_params(request))?;
    let trait_hits = index.search(
        &SlotVector::Dense {
            dim: request.dim as u32,
            data: raw[7].1.clone(),
        },
        request.k,
        Some(request.ef_search),
    )?;
    let hits_path = paths.metrics_dir.join("diskann_hits.tsv");
    fs::write(&hits_path, hits_tsv)?;
    let summary = Summary {
        mode: "happy".to_string(),
        build_backend: request.build_backend.as_str().to_string(),
        root: request.root.display().to_string(),
        graph_path: paths.graph_path.display().to_string(),
        raw_dir: paths.raw_dir.display().to_string(),
        pq_path: request
            .pq
            .is_some()
            .then(|| paths.pq_path.display().to_string()),
        metrics_dir: paths.metrics_dir.display().to_string(),
        node_count: request.nodes,
        dim: request.dim,
        query_count: request.queries,
        k: request.k,
        beamwidth: request.beamwidth,
        ef_search: request.ef_search,
        rescore_k: request.rescore_k,
        recall_floor: request.recall_floor,
        recall_at_10_avg: recalls.iter().sum::<f64>() / recalls.len() as f64,
        recall_at_10_min: recalls.iter().copied().fold(f64::INFINITY, f64::min),
        p50_us: percentile(&latencies, 50),
        p99_us: percentile(&latencies, 99),
        exact_query_node7_rank: rank_of(&node7, 7),
        exact_query_node7_distance: node7
            .iter()
            .find(|(id, _)| *id == 7)
            .map(|(_, distance)| *distance)
            .unwrap_or(f32::INFINITY),
        trait_top_rank: trait_hits.first().map(|hit| hit.rank).unwrap_or(usize::MAX),
        trait_top_cx_id: trait_hits
            .first()
            .map(|hit| hit.cx_id.to_string())
            .unwrap_or_else(|| "none".to_string()),
        graph_bytes: file_len(&paths.graph_path).unwrap_or(0),
        // Packed v2 sidecar (#1990): a single regular file, not a dir of files.
        raw_file_count: 1,
        raw_bytes_total: file_len(&paths.raw_dir).unwrap_or(0),
        pq_bytes: file_len(&paths.pq_path),
        pq_ram_bytes: index.pq_ram_bytes(),
        pq_subvectors: index.pq_summary().map(|(_, subvectors, _)| subvectors),
        pq_centroids: index.pq_summary().map(|(_, _, centroids)| centroids),
        pq_build_diagnostics: index.pq_build_diagnostics().cloned(),
        hits_tsv: hits_path.display().to_string(),
    };
    let summary_path = paths.metrics_dir.join("diskann_search_summary.json");
    write_json(&summary_path, &summary)?;
    enforce_recall_floor(&summary, request.recall_floor, &summary_path, &hits_path)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&summary)
            .map_err(|error| CliError::runtime(format!("serialize summary: {error}")))?
    );
    Ok(())
}

fn enforce_recall_floor(
    summary: &Summary,
    floor: Option<f64>,
    summary_path: &Path,
    hits_path: &Path,
) -> crate::error::CliResult {
    let Some(floor) = floor else {
        return Err(CliError::usage(
            "CALYX_FSV_DISKANN_INVALID_CONFIG: happy mode requires --recall-floor in (0, 1]",
        ));
    };
    if summary.recall_at_10_min + f64::EPSILON < floor {
        return Err(CliError::runtime(format!(
            "CALYX_FSV_DISKANN_RECALL_BELOW_FLOOR: recall_at_10_min={:.6} recall_floor={:.6} summary={} hits={}",
            summary.recall_at_10_min,
            floor,
            summary_path.display(),
            hits_path.display()
        )));
    }
    Ok(())
}

fn run_empty_edge(request: &Request) -> crate::error::CliResult {
    let paths = Paths::for_root(&request.root);
    let before = file_len(&paths.graph_path);
    let err = DiskAnnSearch::build_with_backend(
        SLOT,
        &paths.graph_path,
        &[],
        build_params(request),
        None,
        search_params(request),
        request.build_backend,
    )
    .expect_err("empty graph build must fail closed");
    write_edge(
        &request.root,
        "empty",
        request.build_backend.as_str(),
        before,
        file_len(&paths.graph_path),
        err.code,
        &err.message,
    )
}

fn run_dim_mismatch_edge(request: &Request) -> crate::error::CliResult {
    let paths = Paths::create(&request.root)?;
    let raw = raw_vectors(32, request.dim);
    write_raw_sidecar(&paths.raw_dir, &raw)?;
    let index = build_edge_index(request, &paths, &raw)?;
    let before = file_len(&paths.graph_path);
    let err = index
        .search_ids(
            &raw[7].1[..raw[7].1.len() - 1],
            request.k,
            &search_params(request),
        )
        .expect_err("dim mismatch must fail closed");
    write_edge(
        &request.root,
        "dim-mismatch",
        request.build_backend.as_str(),
        before,
        file_len(&paths.graph_path),
        err.code,
        &err.message,
    )
}

fn run_truncated_edge(request: &Request) -> crate::error::CliResult {
    let paths = Paths::create(&request.root)?;
    let raw = raw_vectors(32, request.dim);
    write_raw_sidecar(&paths.raw_dir, &raw)?;
    let _ = build_edge_index(request, &paths, &raw)?;
    let before = file_len(&paths.graph_path);
    OpenOptions::new()
        .write(true)
        .open(&paths.graph_path)?
        .set_len(before.unwrap_or(0) / 2)?;
    let err = DiskAnnSearch::open(
        SLOT,
        &paths.graph_path,
        (0..32).map(cx).collect(),
        Some(paths.raw_dir.clone()),
        search_params(request),
    )
    .expect_err("truncated graph must fail closed");
    write_edge(
        &request.root,
        "truncated",
        request.build_backend.as_str(),
        before,
        file_len(&paths.graph_path),
        err.code,
        &err.message,
    )
}

/// Build a real packed v2 sidecar, then truncate the packed file on disk and
/// prove the rescore read fails closed (never zero-fills). This replaces the
/// pre-#1990 `missing-raw` edge (removing one per-id file), which no longer
/// exists once the sidecar is a single packed file.
fn run_truncated_raw_edge(request: &Request) -> crate::error::CliResult {
    let paths = Paths::create(&request.root)?;
    let raw = raw_vectors(32, request.dim);
    write_raw_sidecar(&paths.raw_dir, &raw)?;
    let index = build_edge_index(request, &paths, &raw)?;
    let before = file_len(&paths.graph_path);
    // Chop the tail of the packed file so its length no longer matches
    // header + node_count * dim * 4; the reader must reject it on open.
    let raw_len = file_len(&paths.raw_dir).unwrap_or(0);
    OpenOptions::new()
        .write(true)
        .open(&paths.raw_dir)?
        .set_len(raw_len.saturating_sub(4))?;
    let err = index
        .search_ids(&raw[7].1, request.k, &search_params(request))
        .expect_err("truncated packed raw sidecar must fail closed");
    write_edge(
        &request.root,
        "truncated-raw",
        request.build_backend.as_str(),
        before,
        file_len(&paths.graph_path),
        err.code,
        &err.message,
    )
}

/// Prove the versioned reader still reads a legacy v1 directory-of-files
/// sidecar (the real deployed pre-#1990 layout). Builds the graph pointed at a
/// v1 dir, runs a search, and reports the exact-node-7 rank as read-back proof.
fn run_legacy_dir_edge(request: &Request) -> crate::error::CliResult {
    let paths = Paths::create(&request.root)?;
    let raw = raw_vectors(32, request.dim);
    // Materialize the v1 directory layout, then build a graph that references it
    // as the sidecar (the builder leaves an already-populated sidecar in place).
    write_legacy_raw_sidecar_dir(&paths.raw_dir, &raw)?;
    let index = build_edge_index(request, &paths, &raw)?;
    let node7 = index.search_ids(&raw[7].1, request.k, &search_params(request))?;
    let node7_rank = rank_of(&node7, 7);
    if node7_rank == usize::MAX {
        return Err(CliError::runtime(
            "CALYX_FSV_DISKANN_LEGACY_DIR_READ_FAILED: node 7 absent from legacy-dir rescore",
        ));
    }
    let report = LegacyDirReport {
        mode: "legacy-dir".to_string(),
        build_backend: request.build_backend.as_str().to_string(),
        root: request.root.display().to_string(),
        raw_dir: paths.raw_dir.display().to_string(),
        raw_is_dir: paths.raw_dir.is_dir(),
        node_count: raw.len(),
        node7_rank,
        hits: node7.len(),
    };
    let path = paths.metrics_dir.join("diskann_edge_legacy_dir.json");
    write_json(&path, &report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| CliError::runtime(format!("serialize legacy-dir report: {error}")))?
    );
    Ok(())
}

fn build_edge_index(
    request: &Request,
    paths: &Paths,
    raw: &[(CxId, Vec<f32>)],
) -> crate::error::CliResult<DiskAnnSearch> {
    build_index(request, paths, &approx_rows(raw))
}

fn build_index(
    request: &Request,
    paths: &Paths,
    approx: &[(CxId, Vec<f32>)],
) -> crate::error::CliResult<DiskAnnSearch> {
    if let Some(pq) = request.pq {
        return Ok(DiskAnnSearch::build_with_pq_plan(
            SLOT,
            &paths.graph_path,
            approx,
            build_params(request),
            Some(paths.raw_dir.clone()),
            DiskAnnPqSearchBuild {
                search: search_params(request),
                pq,
                backend: request.build_backend,
            },
        )?);
    }
    Ok(DiskAnnSearch::build_with_backend(
        SLOT,
        &paths.graph_path,
        approx,
        build_params(request),
        Some(paths.raw_dir.clone()),
        search_params(request),
        request.build_backend,
    )?)
}

fn run_corrupt_pq_edge(request: &Request) -> crate::error::CliResult {
    let paths = Paths::create(&request.root)?;
    let raw = raw_vectors(64, request.dim);
    write_raw_sidecar(&paths.raw_dir, &raw)?;
    let mut build_request = request.clone();
    if build_request.pq.is_none() {
        build_request.pq = Some(DiskAnnPqBuildParams {
            subvectors: 4,
            centroids: 16,
            iterations: 2,
        });
    }
    let _ = build_edge_index(&build_request, &paths, &raw)?;
    let before = file_len(&paths.graph_path);
    fs::write(&paths.pq_path, b"not-a-pq")?;
    let err = DiskAnnSearch::open(
        SLOT,
        &paths.graph_path,
        (0..64).map(cx).collect(),
        Some(paths.raw_dir.clone()),
        search_params(request),
    )
    .expect_err("corrupt pq sidecar must fail closed");
    write_edge(
        &request.root,
        "corrupt-pq",
        request.build_backend.as_str(),
        before,
        file_len(&paths.graph_path),
        err.code,
        &err.message,
    )
}

fn write_edge(
    root: &Path,
    mode: &str,
    build_backend: &str,
    before: Option<u64>,
    after: Option<u64>,
    code: &'static str,
    message: &str,
) -> crate::error::CliResult {
    let paths = Paths::create(root)?;
    let report = EdgeReport {
        mode: mode.to_string(),
        build_backend: build_backend.to_string(),
        root: root.display().to_string(),
        graph_path: paths.graph_path.display().to_string(),
        before_graph_exists: before.is_some(),
        after_graph_exists: after.is_some(),
        before_graph_bytes: before,
        after_graph_bytes: after,
        expected_error: expected_error(mode).to_string(),
        observed_error: code.to_string(),
        observed_message: message.to_string(),
    };
    let path = paths.metrics_dir.join(format!("diskann_edge_{mode}.json"));
    write_json(&path, &report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| CliError::runtime(format!("serialize edge report: {error}")))?
    );
    Ok(())
}

fn expected_error(mode: &str) -> &'static str {
    match mode {
        "empty" => "CALYX_INDEX_INVALID_PARAMS",
        "dim-mismatch" => "CALYX_INDEX_DIM_MISMATCH",
        "truncated" => "CALYX_INDEX_CORRUPT",
        "truncated-raw" => "CALYX_INDEX_CORRUPT",
        "corrupt-pq" => "CALYX_INDEX_CORRUPT",
        _ => "CALYX_INDEX_INVALID_PARAMS",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary_with_recall(min_recall: f64) -> Summary {
        Summary {
            mode: "happy".to_string(),
            build_backend: "cpu-vamana".to_string(),
            root: "root".to_string(),
            graph_path: "root/idx/slot_00.ann/graph.cda".to_string(),
            raw_dir: "root/cf/slot_00.raw".to_string(),
            pq_path: None,
            metrics_dir: "root/metrics".to_string(),
            node_count: 10,
            dim: 4,
            query_count: 1,
            k: 10,
            beamwidth: 32,
            ef_search: 64,
            rescore_k: 64,
            recall_floor: Some(0.9),
            recall_at_10_avg: min_recall,
            recall_at_10_min: min_recall,
            p50_us: 1,
            p99_us: 1,
            exact_query_node7_rank: 1,
            exact_query_node7_distance: 0.0,
            trait_top_rank: 1,
            trait_top_cx_id: "00000000000000000000000000000007".to_string(),
            graph_bytes: 1,
            raw_file_count: 10,
            raw_bytes_total: 160,
            pq_bytes: None,
            pq_ram_bytes: None,
            pq_subvectors: None,
            pq_centroids: None,
            pq_build_diagnostics: None,
            hits_tsv: "root/metrics/diskann_hits.tsv".to_string(),
        }
    }

    #[test]
    fn recall_floor_rejects_low_happy_recall() {
        let error = enforce_recall_floor(
            &summary_with_recall(0.25),
            Some(0.9),
            Path::new("summary.json"),
            Path::new("hits.tsv"),
        )
        .expect_err("low recall should fail");
        assert!(
            error
                .to_string()
                .contains("CALYX_FSV_DISKANN_RECALL_BELOW_FLOOR")
        );
    }

    #[test]
    fn recall_floor_rejects_missing_happy_floor() {
        let error = enforce_recall_floor(
            &summary_with_recall(1.0),
            None,
            Path::new("summary.json"),
            Path::new("hits.tsv"),
        )
        .expect_err("missing happy recall floor");
        assert!(
            error
                .to_string()
                .contains("happy mode requires --recall-floor")
        );
    }

    #[test]
    fn recall_floor_accepts_floor_value() {
        enforce_recall_floor(
            &summary_with_recall(0.9),
            Some(0.9),
            Path::new("summary.json"),
            Path::new("hits.tsv"),
        )
        .expect("recall at floor");
    }
}
