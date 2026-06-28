mod support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CxId, FixedClock, VaultId};
use calyx_ledger::decode;
use calyx_lodestar::{
    AsterAssocMetadata, AsterAssocNodeProps, AsterSummarizeRequest, DEFAULT_ASTER_ASSOC_COLLECTION,
    RecallTestParams, Scope, ScopeCache, SummarizeParams, encode_assoc_node_props,
    summarize_vault_latest, write_assoc_metadata,
};
use serde_json::{Value, json};

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE1243_FSV_ROOT")
        .map(PathBuf::from)
        .expect("CALYX_ISSUE1243_FSV_ROOT is required")
}

fn seed_summary_vault(dir: &Path) -> AsterVault {
    let vault = AsterVault::new_durable(
        dir,
        vault_id(),
        b"issue1243-grounded-summary".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    write_assoc_metadata(
        &vault,
        DEFAULT_ASTER_ASSOC_COLLECTION,
        &AsterAssocMetadata {
            retention_horizon: Some(1),
        },
    )
    .unwrap();

    for seed in 1..=8u8 {
        let props = AsterAssocNodeProps {
            embedding: Some(vec![seed as f32, 1.0]),
            ts: Some(1_000 + u64::from(seed)),
            anchors: Some(AnchorKind::Label("domain".to_string()))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        graph
            .put_node(cx(seed), &encode_assoc_node_props(&props).unwrap())
            .unwrap();
    }
    for (src, dst) in [
        (1, 2),
        (2, 3),
        (3, 1),
        (4, 5),
        (5, 6),
        (6, 4),
        (3, 4),
        (6, 7),
        (7, 8),
    ] {
        graph.put_edge(cx(src), "prereq", cx(dst), b"1").unwrap();
    }
    vault.flush().unwrap();
    vault
}

fn ledger_payloads(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .map(|(_, row)| {
            let entry = decode(&row).unwrap();
            serde_json::from_slice(&entry.payload).unwrap()
        })
        .collect()
}

#[test]
#[ignore = "manual FSV for #1243; set CALYX_ISSUE1243_FSV_ROOT"]
fn issue1243_grounded_summary_and_replay_manual_fsv() {
    let root = fsv_root();
    support::reset_dir(&root);

    let summary_vault_dir = root.join("summary-vault");
    let vault = seed_summary_vault(&summary_vault_dir);
    let mut cache = ScopeCache::new(8);
    let result = summarize_vault_latest(
        &vault,
        AsterSummarizeRequest {
            collection: DEFAULT_ASTER_ASSOC_COLLECTION,
            scope: Scope::AllAssociations,
            params: Some(SummarizeParams {
                max_kernel_size: Some(8),
                require_grounded: true,
                anchor_kind: Some(AnchorKind::Label("domain".to_string())),
                cache_ttl_secs: Some(0),
            }),
            recall_params: RecallTestParams {
                held_out_fraction: 1.0,
                top_k: 8,
                rng_seed: 12_430,
                min_recall_ratio: 0.0,
            },
        },
        &mut cache,
        &FixedClock::new(12_430),
    )
    .expect("grounded summary");

    assert!(result.kernel_size > 0);
    assert!(!result.kernel_ids.is_empty());
    assert!(result.grounded_fraction >= 0.5);
    assert!((0.0..=1.0).contains(&result.kernel_only_recall));

    let payloads = ledger_payloads(&vault);
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["marker"], "SUMMARIZE_INVOKED");
    assert_eq!(
        payloads[0]["grounded_fraction"].as_f64().unwrap() as f32,
        result.grounded_fraction
    );
    assert!(payloads[0].get("generated_text").is_none());
    assert!(payloads[0].get("summary_text").is_none());

    let reproduce = support::run_reproduce_fsv(&root);
    let reproduced = reproduce["result"]["reproduced"].as_bool().unwrap();
    let max_drift = reproduce["result"]["max_drift"].as_f64().unwrap();
    assert!(reproduced);
    assert!(max_drift <= 1.0e-3);
    assert_eq!(reproduce["before_reproduce_rows"], 3);
    assert_eq!(reproduce["after_reproduce_rows"], 4);
    assert_eq!(
        reproduce["original_score_bytes_hex"],
        reproduce["reproduced_score_bytes_hex"]
    );
    assert_eq!(reproduce["admin_payload"]["type"], "reproduce_v1");

    let readback = json!({
        "issue": 1243,
        "surface": "grounded_progress_summary_plus_provenance_replay",
        "source_of_truth": {
            "summary_vault": summary_vault_dir,
            "reproduce_ledger_cf": root.join("reproduce-ledger-cf"),
        },
        "grounded_progress_summary": {
            "kind": "structural_kernel_node_ids_only",
            "no_generated_text": true,
            "kernel_ids": result.kernel_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "kernel_size": result.kernel_size,
            "kernel_only_recall": result.kernel_only_recall,
            "grounded_fraction": result.grounded_fraction,
            "approx_factor": result.approx_factor,
            "ledger_ref": result.ledger_ref,
            "ledger_payloads": payloads,
        },
        "provenance_replay": reproduce,
    });
    let out = root.join("issue1243-grounded-summary-replay-fsv.json");
    fs::write(&out, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("ISSUE1243_FSV_READBACK={}", out.display());
}
