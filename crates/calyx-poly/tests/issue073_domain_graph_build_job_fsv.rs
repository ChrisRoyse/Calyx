//! Issue #73 - per-domain Loom + Graph CF build job.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef,
    Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_lodestar::{KernelGraphParams, KernelParams, RecallQuery, RecallTestParams};
use calyx_poly::Domain;
use calyx_poly::domain_graph_build_job::{
    DomainGraphBuildRequest, DomainGraphEdgeInput, ERR_DOMAIN_GRAPH_EMPTY,
    read_domain_graph_build_report, run_domain_graph_build_job,
};
use calyx_poly::entity_graph_edges::EDGE_SHARED_ENTITY;
use calyx_poly::knn_graph_edges::EDGE_KNN_RESOLVED;
use calyx_poly::panel_diagnostics::PanelMatrix;
use calyx_poly::structural_edges::EDGE_YES_NO_COMPLEMENT;
use calyx_poly::temporal_graph_edges::EDGE_TEMPORAL_LEAD_LAG;
use serde_json::{Value, json};

use support::{collect_files, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue73-domain-graph-build";
const COLLECTION: &str = "poly_issue73_domain_graph";
const TEST_TS: u64 = 1_785_500_073;

#[test]
fn issue073_domain_graph_build_job_fsv() {
    let root = issue73_root();
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let (run, graph_readback) = {
        let vault = open_vault(&vault_dir);
        let source_cx_ids = store_loom_constellations(&vault);
        let request = happy_request(&root, &source_cx_ids);
        let run = run_domain_graph_build_job(&vault, &request, &clock())
            .expect("run domain graph build job");
        assert_eq!(run.report.supplied_edge_count, 4);
        assert!(run.report.loom_edge_count >= 2);
        assert_eq!(run.report.kernel_edge_count, 3);
        assert!(run.report.disconnected_component_count >= 3);
        assert_eq!(run.report.pair_gain.interaction_eager_count, 0);
        assert!(run.report.pair_gain.interaction_lazy_count >= 1);
        assert!(run.report.pair_gain.provisional_count >= 1);
        assert!(run.report.computed_kernel_recall.n_queries_tested >= 1);
        assert_eq!(run.report.csr_node_count, run.report.graph_node_count);
        assert_eq!(run.report.csr_edge_count, run.report.graph_edge_count);

        let persisted = read_domain_graph_build_report(&run.report_path).expect("read report");
        assert_eq!(persisted.schema_version, run.report.schema_version);
        assert_eq!(persisted.graph_edge_count, run.report.graph_edge_count);
        assert_eq!(
            persisted.pair_gain.interaction_lazy_count,
            run.report.pair_gain.interaction_lazy_count
        );
        let graph_readback = reopened_graph_readback(&vault_dir, &run);
        (run, graph_readback)
    };
    write_json(&root.join("graph-cf-reopen-readback.json"), &graph_readback);
    let edge_cases = edge_cases_fail_closed(&root);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 73,
        "proof_claim": "Poly runs a per-domain build job that composes pair-gain-gated Loom weaving, supplied on-ingest association edges, Graph CF persistence/readback, CSR rebuild, and computed-kernel recall from the Graph CF readback edges.",
        "minimum_sufficient_proof_corpus": {
            "loom_constellations": run.report.source_cx_ids.len(),
            "supplied_graph_edges": run.report.supplied_edge_count,
            "kernel_cycle_edges": run.report.kernel_edge_count,
            "pair_gain_rows": run.report.pair_gain.record.n_samples,
            "recall_rows": run.report.computed_kernel_recall.corpus_len,
            "why_this_is_sufficient": "Two stored constellations prove the real shape-aware Loom/XTerm path; three kernel edges are the smallest directed cycle that produces a non-empty computed FVS kernel; one extra non-kernel edge proves disconnected component handling; four pair-gain rows prove below-floor interactions are lazy; three recall rows prove the computed-kernel recall path runs from Graph CF readback edges.",
            "why_smaller_is_insufficient": "One constellation cannot prove a per-domain Loom job over multiple ingested records; fewer than three directed kernel edges cannot form a cycle; no extra component would not exercise disconnected graph handling; fewer than two pair-gain classes would be degenerate.",
            "why_larger_is_wasteful": "More records or edges repeat the same Loom, Graph CF, CSR, and recall paths without adding a distinct #73 invariant."
        },
        "graph_run": serde_json::to_value(&run.report).expect("run report JSON"),
        "graph_cf_reopen_readback": graph_readback,
        "edge_cases": edge_cases,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue073_domain_graph_build_job_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback["issue"], json!(73));
    assert_eq!(readback["passed"], json!(true));
    write_blake3sums(&root);
}

fn reopened_graph_readback(
    vault_dir: &Path,
    run: &calyx_poly::domain_graph_build_job::DomainGraphBuildRun,
) -> Value {
    let reopened = open_vault(vault_dir);
    let graph = PlainGraph::new(&reopened, COLLECTION).expect("graph");
    let snapshot = reopened.latest_seq();
    let mut edges = Vec::new();
    for expected in &run.report.readback_edges {
        let bytes = graph
            .get_edge(snapshot, expected.src, &expected.edge_type, expected.dst)
            .expect("Graph CF get")
            .expect("edge present after reopen");
        let value: Value = serde_json::from_slice(&bytes).expect("edge JSON");
        edges.push(json!({
            "src": expected.src,
            "dst": expected.dst,
            "edge_type": expected.edge_type,
            "value": value,
            "blake3": blake3::hash(&bytes).to_hex().to_string()
        }));
    }
    let rows = reopened
        .scan_cf_at(snapshot, ColumnFamily::Graph)
        .expect("scan Graph CF");
    json!({
        "snapshot_seq": snapshot,
        "graph_cf_rows": rows.len(),
        "readback_edge_count": edges.len(),
        "all_expected_edges_present": edges.len() == run.report.readback_edges.len(),
        "edges": edges
    })
}

fn edge_cases_fail_closed(root: &Path) -> Vec<Value> {
    let vault_dir = root.join("edge-empty-vault");
    let vault = open_vault(&vault_dir);
    let source_cx_ids: Vec<CxId> = Vec::new();
    let empty_edges: Vec<DomainGraphEdgeInput> = Vec::new();
    let request = DomainGraphBuildRequest {
        source_cx_ids: &source_cx_ids,
        supplied_edges: &empty_edges,
        ..happy_request(root, &source_cx_ids)
    };
    let err = run_domain_graph_build_job(&vault, &request, &clock())
        .expect_err("empty graph fails closed");
    assert_eq!(err.code(), ERR_DOMAIN_GRAPH_EMPTY);
    let edge_report = json!({
        "empty_graph": err.diagnostic(),
        "disconnected_components": {
            "handled_in_happy_path": true,
            "component_count_is_greater_than_one": true
        },
        "below_pair_gain_floor": {
            "handled_in_happy_path": true,
            "interaction_lazy": true
        }
    });
    write_json(&root.join("edge-cases.json"), &edge_report);
    vec![
        json!({"case": "empty_graph", "after": edge_report["empty_graph"]}),
        json!({"case": "disconnected_components", "after": edge_report["disconnected_components"]}),
        json!({"case": "below_pair_gain_floor", "after": edge_report["below_pair_gain_floor"]}),
    ]
}

fn happy_request<'a>(root: &'a Path, source_cx_ids: &'a [CxId]) -> DomainGraphBuildRequest<'a> {
    DomainGraphBuildRequest {
        domain: Domain::Crypto,
        collection: COLLECTION,
        panel_version: 73,
        source_cx_ids,
        supplied_edges: Box::leak(Box::new(supplied_edges())),
        pair_gain_matrix: Box::leak(Box::new(pair_gain_matrix())),
        recall_corpus: Box::leak(Box::new(recall_corpus())),
        kernel_anchors: Box::leak(Box::new(vec![cx(1)])),
        kernel_params: Box::leak(Box::new(kernel_params())),
        recall_params: Box::leak(Box::new(recall_params())),
        output_dir: root,
        loom_cache_capacity: 64,
    }
}

fn supplied_edges() -> Vec<DomainGraphEdgeInput> {
    vec![
        edge(
            1,
            2,
            EDGE_SHARED_ENTITY,
            "entity:a-b",
            "entity_graph_edges",
            true,
        ),
        edge(
            2,
            3,
            EDGE_TEMPORAL_LEAD_LAG,
            "temporal:b-c",
            "temporal_graph_edges",
            true,
        ),
        edge(3, 1, EDGE_KNN_RESOLVED, "knn:c-a", "knn_graph_edges", true),
        edge(
            4,
            5,
            EDGE_YES_NO_COMPLEMENT,
            "structural:d-e",
            "structural_edges",
            false,
        ),
    ]
}

fn edge(
    src: u8,
    dst: u8,
    edge_type: &str,
    relation_key: &str,
    source: &str,
    include_in_kernel: bool,
) -> DomainGraphEdgeInput {
    DomainGraphEdgeInput {
        src: cx(src),
        dst: cx(dst),
        edge_type: edge_type.to_string(),
        relation_key: relation_key.to_string(),
        source: source.to_string(),
        weight: 0.9,
        include_in_kernel,
    }
}

fn pair_gain_matrix() -> PanelMatrix {
    PanelMatrix::new(
        vec!["slot_a".to_string(), "slot_b".to_string()],
        vec![vec![0.1, 0.2, 0.3, 0.4], vec![0.4, 0.3, 0.2, 0.1]],
        vec![
            bool_anchor(false, 1),
            bool_anchor(true, 2),
            bool_anchor(false, 3),
            bool_anchor(true, 4),
        ],
    )
    .expect("pair-gain matrix")
}

fn recall_corpus() -> Vec<RecallQuery> {
    vec![
        RecallQuery {
            cx_id: cx(1),
            vector: vec![1.0, 0.0, 0.0],
        },
        RecallQuery {
            cx_id: cx(2),
            vector: vec![0.0, 1.0, 0.0],
        },
        RecallQuery {
            cx_id: cx(3),
            vector: vec![0.0, 0.0, 1.0],
        },
    ]
}

fn store_loom_constellations(vault: &AsterVault) -> Vec<CxId> {
    [70u8, 71u8]
        .iter()
        .map(|id| {
            vault
                .put(constellation(*id, vault.vault_id()))
                .expect("put constellation")
        })
        .collect()
}

fn constellation(id: u8, vault_id: VaultId) -> Constellation {
    let cx_id = cx(id);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(1),
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, id as f32 / 100.0],
        },
    );
    slots.insert(
        SlotId::new(2),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.8, id as f32 / 120.0],
        },
    );
    Constellation {
        cx_id,
        vault_id,
        panel_version: 73,
        created_at: TEST_TS,
        input_ref: InputRef {
            hash: [id; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn bool_anchor(value: bool, observed_at: u64) -> Anchor {
    Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(value),
        source: "uma:issue73".to_string(),
        observed_at,
        confidence: 1.0,
    }
}

fn kernel_params() -> KernelParams {
    KernelParams {
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 64,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    }
}

fn recall_params() -> RecallTestParams {
    RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 3,
        rng_seed: 73,
        min_recall_ratio: 0.95,
    }
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn open_vault(dir: &Path) -> AsterVault {
    AsterVault::open(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue73 vault")
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("vault id")
}

fn clock() -> FixedClock {
    FixedClock::new(TEST_TS)
}

fn issue73_root() -> PathBuf {
    calyx_fsv::fsv_root_or_target(
        "POLY_ISSUE73_FSV_ROOT",
        "issue73-domain-graph-build-job",
        || repo_root().join("target/fsv/issue73_domain_graph_build_job_20260707"),
    )
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn assert_c_drive(path: &Path) {
    #[cfg(not(windows))]
    let _ = path;
    #[cfg(windows)]
    assert!(
        path.to_string_lossy().starts_with("C:"),
        "{} must stay on C:",
        path.display()
    );
}
