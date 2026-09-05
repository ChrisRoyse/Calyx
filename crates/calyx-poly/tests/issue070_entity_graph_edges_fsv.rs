//! Issue #70 - shared holder/wallet entity edges into Aster Graph CF.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use calyx_poly::entity_graph_edges::{
    EDGE_SHARED_ENTITY, ERR_ENTITY_GRAPH_INVALID_INPUT, EntityMarketEvidence,
    compute_entity_graph_edges, persist_entity_graph_edges,
};
use calyx_poly::model::{CounterpartyVolume, HolderShare, MakerShare, MakerShareEvidenceSource};
use serde_json::{Value, json};

use support::{collect_files, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue70-entity-graph-edges";
const COLLECTION: &str = "poly_issue70_entity_graph";
const WHALE: &str = "0xabcdef0000000000000000000000000000000001";

#[test]
fn issue070_entity_graph_edges_fsv() {
    let root = issue70_root();
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let records = happy_records();
    let run = {
        let vault = open_vault(&vault_dir);
        persist_entity_graph_edges(&vault, COLLECTION, "crypto", &records)
            .expect("persist entity graph edges")
    };
    assert_eq!(run.computed.edge_count, 2);
    assert_eq!(run.computed.absent.len(), 2);
    assert_eq!(run.graph_cf_row_count, 7);
    assert_eq!(run.readback_edges.len(), run.computed.edges.len());
    assert!(
        run.computed
            .edges
            .iter()
            .all(|edge| edge.edge_type == EDGE_SHARED_ENTITY)
    );
    assert!(
        run.computed
            .edges
            .iter()
            .all(|edge| (edge.weight - 0.8).abs() < 1e-9)
    );
    assert_eq!(run.computed.edges[0].shared_entities[0].address, WHALE);

    let graph_readback = reopened_graph_readback(&vault_dir, &run);
    write_json(&root.join("graph-cf-readback.json"), &graph_readback);
    let edge_cases = edge_cases_fail_closed(&root);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 70,
        "proof_claim": "Poly normalizes public holder/maker/counterparty entity evidence, computes shared-entity market edges only from real overlap, persists typed association.shared_entity edges into Aster Graph CF, and reads those Graph CF rows back byte-for-byte.",
        "minimum_sufficient_proof_corpus": {
            "market_records": records.len(),
            "loaded_edges": run.computed.edge_count,
            "why_this_is_sufficient": "Three records prove positive overlap, address normalization, dominant-whale weighting, and no-edge/no-fabrication for an unrelated market in one run.",
            "why_smaller_is_insufficient": "Two records can prove overlap or no-overlap, but not both a positive edge and an unrelated no-edge pair in the same source-of-truth run.",
            "why_larger_is_wasteful": "The behavior is deterministic normalization/intersection plus Graph CF persistence; more markets repeat the same path without adding proof."
        },
        "graph_run": serde_json::to_value(&run).expect("run JSON"),
        "graph_cf_readback": graph_readback,
        "edge_cases": edge_cases,
        "physical_files": files,
        "scope_note": "This materializes entity-overlap edges from typed evidence. #27 still owns large on-chain source capture/backfill.",
        "passed": true
    });
    let report_path = root.join("issue070_entity_graph_edges_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

fn reopened_graph_readback(
    vault_dir: &Path,
    run: &calyx_poly::entity_graph_edges::EntityGraphRun,
) -> Value {
    let reopened = open_vault(vault_dir);
    let graph = PlainGraph::new(&reopened, COLLECTION).expect("graph");
    let snapshot = reopened.latest_seq();
    let mut edges = Vec::new();
    for expected in &run.readback_edges {
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
        "edges": edges
    })
}

fn edge_cases_fail_closed(root: &Path) -> Vec<Value> {
    let no_shared = compute_entity_graph_edges(&[
        market(20, "edge-no-shared-a", &[holder("0xaaa", 10.0)]),
        market(21, "edge-no-shared-b", &[holder("0xbbb", 10.0)]),
    ])
    .expect("no shared entities is an absent relation, not an error");
    assert_eq!(no_shared.edge_count, 0);
    assert!(
        no_shared
            .absent
            .iter()
            .any(|row| row.code == "no_shared_entities")
    );

    let normalized = compute_entity_graph_edges(&[
        market(
            22,
            "edge-case-a",
            &[holder("  0XCAFE  ", 9.0), holder("0xsmall", 1.0)],
        ),
        market(
            23,
            "edge-case-b",
            &[holder("0xcafe", 8.0), holder("0xother", 2.0)],
        ),
    ])
    .expect("normalized overlap");
    assert_eq!(normalized.edge_count, 2);
    assert_eq!(normalized.edges[0].shared_entities[0].address, "0xcafe");
    assert!((normalized.edges[0].weight - 0.8).abs() < 1e-9);

    let invalid = compute_entity_graph_edges(&[
        market(24, "edge-invalid-a", &[holder("0xvalid", 1.0)]),
        market(25, "edge-invalid-b", &[holder("0xbad", f64::NAN)]),
    ])
    .expect_err("non-finite amount fails closed");
    assert_eq!(invalid.code(), ERR_ENTITY_GRAPH_INVALID_INPUT);

    let edge_report = json!({
        "no_shared_entities": no_shared,
        "address_normalization_and_dominant_whale": normalized,
        "non_finite_amount": invalid.diagnostic()
    });
    write_json(&root.join("edge-cases.json"), &edge_report);
    vec![
        json!({"case": "no_shared_entities", "after": edge_report["no_shared_entities"]}),
        json!({"case": "address_normalization_and_dominant_whale", "after": edge_report["address_normalization_and_dominant_whale"]}),
        json!({"case": "non_finite_amount", "after": edge_report["non_finite_amount"]}),
    ]
}

fn happy_records() -> Vec<EntityMarketEvidence> {
    vec![
        EntityMarketEvidence {
            cx_id: cx(1),
            condition_id: "shared-a".to_string(),
            holders: vec![
                holder("  0XABCDEF0000000000000000000000000000000001  ", 90.0),
                holder("0xsmall-a", 10.0),
            ],
            makers: vec![maker("0xmaker-a", 5.0)],
            counterparties: vec![counterparty("0xcp-a", 100.0)],
        },
        EntityMarketEvidence {
            cx_id: cx(2),
            condition_id: "shared-b".to_string(),
            holders: vec![holder(WHALE, 80.0), holder("0xsmall-b", 20.0)],
            makers: vec![],
            counterparties: vec![counterparty("0xcp-b", 50.0)],
        },
        EntityMarketEvidence {
            cx_id: cx(3),
            condition_id: "unrelated-c".to_string(),
            holders: vec![holder("0xunrelated", 100.0)],
            makers: vec![],
            counterparties: vec![],
        },
    ]
}

fn market(id: u8, condition_id: &str, holders: &[HolderShare]) -> EntityMarketEvidence {
    EntityMarketEvidence {
        cx_id: cx(id),
        condition_id: condition_id.to_string(),
        holders: holders.to_vec(),
        makers: Vec::new(),
        counterparties: Vec::new(),
    }
}

fn holder(wallet: &str, amount: f64) -> HolderShare {
    HolderShare {
        wallet: wallet.to_string(),
        amount,
        outcome_index: 0,
    }
}

fn maker(maker: &str, size: f64) -> MakerShare {
    MakerShare {
        maker: maker.to_string(),
        size,
        evidence_source: MakerShareEvidenceSource::RestingClobOrderBook,
    }
}

fn counterparty(counterparty: &str, volume: f64) -> CounterpartyVolume {
    CounterpartyVolume {
        counterparty: counterparty.to_string(),
        volume,
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
    .expect("open issue70 vault")
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("vault id")
}

fn issue70_root() -> PathBuf {
    calyx_fsv::fsv_root_or_target(
        "POLY_ISSUE70_FSV_ROOT",
        "issue70-entity-graph-edges",
        || repo_root().join("target/fsv/issue70_entity_graph_edges_20260707"),
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
