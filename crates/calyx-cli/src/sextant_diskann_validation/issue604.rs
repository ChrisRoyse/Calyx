use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::index::{
    ConcatCrossTermDiskAnn, ConcatCrossTermKey, DiskAnnBuildParams, DiskAnnSearchParams,
    MaxSimIndex, SextantIndex, TokenDiskAnnMaxSim,
};
use serde::Serialize;

#[derive(Clone, Debug)]
struct Request {
    root: PathBuf,
    docs: usize,
    concat_rows: usize,
    tokens_per_doc: usize,
    dim: usize,
    queries: usize,
    k: usize,
    recall_target: f64,
    beamwidth: usize,
    ef_search: usize,
    rescore_k: usize,
}

#[derive(Serialize)]
struct Summary {
    root: String,
    token_graph_path: String,
    token_docs_path: String,
    token_map_path: String,
    token_bytes_path: String,
    concat_graph_path: String,
    concat_keys_path: String,
    docs: usize,
    token_count: usize,
    concat_rows: usize,
    dim: usize,
    queries: usize,
    k: usize,
    recall_target: f64,
    token_recall_avg: f64,
    token_recall_min: f64,
    concat_recall_avg: f64,
    concat_recall_min: f64,
    token_graph_bytes: u64,
    concat_graph_bytes: u64,
    sidecar_bytes: u64,
    hits_tsv: String,
    edge_report: String,
}

#[derive(Serialize)]
struct EdgeReport {
    case: String,
    before_exists: bool,
    after_exists: bool,
    expected_error: String,
    observed_error: String,
    observed_message: String,
}

pub(super) fn is_issue604(args: &[String]) -> bool {
    value(args, "--mode") == Some("issue604")
}

pub(super) fn run(args: &[String]) -> crate::error::CliResult {
    let request = Request::parse(args)?;
    let paths = Paths::create(&request.root)?;
    let token_rows = token_rows(request.docs, request.tokens_per_doc, request.dim);
    let concat_rows = concat_rows(request.concat_rows, request.dim);
    let params = build_params(&request);
    let search = search_params(&request);
    let token = TokenDiskAnnMaxSim::build(
        SlotId::new(0),
        &paths.token_root,
        &token_rows,
        params,
        search,
    )
    .map_err(|error| error.to_string())?;
    let concat = ConcatCrossTermDiskAnn::build(&paths.concat_root, &concat_rows, params, search)
        .map_err(|error| error.to_string())?;
    let mut hits = String::from("kind\tquery_id\trecall\ttop_id\tlatency_us\n");
    let (token_recall, _token_latency) = token_recall(&request, &token, &token_rows, &mut hits)?;
    let (concat_recall, _concat_latency) =
        concat_recall(&request, &concat, &concat_rows, &mut hits)?;
    let hits_path = paths.metrics_dir.join("issue604_hits.tsv");
    fs::write(&hits_path, hits).map_err(|error| error.to_string())?;
    let edge_path = paths.metrics_dir.join("issue604_edges.json");
    write_json(&edge_path, &run_edges(&paths.edge_root, &request)?)?;
    let summary = Summary {
        root: request.root.display().to_string(),
        token_graph_path: paths.token_root.join("graph.cda").display().to_string(),
        token_docs_path: paths.token_root.join("docs.cdt").display().to_string(),
        token_map_path: paths
            .token_root
            .join("token_docs.u32")
            .display()
            .to_string(),
        token_bytes_path: paths.token_root.join("tokens.f32").display().to_string(),
        concat_graph_path: paths.concat_root.join("graph.cda").display().to_string(),
        concat_keys_path: paths.concat_root.join("keys.cdx").display().to_string(),
        docs: request.docs,
        token_count: request.docs * request.tokens_per_doc,
        concat_rows: request.concat_rows,
        dim: request.dim,
        queries: request.queries,
        k: request.k,
        recall_target: request.recall_target,
        token_recall_avg: avg(&token_recall),
        token_recall_min: min(&token_recall),
        concat_recall_avg: avg(&concat_recall),
        concat_recall_min: min(&concat_recall),
        token_graph_bytes: file_len(&paths.token_root.join("graph.cda")),
        concat_graph_bytes: file_len(&paths.concat_root.join("graph.cda")),
        sidecar_bytes: dir_bytes(&paths.token_root)?
            + file_len(&paths.concat_root.join("keys.cdx")),
        hits_tsv: hits_path.display().to_string(),
        edge_report: edge_path.display().to_string(),
    };
    if summary.token_recall_min < request.recall_target
        || summary.concat_recall_min < request.recall_target
    {
        return Err(format!(
            "recall below target: token_min={} concat_min={} target={}",
            summary.token_recall_min, summary.concat_recall_min, request.recall_target
        )
        .into());
    }
    write_json(&paths.metrics_dir.join("issue604_summary.json"), &summary)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn token_recall(
    request: &Request,
    index: &TokenDiskAnnMaxSim,
    rows: &[(CxId, Vec<Vec<f32>>)],
    hits: &mut String,
) -> Result<(Vec<f64>, Vec<u128>), String> {
    let mut recalls = Vec::new();
    let mut latencies = Vec::new();
    for q in query_ids(request.queries, rows.len()) {
        let exact = exact_token(rows, q, request.k);
        let started = Instant::now();
        let got = index
            .search(
                &SlotVector::Multi {
                    token_dim: request.dim as u32,
                    tokens: rows[q].1.clone(),
                },
                request.k,
                Some(request.ef_search),
            )
            .map_err(|error| error.to_string())?;
        latencies.push(started.elapsed().as_micros());
        let got_ids: BTreeSet<_> = got.iter().map(|hit| hit.cx_id).collect();
        let recall = overlap(&exact, &got_ids);
        recalls.push(recall);
        hits.push_str(&format!(
            "token\t{q}\t{recall:.4}\t{}\t{}\n",
            got.first()
                .map(|hit| hit.cx_id.to_string())
                .unwrap_or_default(),
            latencies.last().copied().unwrap_or(0)
        ));
    }
    Ok((recalls, latencies))
}

fn concat_recall(
    request: &Request,
    index: &ConcatCrossTermDiskAnn,
    rows: &[(ConcatCrossTermKey, Vec<f32>)],
    hits: &mut String,
) -> Result<(Vec<f64>, Vec<u128>), String> {
    let mut recalls = Vec::new();
    let mut latencies = Vec::new();
    for q in query_ids(request.queries, rows.len()) {
        let exact = exact_concat(rows, q, request.k);
        let started = Instant::now();
        let got = index
            .search_terms(&rows[q].1, request.k, Some(request.ef_search))
            .map_err(|error| error.to_string())?;
        latencies.push(started.elapsed().as_micros());
        let got_ids: BTreeSet<_> = got.iter().map(|hit| hit.key.cx_id).collect();
        let recall = overlap(&exact, &got_ids);
        recalls.push(recall);
        hits.push_str(&format!(
            "concat\t{q}\t{recall:.4}\t{}\t{}\n",
            got.first()
                .map(|hit| hit.key.cx_id.to_string())
                .unwrap_or_default(),
            latencies.last().copied().unwrap_or(0)
        ));
    }
    Ok((recalls, latencies))
}

fn run_edges(root: &Path, request: &Request) -> Result<Vec<EdgeReport>, String> {
    let mut reports = Vec::new();
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let empty_root = root.join("empty-token");
    let before = empty_root.join("graph.cda").exists();
    let empty = TokenDiskAnnMaxSim::build(
        SlotId::new(0),
        &empty_root,
        &[],
        build_params(request),
        search_params(request),
    )
    .expect_err("empty token rows fail");
    reports.push(edge(
        "empty-token",
        before,
        empty_root.join("graph.cda").exists(),
        "CALYX_INDEX_INVALID_PARAMS",
        &empty,
    ));
    let rows = token_rows(8, 1, request.dim);
    let dim_root = root.join("token-dim");
    let token = TokenDiskAnnMaxSim::build(
        SlotId::new(0),
        &dim_root,
        &rows,
        build_params(request),
        search_params(request),
    )
    .map_err(|error| error.to_string())?;
    let dim = token
        .search(
            &SlotVector::Multi {
                token_dim: request.dim as u32 + 1,
                tokens: vec![vec![1.0; request.dim + 1]],
            },
            3,
            Some(request.ef_search),
        )
        .expect_err("token dim mismatch fails");
    reports.push(edge(
        "token-dim-mismatch",
        true,
        true,
        "CALYX_SEXTANT_VECTOR_SHAPE",
        &dim,
    ));
    fs::remove_file(dim_root.join("tokens.f32")).map_err(|error| error.to_string())?;
    let missing = TokenDiskAnnMaxSim::open(SlotId::new(0), &dim_root, search_params(request), 8)
        .expect_err("missing token sidecar fails");
    reports.push(edge(
        "missing-token-sidecar",
        false,
        false,
        "CALYX_INDEX_IO",
        &missing,
    ));
    Ok(reports)
}

fn edge(
    case: &str,
    before_exists: bool,
    after_exists: bool,
    expected: &str,
    error: &calyx_core::CalyxError,
) -> EdgeReport {
    EdgeReport {
        case: case.to_string(),
        before_exists,
        after_exists,
        expected_error: expected.to_string(),
        observed_error: error.code.to_string(),
        observed_message: error.message.clone(),
    }
}

fn token_rows(docs: usize, tokens_per_doc: usize, dim: usize) -> Vec<(CxId, Vec<Vec<f32>>)> {
    (0..docs)
        .map(|doc| {
            let tokens = (0..tokens_per_doc)
                .map(|tok| vec_for(doc * 31 + tok, dim))
                .collect();
            (cx(doc), tokens)
        })
        .collect()
}

fn concat_rows(count: usize, dim: usize) -> Vec<(ConcatCrossTermKey, Vec<f32>)> {
    (0..count)
        .map(|idx| {
            (
                ConcatCrossTermKey {
                    cx_id: cx(idx),
                    a: SlotId::new(1),
                    b: SlotId::new(2),
                },
                vec_for(idx * 17, dim),
            )
        })
        .collect()
}

fn vec_for(seed: usize, dim: usize) -> Vec<f32> {
    let phase = seed as f32 * 0.000_031;
    (0..dim)
        .map(|axis| {
            let freq = axis as f32 + 1.0;
            (phase * freq).sin() + 0.5 * (phase * (freq + 0.37)).cos()
        })
        .collect()
}

fn exact_token(rows: &[(CxId, Vec<Vec<f32>>)], q: usize, k: usize) -> BTreeSet<CxId> {
    let query = &rows[q].1;
    let mut scores: Vec<_> = rows
        .iter()
        .map(|(cx, doc)| (*cx, MaxSimIndex::maxsim(query, doc)))
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scores.into_iter().take(k).map(|(cx, _)| cx).collect()
}

fn exact_concat(rows: &[(ConcatCrossTermKey, Vec<f32>)], q: usize, k: usize) -> BTreeSet<CxId> {
    let query = &rows[q].1;
    let mut scores: Vec<_> = rows
        .iter()
        .map(|(key, vector)| (key.cx_id, distance(query, vector)))
        .collect();
    scores.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    scores.into_iter().take(k).map(|(cx, _)| cx).collect()
}

fn overlap(exact: &BTreeSet<CxId>, got: &BTreeSet<CxId>) -> f64 {
    exact.intersection(got).count() as f64 / exact.len().max(1) as f64
}

fn distance(a: &[f32], b: &[f32]) -> f32 {
    let (dot, aa, bb) = a.iter().zip(b).fold((0.0, 0.0, 0.0), |acc, (x, y)| {
        (acc.0 + x * y, acc.1 + x * x, acc.2 + y * y)
    });
    if aa == 0.0 || bb == 0.0 {
        1.0
    } else {
        (1.0 - dot / (aa.sqrt() * bb.sqrt())).max(0.0)
    }
}

fn build_params(request: &Request) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim: request.dim,
        m_max: 8,
        ef_construction: request.ef_search.max(32),
        alpha: 1.2,
    }
}

fn search_params(request: &Request) -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: request.beamwidth,
        ef_search: request.ef_search,
        rescore_k: request.rescore_k,
        rescore_from_raw: false,
    }
}

impl Request {
    fn parse(args: &[String]) -> Result<Self, String> {
        let root = required_path(args, "--root")?;
        let docs = number(args, "--docs", 1000)?;
        let k = number(args, "--k", 10)?;
        Ok(Self {
            root,
            docs,
            concat_rows: number(args, "--concat-rows", docs)?,
            tokens_per_doc: number(args, "--tokens-per-doc", 1)?,
            dim: number(args, "--dim", 8)?,
            queries: number(args, "--queries", 8)?,
            k,
            recall_target: float(args, "--recall-target", 0.90)?,
            beamwidth: number(args, "--beamwidth", 32)?,
            ef_search: number(args, "--ef-search", 128)?.max(k),
            rescore_k: number(args, "--rescore-k", 128)?.max(k),
        })
    }
}

struct Paths {
    token_root: PathBuf,
    concat_root: PathBuf,
    metrics_dir: PathBuf,
    edge_root: PathBuf,
}

impl Paths {
    fn create(root: &Path) -> Result<Self, String> {
        let paths = Self {
            token_root: root.join("idx/slot_00.token.ann"),
            concat_root: root.join("idx/xterm.concat.ann"),
            metrics_dir: root.join("metrics"),
            edge_root: root.join("edges"),
        };
        fs::create_dir_all(&paths.metrics_dir).map_err(|error| error.to_string())?;
        Ok(paths)
    }
}

fn query_ids(queries: usize, len: usize) -> impl Iterator<Item = usize> {
    (0..queries).map(move |idx| (idx * 17 + 7) % len)
}

fn avg(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

fn min(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::INFINITY, f64::min)
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn dir_bytes(path: &Path) -> Result<u64, String> {
    fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .map(|entry| {
            entry
                .map_err(|error| error.to_string())
                .and_then(|entry| entry.metadata().map_err(|error| error.to_string()))
                .map(|meta| meta.len())
        })
        .sum()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::result::Result<(), String> {
    fs::write(
        path,
        serde_json::to_string_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn cx(idx: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..].copy_from_slice(&(idx as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

fn required_path(args: &[String], flag: &str) -> Result<PathBuf, String> {
    value(args, flag)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} is required"))
}

fn number(args: &[String], flag: &str, default: usize) -> Result<usize, String> {
    value(args, flag)
        .map(str::parse::<usize>)
        .transpose()
        .map_err(|error| format!("{flag}: {error}"))?
        .map_or(Ok(default), |n| {
            if n == 0 {
                Err(format!("{flag} must be positive"))
            } else {
                Ok(n)
            }
        })
}

fn float(args: &[String], flag: &str, default: f64) -> Result<f64, String> {
    value(args, flag)
        .map(str::parse::<f64>)
        .transpose()
        .map_err(|error| format!("{flag}: {error}"))?
        .map_or(Ok(default), |n| {
            if n.is_finite() && n > 0.0 {
                Ok(n)
            } else {
                Err(format!("{flag} must be positive and finite"))
            }
        })
}

fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].as_str())
}
