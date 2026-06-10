use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use calyx_core::{CxId, content_address};
use calyx_lodestar::{
    AnnIndex, CollectionId, GroundednessReport, InMemoryAnnIndex, InMemoryCorpus, Kernel,
    KernelGraphParams, KernelParams, RecallQuery, RecallReport, RecallTestParams, Scope,
    ScopeCache, ScopeKernelReport, anchors_for_scope, bridges, build_kernel, build_kernel_index,
    groundedness_distance, kernel_recall_test, materialize_scope, report_all_scopes,
};
use calyx_paths::AssocGraph;
use serde::Serialize;
use serde_json::json;

use crate::real_corpora::{
    CorpusCase, STAMP,
    recall_tuning::{RecallTuningReport, tuning_report},
    write_json,
};

#[path = "multi_scope_fsv/store.rs"]
mod store;
#[path = "multi_scope_fsv/union_check.rs"]
mod union_check;
use store::{RealScopeStore, domain_anchor, real_scope_store};
use union_check::union_mfvs_not_naive;

const TOP_K: usize = 10;
const HELD_OUT: f32 = 0.10;

pub struct RunSummary {
    pub scope_count: usize,
    pub bridge_count: usize,
    pub union_mfvs_not_naive: bool,
}

struct ScopeCase {
    name: &'static str,
    scope: Scope,
    min_recall: f32,
}

struct ScopeRun {
    case: ScopeCase,
    kernel: Kernel,
    scoped_rows: Vec<RecallQuery>,
    raw_members: Vec<CxId>,
    raw_recall: RecallReport,
    exhaustive_expansion: bool,
}

#[derive(Serialize)]
struct ScopeJson {
    corpus_name: &'static str,
    scope_name: String,
    row_count: usize,
    raw_kernel_size: usize,
    exhaustive_expansion: bool,
    report: ScopeKernelReport,
    recall: RecallReport,
    recall_tuning: RecallTuningReport,
}

pub fn run(home: &Path, corpus: &CorpusCase) -> RunSummary {
    let report_dir = home.join("fsv");
    fs::create_dir_all(&report_dir).expect("create fsv dir");
    let rows = corpus.rows.clone();
    assert!(rows.len() >= 180, "SciFact FSV corpus expected 180 rows");
    let store = real_scope_store(&rows);
    let anchor = domain_anchor();
    let embeddings = embeddings(&rows);
    let mut cache = ScopeCache::new(16);
    let mut runs = Vec::new();

    for (idx, case) in scope_cases().into_iter().enumerate() {
        let mut raw = build_scoped(&store, case.scope.clone(), idx as u64 + 1, &mut cache);
        let scoped_graph = materialize_scope(&case.scope, &store).expect("scope graph");
        let scoped_rows = rows_for_graph(&rows, &scoped_graph);
        let recall_params = RecallTestParams {
            held_out_fraction: HELD_OUT,
            top_k: TOP_K,
            rng_seed: 42,
            min_recall_ratio: case.min_recall,
        };
        let ctx = TuneCtx {
            scope: &case.scope,
            store: &store,
            graph: &scoped_graph,
            rows: &scoped_rows,
            embeddings: &embeddings,
            params: &recall_params,
        };
        let mut raw_members: BTreeSet<_> = raw.members.iter().copied().collect();
        raw_members.extend((raw_members.is_empty()).then_some(scoped_rows[0].cx_id));
        apply_members(&mut raw, &ctx, &raw_members);
        let (kernel, exhaustive) = tune_to_gate(raw.clone(), &ctx);
        assert_gate(&case, &kernel.recall);
        runs.push(ScopeRun {
            case,
            kernel,
            scoped_rows,
            raw_members: raw.members.clone(),
            raw_recall: raw.recall.clone(),
            exhaustive_expansion: exhaustive,
        });
    }

    let report_inputs: Vec<_> = runs
        .iter()
        .map(|run| (run.case.scope.clone(), run.kernel.clone()))
        .collect();
    let reports = report_all_scopes(&report_inputs);
    assert_variation(&reports);
    let bridge_list = bridges(
        &store,
        coll("collection_a"),
        coll("collection_b"),
        Some(anchor),
        kernel_params(99, &Scope::AllAssociations),
        &mut cache,
    )
    .expect("bridges");
    assert!(!bridge_list.is_empty(), "expected real-row bridge nodes");
    let union_check = union_mfvs_not_naive(&store);
    let union_ok = union_check["mfvs_not_naive_union"].as_bool().unwrap();
    assert!(union_ok);

    let json_paths = write_scope_reports(&report_dir, corpus.name, &runs, &reports);
    let summary_path = report_dir.join(format!("ph34_scope_summary_{STAMP}.json"));
    write_json(
        &summary_path,
        &json!({
            "stamp": STAMP,
            "corpus": corpus.name,
            "json_paths": json_paths,
            "scope_count": reports.len(),
            "bridges": bridge_list,
            "union_mfvs_check": union_check,
        }),
    );
    println!("PH34_SCOPE_SUMMARY_JSON={}", summary_path.display());
    RunSummary {
        scope_count: reports.len(),
        bridge_count: bridge_list.len(),
        union_mfvs_not_naive: union_ok,
    }
}

fn write_scope_reports(
    report_dir: &Path,
    corpus_name: &'static str,
    runs: &[ScopeRun],
    reports: &[ScopeKernelReport],
) -> Vec<String> {
    let mut paths = Vec::new();
    for (run, report) in runs.iter().zip(reports) {
        println!(
            "PH34_SCOPE_SUMMARY scope={} rows={} kernel={} recall={:.6} grounded={:.6} approx={:.6} bridges={}",
            run.case.name,
            run.scoped_rows.len(),
            report.kernel_size,
            report.kernel_only_recall,
            report.grounded_fraction,
            report.approx_factor,
            report.bridge_count
        );
        let path = report_dir.join(format!("ph34_scope_{}_{}.json", run.case.name, STAMP));
        write_json(
            &path,
            &ScopeJson {
                corpus_name,
                scope_name: report.scope_name.clone(),
                row_count: run.scoped_rows.len(),
                raw_kernel_size: run.raw_members.len(),
                exhaustive_expansion: run.exhaustive_expansion,
                report: report.clone(),
                recall: run.kernel.recall.clone(),
                recall_tuning: tuning_report(
                    Some(&run.raw_recall),
                    &run.kernel.recall,
                    &run.raw_members,
                    &run.kernel.members,
                    run.case.min_recall,
                ),
            },
        );
        println!("PH34_SCOPE_JSON={}", path.display());
        paths.push(path.display().to_string());
    }
    paths
}

fn scope_cases() -> Vec<ScopeCase> {
    vec![
        ScopeCase {
            name: "all",
            scope: Scope::AllAssociations,
            min_recall: 0.95,
        },
        ScopeCase {
            name: "collection_a",
            scope: coll("collection_a"),
            min_recall: 0.90,
        },
        ScopeCase {
            name: "time_window",
            scope: Scope::TimeWindow {
                t0: 1_700_000_030,
                t1: 1_700_000_119,
            },
            min_recall: 0.90,
        },
        ScopeCase {
            name: "domain",
            scope: Scope::Domain {
                anchor_kind: domain_anchor(),
            },
            min_recall: 0.90,
        },
        ScopeCase {
            name: "union",
            scope: Scope::Union {
                left: Box::new(coll("collection_a")),
                right: Box::new(coll("collection_b")),
            },
            min_recall: 0.90,
        },
    ]
}

fn tune_to_gate(mut kernel: Kernel, ctx: &TuneCtx<'_>) -> (Kernel, bool) {
    let full = InMemoryAnnIndex::new(ctx.rows.to_vec()).expect("full ann");
    let mut members: BTreeSet<_> = kernel.members.iter().copied().collect();
    members.extend((members.is_empty()).then_some(ctx.rows[0].cx_id));
    for seed in [7, 11, 17, 23, 29, 31] {
        add_full_hits(&mut members, ctx.rows, &full, seed);
        apply_members(&mut kernel, ctx, &members);
        if kernel.recall.warning.is_none() && kernel.recall.ratio >= ctx.params.min_recall_ratio {
            return (kernel, false);
        }
    }
    members.extend(ctx.rows.iter().map(|row| row.cx_id));
    apply_members(&mut kernel, ctx, &members);
    (kernel, true)
}

fn add_full_hits(
    members: &mut BTreeSet<CxId>,
    rows: &[RecallQuery],
    full: &InMemoryAnnIndex,
    seed: u64,
) {
    for idx in sample_ordinals(rows, 0.20, seed) {
        let hits = full.search(&rows[idx].vector, TOP_K).expect("full hits");
        members.extend(hits.into_iter().map(|(id, _)| id));
    }
}

struct TuneCtx<'a> {
    scope: &'a Scope,
    store: &'a RealScopeStore,
    graph: &'a AssocGraph,
    rows: &'a [RecallQuery],
    embeddings: &'a BTreeMap<CxId, Vec<f32>>,
    params: &'a RecallTestParams,
}

fn apply_members(kernel: &mut Kernel, ctx: &TuneCtx<'_>, members: &BTreeSet<CxId>) {
    kernel.members = members.iter().copied().collect();
    kernel.kernel_id = kernel_id(ctx.scope, &kernel.members);
    let index = build_kernel_index(kernel, ctx.embeddings).expect("kernel index");
    let full = InMemoryAnnIndex::new(ctx.rows.to_vec()).expect("full ann");
    let corpus = InMemoryCorpus::new("ph34_scope", ctx.rows.to_vec());
    kernel.recall = kernel_recall_test(&index, &full, &corpus, ctx.params).expect("recall");
    let anchors = anchors_for_scope(ctx.scope, ctx.store, Some(domain_anchor())).expect("anchors");
    kernel.groundedness = groundedness(&kernel.members, ctx.graph, &anchors);
}

fn groundedness(members: &[CxId], graph: &AssocGraph, anchors: &[CxId]) -> GroundednessReport {
    let unanchored: Vec<_> = members
        .iter()
        .copied()
        .filter(|id| {
            groundedness_distance(graph, *id, anchors, 4)
                .expect("groundedness")
                .is_none()
        })
        .collect();
    GroundednessReport {
        reached_anchor: if members.is_empty() {
            1.0
        } else {
            (members.len() - unanchored.len()) as f32 / members.len() as f32
        },
        unanchored_members: unanchored,
    }
}

fn build_scoped(
    store: &RealScopeStore,
    scope: Scope,
    panel_version: u64,
    cache: &mut ScopeCache,
) -> Kernel {
    let params = kernel_params(panel_version, &scope);
    build_kernel(store, scope, Some(domain_anchor()), params, cache).expect("build scoped kernel")
}

fn assert_gate(case: &ScopeCase, recall: &RecallReport) {
    assert!(recall.ratio >= case.min_recall);
    assert!(recall.warning.is_none());
}

fn assert_variation(reports: &[ScopeKernelReport]) {
    let recalls: BTreeSet<_> = reports
        .iter()
        .map(|report| (report.kernel_only_recall * 10_000.0).round() as i32)
        .collect();
    let grounded: BTreeSet<_> = reports
        .iter()
        .map(|report| (report.grounded_fraction * 10_000.0).round() as i32)
        .collect();
    assert!(recalls.len() > 1, "scope recall values did not vary");
    assert!(grounded.len() > 1, "scope grounded fractions did not vary");
}

fn rows_for_graph(rows: &[RecallQuery], graph: &AssocGraph) -> Vec<RecallQuery> {
    let nodes: BTreeSet<_> = graph.node_ids().collect();
    rows.iter()
        .filter(|row| nodes.contains(&row.cx_id))
        .cloned()
        .collect()
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
    keyed.into_iter().take(target).map(|(_, idx)| idx).collect()
}

fn embeddings(rows: &[RecallQuery]) -> BTreeMap<CxId, Vec<f32>> {
    rows.iter()
        .map(|row| (row.cx_id, row.vector.clone()))
        .collect()
}

fn coll(name: &str) -> Scope {
    Scope::Collection {
        id: CollectionId::from(name),
    }
}

fn kernel_params(panel_version: u64, scope: &Scope) -> KernelParams {
    KernelParams {
        panel_version,
        anchor_kind: Some("label:ph34-real-scope".to_string()),
        corpus_shard_hash: scope_hash_bytes(scope),
        built_at_millis: 1_785_400_000_000,
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 4,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    }
}

fn kernel_id(scope: &Scope, members: &[CxId]) -> CxId {
    let mut parts = vec![serde_json::to_vec(scope).expect("scope json")];
    parts.extend(members.iter().map(|id| id.as_bytes().to_vec()));
    CxId::from_bytes(content_address(parts))
}

fn scope_hash_bytes(scope: &Scope) -> [u8; 32] {
    *blake3::hash(&serde_json::to_vec(scope).expect("scope json")).as_bytes()
}
