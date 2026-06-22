#![cfg(feature = "fsv")]

#[allow(dead_code)]
#[path = "support/real_corpora.rs"]
mod real_corpora;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use calyx_core::CxId;
use calyx_lodestar::{
    AnswerPath, Kernel, RecallQuery, build_kernel_index, kernel_answer, kernel_search,
};
use calyx_paths::reach;
use serde::Serialize;
use serde_json::json;

use real_corpora::{
    CorpusCase, STAMP, TunedKernelFixtureReport, calyx_code, calyx_home, cora_graph,
    embeddings_for_case, scifact_text, source_readback_json, tuned_kernel_fixture, write_json,
};

const OLD_CANDIDATE_WINDOW: usize = 10;
const MAX_ANSWER_HOPS: usize = 64;

#[derive(Clone, Debug)]
struct AnchorSearchFixture {
    query: RecallQuery,
    anchor: CxId,
    anchored_nodes: Vec<CxId>,
    anchor_rank: usize,
    top_window: Vec<(CxId, f32)>,
    full_hit_count: usize,
    expected_path: Vec<CxId>,
}

#[derive(Serialize)]
struct AnchorSearchReport {
    stamp: &'static str,
    corpus_name: &'static str,
    modality: &'static str,
    source_readback: serde_json::Value,
    row_count: usize,
    graph_nodes: usize,
    graph_edges: usize,
    anchor_count: usize,
    kernel_id: CxId,
    kernel_member_count: usize,
    tuned_kernel: TunedKernelFixtureReport,
    old_candidate_window: usize,
    documented_exhaustive_candidate_bound: usize,
    full_hit_count: usize,
    anchor_rank: usize,
    anchor_absent_from_old_window: bool,
    selected_anchor: CxId,
    production_anchor_set_count: usize,
    production_anchor_set: Vec<CxId>,
    query_cx: CxId,
    expected_path: Vec<CxId>,
    expected_path_hops: usize,
    top_window: Vec<(CxId, f32)>,
    search_elapsed_micros: u64,
    answer_elapsed_micros: u64,
    answer_artifact: String,
    answer_bytes_len: usize,
    answer_readback_anchor: CxId,
    answer_readback_hops: usize,
    answer: AnswerPath,
}

#[test]
#[ignore = "manual FSV: reads real corpora and writes durable anchor-search readback"]
fn ph33_real_anchor_search_exhaustive_fallback_manual() {
    let home = calyx_home();
    let report_dir = report_dir(&home);
    fs::create_dir_all(&report_dir).expect("create fsv dir");

    let cases = vec![scifact_text(&home), calyx_code(&home), cora_graph(&home)];
    let (case, kernel, tuning, fixture) =
        find_fixture(&cases).expect("real corpus anchor outside old top-10 window");
    let embeddings = embeddings_for_case(case);
    let index = build_kernel_index(&kernel, &embeddings).expect("kernel index");

    let search_started = Instant::now();
    let full_hits = kernel_search(&index, &fixture.query.vector, index.rows().len())
        .expect("exhaustive candidate search");
    let search_elapsed = search_started.elapsed();
    let anchor_rank = full_hits
        .iter()
        .position(|(cx_id, _)| *cx_id == fixture.anchor)
        .map(|idx| idx + 1)
        .expect("selected anchor is present");
    let top_window: Vec<_> = full_hits
        .iter()
        .take(OLD_CANDIDATE_WINDOW)
        .copied()
        .collect();
    assert_eq!(anchor_rank, fixture.anchor_rank);
    assert_eq!(full_hits.len(), fixture.full_hit_count);
    assert_eq!(top_window, fixture.top_window);
    assert!(!top_window.iter().any(|(cx_id, _)| *cx_id == fixture.anchor));

    let answer_started = Instant::now();
    let answer = kernel_answer(
        &index,
        case.graph(),
        fixture.query.cx_id,
        &fixture.query.vector,
        &fixture.anchored_nodes,
        MAX_ANSWER_HOPS,
    )
    .expect("kernel answer should find selected real anchor");
    let answer_elapsed = answer_started.elapsed();

    assert_eq!(answer.anchor_kernel_node, fixture.anchor);
    assert_eq!(answer.query_cx, fixture.query.cx_id);
    assert_eq!(answer.hops.len(), fixture.expected_path.len() - 1);
    assert!(answer.total_score.is_finite());
    assert!(answer.total_score > 0.0);

    let answer_path = report_dir.join(format!(
        "ph33_anchor_search_answer_{}_{}.json",
        case.name, STAMP
    ));
    write_json(&answer_path, &answer);
    let answer_bytes = fs::read(&answer_path).expect("read answer artifact bytes");
    let answer_readback: AnswerPath =
        serde_json::from_slice(&answer_bytes).expect("decode answer artifact");
    assert_eq!(answer_readback, answer);

    let report = AnchorSearchReport {
        stamp: STAMP,
        corpus_name: case.name,
        modality: case.modality(),
        source_readback: source_readback_json(case),
        row_count: case.rows.len(),
        graph_nodes: case.graph().node_count(),
        graph_edges: case.graph().edge_count(),
        anchor_count: case.anchors().len(),
        kernel_id: kernel.kernel_id,
        kernel_member_count: index.rows().len(),
        tuned_kernel: tuning,
        old_candidate_window: OLD_CANDIDATE_WINDOW,
        documented_exhaustive_candidate_bound: index.rows().len(),
        full_hit_count: full_hits.len(),
        anchor_rank,
        anchor_absent_from_old_window: anchor_rank > OLD_CANDIDATE_WINDOW,
        selected_anchor: fixture.anchor,
        production_anchor_set_count: fixture.anchored_nodes.len(),
        production_anchor_set: fixture.anchored_nodes.clone(),
        query_cx: fixture.query.cx_id,
        expected_path: fixture.expected_path,
        expected_path_hops: answer.hops.len(),
        top_window,
        search_elapsed_micros: micros(search_elapsed),
        answer_elapsed_micros: micros(answer_elapsed),
        answer_artifact: answer_path.display().to_string(),
        answer_bytes_len: answer_bytes.len(),
        answer_readback_anchor: answer_readback.anchor_kernel_node,
        answer_readback_hops: answer_readback.hops.len(),
        answer,
    };
    let report_path = report_dir.join(format!(
        "ph33_anchor_search_exhaustive_fallback_{}_{}.json",
        case.name, STAMP
    ));
    write_json(&report_path, &report);
    let report_bytes = fs::read(&report_path).expect("read report artifact bytes");
    let report_value: serde_json::Value =
        serde_json::from_slice(&report_bytes).expect("decode report artifact");

    assert_eq!(
        report_value["anchor_absent_from_old_window"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(
        report_value["documented_exhaustive_candidate_bound"],
        json!(index.rows().len())
    );
    println!(
        "PH33_REAL_ANCHOR_SEARCH_REPORT={} ANSWER={} anchor_rank={} candidate_bound={}",
        report_path.display(),
        answer_path.display(),
        anchor_rank,
        index.rows().len()
    );
}

fn find_fixture(
    cases: &[CorpusCase],
) -> Option<(
    &CorpusCase,
    Kernel,
    TunedKernelFixtureReport,
    AnchorSearchFixture,
)> {
    for case in cases {
        let (kernel, tuning) = tuned_kernel_fixture(case);
        let embeddings = embeddings_for_case(case);
        let index = build_kernel_index(&kernel, &embeddings).ok()?;
        let member_set: BTreeSet<_> = kernel.members.iter().copied().collect();
        let anchors: Vec<_> = case
            .anchors()
            .iter()
            .copied()
            .filter(|anchor| member_set.contains(anchor))
            .collect();
        if anchors.is_empty() || index.rows().len() <= OLD_CANDIDATE_WINDOW {
            continue;
        }
        if let Some(fixture) = best_case_fixture(case, &index, &anchors) {
            return Some((case, kernel, tuning, fixture));
        }
    }
    None
}

fn best_case_fixture(
    case: &CorpusCase,
    index: &calyx_lodestar::KernelIndex,
    anchors: &[CxId],
) -> Option<AnchorSearchFixture> {
    let row_by_id: BTreeMap<_, _> = case.rows.iter().map(|row| (row.cx_id, row)).collect();
    let mut best: Option<AnchorSearchFixture> = None;

    for query in &case.rows {
        let Ok(answer) = kernel_answer(
            index,
            case.graph(),
            query.cx_id,
            &query.vector,
            anchors,
            MAX_ANSWER_HOPS,
        ) else {
            continue;
        };
        if query.cx_id == answer.anchor_kernel_node {
            continue;
        }
        let Ok(Some(path)) = reach(
            case.graph(),
            answer.anchor_kernel_node,
            query.cx_id,
            MAX_ANSWER_HOPS,
        ) else {
            continue;
        };
        if !path.iter().all(|cx_id| row_by_id.contains_key(cx_id)) {
            continue;
        }
        let hits = kernel_search(index, &query.vector, index.rows().len()).ok()?;
        let Some(anchor_rank) = hits
            .iter()
            .position(|(cx_id, _)| *cx_id == answer.anchor_kernel_node)
            .map(|idx| idx + 1)
        else {
            continue;
        };
        if anchor_rank <= OLD_CANDIDATE_WINDOW {
            continue;
        }
        let candidate = AnchorSearchFixture {
            query: query.clone(),
            anchor: answer.anchor_kernel_node,
            anchored_nodes: anchors.to_vec(),
            anchor_rank,
            top_window: hits.iter().take(OLD_CANDIDATE_WINDOW).copied().collect(),
            full_hit_count: hits.len(),
            expected_path: path,
        };
        if best
            .as_ref()
            .is_none_or(|current| candidate.anchor_rank > current.anchor_rank)
        {
            best = Some(candidate);
        }
    }
    best
}

fn report_dir(home: &std::path::Path) -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join("fsv"))
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
