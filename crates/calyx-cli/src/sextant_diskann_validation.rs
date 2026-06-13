mod support;

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::time::Instant;

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::index::{DiskAnnSearch, SextantIndex};
use serde::Serialize;

use support::{
    Mode, Paths, Request, approx_rows, build_params, cx, dir_bytes, exact_top_k, file_len,
    percentile, rank_of, raw_vectors, search_params, write_json, write_raw_sidecar,
};

const SLOT: SlotId = SlotId::new(0);
#[derive(Serialize)]
struct Summary {
    mode: String,
    root: String,
    graph_path: String,
    raw_dir: String,
    metrics_dir: String,
    node_count: usize,
    dim: usize,
    query_count: usize,
    k: usize,
    beamwidth: usize,
    ef_search: usize,
    rescore_k: usize,
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
    hits_tsv: String,
}

#[derive(Serialize)]
struct EdgeReport {
    mode: String,
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

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let request = Request::parse(args)?;
    match request.mode {
        Mode::Happy => run_happy(&request),
        Mode::Empty => run_empty_edge(&request),
        Mode::DimMismatch => run_dim_mismatch_edge(&request),
        Mode::Truncated => run_truncated_edge(&request),
        Mode::MissingRaw => run_missing_raw_edge(&request),
    }
}

fn run_happy(request: &Request) -> Result<(), String> {
    let paths = Paths::create(&request.root)?;
    let raw = raw_vectors(request.nodes, request.dim);
    let approx = approx_rows(&raw);
    write_raw_sidecar(&paths.raw_dir, &raw)?;
    let index = DiskAnnSearch::build(
        SLOT,
        &paths.graph_path,
        &approx,
        build_params(request),
        Some(paths.raw_dir.clone()),
        search_params(request),
    )
    .map_err(|error| error.to_string())?;
    let mut latencies = Vec::with_capacity(request.queries);
    let mut recalls = Vec::with_capacity(request.queries);
    let mut hits_tsv = String::from("query_id\trank\tnode_id\tdistance\texact_top10\n");
    for q in 0..request.queries {
        let query_id = (q * 17 + 7) % request.nodes;
        let exact = exact_top_k(&raw, query_id, request.k);
        let exact_ids: BTreeSet<_> = exact.iter().map(|(id, _)| *id).collect();
        let started = Instant::now();
        let hits = index
            .search_ids(&raw[query_id].1, request.k, &search_params(request))
            .map_err(|error| error.to_string())?;
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
    let node7 = index
        .search_ids(&raw[7].1, request.k, &search_params(request))
        .map_err(|error| error.to_string())?;
    let trait_hits = index
        .search(
            &SlotVector::Dense {
                dim: request.dim as u32,
                data: raw[7].1.clone(),
            },
            request.k,
            Some(request.ef_search),
        )
        .map_err(|error| error.to_string())?;
    let hits_path = paths.metrics_dir.join("diskann_hits.tsv");
    fs::write(&hits_path, hits_tsv).map_err(|error| error.to_string())?;
    let summary = Summary {
        mode: "happy".to_string(),
        root: request.root.display().to_string(),
        graph_path: paths.graph_path.display().to_string(),
        raw_dir: paths.raw_dir.display().to_string(),
        metrics_dir: paths.metrics_dir.display().to_string(),
        node_count: request.nodes,
        dim: request.dim,
        query_count: request.queries,
        k: request.k,
        beamwidth: request.beamwidth,
        ef_search: request.ef_search,
        rescore_k: request.rescore_k,
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
        raw_file_count: fs::read_dir(&paths.raw_dir)
            .map_err(|error| error.to_string())?
            .count(),
        raw_bytes_total: dir_bytes(&paths.raw_dir)?,
        hits_tsv: hits_path.display().to_string(),
    };
    write_json(
        &paths.metrics_dir.join("diskann_search_summary.json"),
        &summary,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn run_empty_edge(request: &Request) -> Result<(), String> {
    let paths = Paths::for_root(&request.root);
    let before = file_len(&paths.graph_path);
    let err = DiskAnnSearch::build(
        SLOT,
        &paths.graph_path,
        &[],
        build_params(request),
        None,
        search_params(request),
    )
    .expect_err("empty graph build must fail closed");
    write_edge(
        &request.root,
        "empty",
        before,
        file_len(&paths.graph_path),
        err.code,
        &err.message,
    )
}

fn run_dim_mismatch_edge(request: &Request) -> Result<(), String> {
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
        before,
        file_len(&paths.graph_path),
        err.code,
        &err.message,
    )
}

fn run_truncated_edge(request: &Request) -> Result<(), String> {
    let paths = Paths::create(&request.root)?;
    let raw = raw_vectors(32, request.dim);
    write_raw_sidecar(&paths.raw_dir, &raw)?;
    let _ = build_edge_index(request, &paths, &raw)?;
    let before = file_len(&paths.graph_path);
    OpenOptions::new()
        .write(true)
        .open(&paths.graph_path)
        .map_err(|error| error.to_string())?
        .set_len(before.unwrap_or(0) / 2)
        .map_err(|error| error.to_string())?;
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
        before,
        file_len(&paths.graph_path),
        err.code,
        &err.message,
    )
}

fn run_missing_raw_edge(request: &Request) -> Result<(), String> {
    let paths = Paths::create(&request.root)?;
    let raw = raw_vectors(32, request.dim);
    write_raw_sidecar(&paths.raw_dir, &raw)?;
    fs::remove_file(paths.raw_dir.join("7")).map_err(|error| error.to_string())?;
    let index = build_edge_index(request, &paths, &raw)?;
    let before = file_len(&paths.graph_path);
    let err = index
        .search_ids(&raw[7].1, request.k, &search_params(request))
        .expect_err("missing raw sidecar must fail closed");
    write_edge(
        &request.root,
        "missing-raw",
        before,
        file_len(&paths.graph_path),
        err.code,
        &err.message,
    )
}

fn build_edge_index(
    request: &Request,
    paths: &Paths,
    raw: &[(CxId, Vec<f32>)],
) -> Result<DiskAnnSearch, String> {
    DiskAnnSearch::build(
        SLOT,
        &paths.graph_path,
        &approx_rows(raw),
        build_params(request),
        Some(paths.raw_dir.clone()),
        search_params(request),
    )
    .map_err(|error| error.to_string())
}

fn write_edge(
    root: &Path,
    mode: &str,
    before: Option<u64>,
    after: Option<u64>,
    code: &'static str,
    message: &str,
) -> Result<(), String> {
    let paths = Paths::create(root)?;
    let report = EdgeReport {
        mode: mode.to_string(),
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
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn expected_error(mode: &str) -> &'static str {
    match mode {
        "empty" => "CALYX_INDEX_INVALID_PARAMS",
        "dim-mismatch" => "CALYX_INDEX_DIM_MISMATCH",
        "truncated" | "missing-raw" => "CALYX_INDEX_IO",
        _ => "CALYX_INDEX_INVALID_PARAMS",
    }
}
