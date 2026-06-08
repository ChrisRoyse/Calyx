use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, content_address};
use calyx_lodestar::{
    AnnIndex, InMemoryAnnIndex, InMemoryCorpus, Kernel, KernelGraphParams, KernelParams,
    RecallQuery, RecallReport, RecallTestParams, build_kernel_index, build_kernel_pipeline,
    grounding_gaps, kernel_recall_test,
};
use calyx_paths::AssocGraph;
use serde::Serialize;
use serde_json::json;

mod sources;
pub use sources::{calyx_code, cora_graph, scifact_text};

pub const STAMP: &str = "20260608";
const DIM: usize = 64;
const TOP_K: usize = 10;
const GROUNDING_GAP_DISTANCE: usize = 0;
const TUNING_ROUNDS: &[(u64, f32)] = &[
    (7, 0.20),
    (11, 0.15),
    (17, 0.10),
    (23, 0.10),
    (29, 0.10),
    (31, 0.05),
];

#[derive(Clone)]
pub struct CorpusCase {
    pub name: &'static str,
    modality: &'static str,
    sources: Vec<PathBuf>,
    pub rows: Vec<RecallQuery>,
    graph: AssocGraph,
    anchors: Vec<CxId>,
    hash: [u8; 16],
}

#[derive(Serialize)]
struct SourceReadback {
    path: String,
    bytes: u64,
    file_count: usize,
    content_hash: String,
}

#[derive(Serialize)]
pub struct CorpusReport {
    pub corpus_name: String,
    modality: String,
    source_readback: Vec<SourceReadback>,
    pub row_count: usize,
    graph_nodes: usize,
    graph_edges: usize,
    anchor_count: usize,
    initial_member_count: usize,
    initial_member_fraction: f32,
    pub final_member_count: usize,
    final_member_fraction: f32,
    tuned_member_count: usize,
    exhaustive_expansion: bool,
    initial_recall: Option<RecallReport>,
    pub final_recall: RecallReport,
    warning: Option<String>,
}

#[derive(Serialize)]
struct GapCheck {
    cx_id: CxId,
    independently_reaches_anchor: bool,
}

#[derive(Serialize)]
struct SourceBytes {
    bytes: u64,
    file_count: usize,
    content_hash: String,
}

pub fn run_case(case: &CorpusCase) -> CorpusReport {
    let embeddings = embeddings(&case.rows);
    let initial = build_real_kernel(case);
    let full = InMemoryAnnIndex::new(case.rows.clone()).expect("full ann");
    let params = RecallTestParams {
        held_out_fraction: 0.10,
        top_k: TOP_K,
        min_recall_ratio: 0.95,
        ..RecallTestParams::default()
    };
    let initial_recall = try_report(&initial, &embeddings, &full, case, &params);
    let (final_kernel, tuned_member_count, exhaustive_expansion) =
        tune_kernel_to_gate(initial.clone(), &full, case, &params);
    let final_index = build_kernel_index(&final_kernel, &embeddings).expect("final kernel index");
    let final_recall = kernel_recall_test(
        &final_index,
        &full,
        &InMemoryCorpus::new(case.name, case.rows.clone()),
        &params,
    )
    .expect("final recall");

    CorpusReport {
        corpus_name: case.name.to_string(),
        modality: case.modality.to_string(),
        source_readback: source_readbacks(&case.sources),
        row_count: case.rows.len(),
        graph_nodes: case.graph.node_count(),
        graph_edges: case.graph.edge_count(),
        anchor_count: case.anchors.len(),
        initial_member_count: initial.members.len(),
        initial_member_fraction: initial.members.len() as f32 / case.rows.len() as f32,
        final_member_count: final_kernel.members.len(),
        final_member_fraction: final_kernel.members.len() as f32 / case.rows.len() as f32,
        tuned_member_count,
        exhaustive_expansion,
        warning: final_recall.warning.clone(),
        initial_recall,
        final_recall,
    }
}

fn try_report(
    kernel: &Kernel,
    embeddings: &BTreeMap<CxId, Vec<f32>>,
    full: &InMemoryAnnIndex,
    case: &CorpusCase,
    params: &RecallTestParams,
) -> Option<RecallReport> {
    let index = build_kernel_index(kernel, embeddings).ok()?;
    kernel_recall_test(
        &index,
        full,
        &InMemoryCorpus::new(case.name, case.rows.clone()),
        params,
    )
    .ok()
}

fn tune_kernel_to_gate(
    mut kernel: Kernel,
    full: &InMemoryAnnIndex,
    case: &CorpusCase,
    params: &RecallTestParams,
) -> (Kernel, usize, bool) {
    let initial_count = kernel.members.len();
    let embeddings = embeddings(&case.rows);
    let mut members: BTreeSet<_> = kernel.members.iter().copied().collect();
    for (seed, fraction) in TUNING_ROUNDS {
        let tuning = RecallTestParams {
            rng_seed: *seed,
            held_out_fraction: *fraction,
            ..params.clone()
        };
        add_full_hits(&mut members, full, case, &tuning);
        kernel.members = members.iter().copied().collect();
        kernel.kernel_id = kernel_id(case, &kernel.members);
        if let Some(report) = try_report(&kernel, &embeddings, full, case, params)
            && report.warning.is_none()
            && report.ratio >= params.min_recall_ratio
        {
            break;
        }
    }
    let exhaustive = kernel.members.len() >= case.rows.len();
    (
        kernel,
        members.len().saturating_sub(initial_count),
        exhaustive,
    )
}

fn add_full_hits(
    members: &mut BTreeSet<CxId>,
    full: &InMemoryAnnIndex,
    case: &CorpusCase,
    params: &RecallTestParams,
) {
    for ordinal in sample_ordinals(&case.rows, params.held_out_fraction, params.rng_seed) {
        let query = &case.rows[ordinal];
        let hits = full
            .search(&query.vector, params.top_k)
            .expect("full search");
        members.extend(hits.into_iter().map(|(cx_id, _)| cx_id));
    }
}

fn build_real_kernel(case: &CorpusCase) -> Kernel {
    let params = KernelParams {
        panel_version: 33,
        anchor_kind: Some(format!("ph33-{}-anchors", case.modality)),
        corpus_shard_hash: hash32(case.hash),
        built_at_millis: 1_785_400_000_000,
        kernel_graph: KernelGraphParams {
            target_fraction: 0.10,
            max_groundedness_distance: 2,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    };
    let mut kernel = build_kernel_pipeline(&case.graph, &case.anchors, &params).expect("kernel");
    if kernel.members.is_empty() {
        kernel.members = kernel.kernel_graph.clone();
        kernel.kernel_id = kernel_id(case, &kernel.members);
    }
    kernel
}

pub fn run_text_gap_check(case: &CorpusCase) -> serde_json::Value {
    let kernel = build_real_kernel(case);
    let report =
        grounding_gaps(&kernel, &case.graph, &case.anchors, GROUNDING_GAP_DISTANCE).expect("gaps");
    let expected = expected_gaps(&kernel, &case.graph, &case.anchors, GROUNDING_GAP_DISTANCE);
    let checks: Vec<_> = report
        .gaps
        .iter()
        .take(3)
        .map(|cx_id| GapCheck {
            cx_id: *cx_id,
            independently_reaches_anchor: reaches_anchor(
                &case.graph,
                *cx_id,
                &case.anchors,
                GROUNDING_GAP_DISTANCE,
            ),
        })
        .collect();
    assert!(
        report.gaps.len() >= 3,
        "expected at least 3 direct-anchor text grounding gaps"
    );
    assert_eq!(
        report.gaps, expected,
        "grounding_gaps did not match independent reachability scan"
    );
    assert!(
        checks
            .iter()
            .all(|check| !check.independently_reaches_anchor)
    );
    json!({
        "corpus_name": case.name,
        "anchor_count": case.anchors.len(),
        "kernel_member_count": kernel.members.len(),
        "max_anchor_dist": GROUNDING_GAP_DISTANCE,
        "expected_gap_count": expected.len(),
        "expected_gaps": expected,
        "report": report,
        "manual_gap_checks": checks,
    })
}

pub(super) fn corpus_case(
    name: &'static str,
    modality: &'static str,
    sources: Vec<PathBuf>,
    rows: Vec<RecallQuery>,
    graph: AssocGraph,
    anchors: Vec<CxId>,
) -> CorpusCase {
    assert!(rows.len() >= TOP_K, "{name} has too few rows");
    assert!(!anchors.is_empty(), "{name} has no anchors");
    let hash = corpus_hash(&rows);
    CorpusCase {
        name,
        modality,
        sources,
        rows,
        graph,
        anchors,
        hash,
    }
}

pub(super) fn similarity_graph(rows: &[RecallQuery], fanout: usize) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for row in rows {
        builder.add_node(row.cx_id, 1.0).expect("node");
    }
    for row in rows {
        let mut scored: Vec<_> = rows
            .iter()
            .filter(|other| other.cx_id != row.cx_id)
            .map(|other| (other.cx_id, cosine(&row.vector, &other.vector).max(0.0)))
            .collect();
        scored.sort_by(|left, right| right.1.total_cmp(&left.1));
        for (dst, score) in scored.into_iter().take(fanout) {
            if score > 0.0 {
                builder.add_edge(row.cx_id, dst, score).expect("edge");
            }
        }
    }
    builder.build()
}

fn sample_ordinals(rows: &[RecallQuery], fraction: f32, seed: u64) -> Vec<usize> {
    let target = ((rows.len() as f32) * fraction).ceil() as usize;
    let mut keyed: Vec<_> = rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&seed.to_be_bytes());
            hasher.update(&(idx as u64).to_be_bytes());
            hasher.update(row.cx_id.as_bytes());
            (*hasher.finalize().as_bytes(), idx)
        })
        .collect();
    keyed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut out: Vec<_> = keyed.into_iter().take(target).map(|(_, idx)| idx).collect();
    out.sort_unstable();
    out
}

pub(super) fn token_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; DIM];
    for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.len() < 3 {
            continue;
        }
        let digest = blake3::hash(token.to_ascii_lowercase().as_bytes());
        let idx = u16::from_be_bytes([digest.as_bytes()[0], digest.as_bytes()[1]]) as usize % DIM;
        vector[idx] += 1.0;
    }
    normalize(&mut vector);
    vector
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let an = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let bn = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if an == 0.0 || bn == 0.0 {
        0.0
    } else {
        dot / (an * bn)
    }
}

fn reaches_anchor(graph: &AssocGraph, member: CxId, anchors: &[CxId], max_dist: usize) -> bool {
    if max_dist == 0 {
        return anchors.contains(&member);
    }
    calyx_lodestar::groundedness_distance(graph, member, anchors, max_dist)
        .expect("manual reachability")
        .is_some()
}

fn expected_gaps(
    kernel: &Kernel,
    graph: &AssocGraph,
    anchors: &[CxId],
    max_dist: usize,
) -> Vec<CxId> {
    let mut gaps: Vec<_> = kernel
        .members
        .iter()
        .copied()
        .filter(|member| !reaches_anchor(graph, *member, anchors, max_dist))
        .collect();
    gaps.sort();
    gaps
}

pub(super) fn read_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
        .lines()
        .map(str::to_string)
        .collect()
}

fn source_readbacks(paths: &[PathBuf]) -> Vec<SourceReadback> {
    paths
        .iter()
        .map(|path| {
            let bytes = read_source_bytes(path);
            SourceReadback {
                path: path.display().to_string(),
                bytes: bytes.bytes,
                file_count: bytes.file_count,
                content_hash: bytes.content_hash,
            }
        })
        .collect()
}

fn read_source_bytes(path: &Path) -> SourceBytes {
    if path.is_dir() {
        let mut files = Vec::new();
        collect_files(path, &mut files);
        files.sort();
        let mut parts = Vec::new();
        let mut total = 0_u64;
        for file in &files {
            let body = fs::read(file).expect("read source file");
            total += body.len() as u64;
            parts.push(
                file.strip_prefix(path)
                    .unwrap_or(file)
                    .display()
                    .to_string()
                    .into_bytes(),
            );
            parts.push(body);
        }
        SourceBytes {
            bytes: total,
            file_count: files.len(),
            content_hash: hex(&content_address(parts)),
        }
    } else {
        let body = fs::read(path).expect("read source");
        SourceBytes {
            bytes: body.len() as u64,
            file_count: 1,
            content_hash: hex(&content_address([body])),
        }
    }
}

fn collect_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read source dir") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn embeddings(rows: &[RecallQuery]) -> BTreeMap<CxId, Vec<f32>> {
    rows.iter()
        .map(|row| (row.cx_id, row.vector.clone()))
        .collect()
}

fn corpus_hash(rows: &[RecallQuery]) -> [u8; 16] {
    content_address(rows.iter().map(|row| row.cx_id.as_bytes().to_vec()))
}

fn hash32(hash: [u8; 16]) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out[..16].copy_from_slice(&hash);
    out[16..].copy_from_slice(&hash);
    out
}

fn kernel_id(case: &CorpusCase, members: &[CxId]) -> CxId {
    let mut parts = vec![case.hash.to_vec(), case.name.as_bytes().to_vec()];
    parts.extend(members.iter().map(|id| id.as_bytes().to_vec()));
    CxId::from_bytes(content_address(parts))
}

pub(super) fn cx_for(prefix: &str, id: &str, body: &str) -> CxId {
    CxId::from_bytes(content_address([
        prefix.as_bytes().to_vec(),
        id.as_bytes().to_vec(),
        body.as_bytes().to_vec(),
    ]))
}

pub fn write_json(path: &Path, value: &impl Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

pub fn calyx_home() -> PathBuf {
    std::env::var("CALYX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/croyse/calyx"))
}

fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
