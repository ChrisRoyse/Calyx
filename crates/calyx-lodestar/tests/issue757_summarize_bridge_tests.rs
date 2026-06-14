use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, CxId, FixedClock, VaultId};
use calyx_ledger::decode;
use calyx_lodestar::{
    AsterAssocMetadata, AsterAssocNodeProps, AsterSummarizeRequest,
    CALYX_TIMETRAVEL_BEFORE_HORIZON, DEFAULT_ASTER_ASSOC_COLLECTION, RecallTestParams, Scope,
    ScopeCache, SummarizeParams, encode_assoc_node_props, summarize_vault_as_of,
    summarize_vault_latest, write_assoc_metadata,
};

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn seeded_vault() -> AsterVault {
    let vault = AsterVault::new(vault_id(), b"issue757");
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    write_assoc_metadata(
        &vault,
        DEFAULT_ASTER_ASSOC_COLLECTION,
        &AsterAssocMetadata {
            retention_horizon: Some(1_000),
        },
    )
    .unwrap();
    for seed in 1..=6u8 {
        let props = AsterAssocNodeProps {
            embedding: Some(vec![seed as f32, 1.0]),
            ts: Some(1_000 + u64::from(seed)),
            anchors: (seed == 1)
                .then(|| AnchorKind::Label("domain".to_string()))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        graph
            .put_node(cx(seed), &encode_assoc_node_props(&props).unwrap())
            .unwrap();
    }
    for (src, dst) in [(1, 2), (2, 3), (3, 1), (4, 5), (5, 6), (6, 4), (3, 4)] {
        graph.put_edge(cx(src), "assoc", cx(dst), b"1").unwrap();
    }
    vault
}

#[test]
fn aster_bridge_summary_measures_recall_and_writes_aster_ledger() {
    let vault = seeded_vault();
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(7_000);
    let result = summarize_vault_latest(
        &vault,
        AsterSummarizeRequest {
            collection: DEFAULT_ASTER_ASSOC_COLLECTION,
            scope: Scope::AllAssociations,
            params: Some(SummarizeParams {
                max_kernel_size: Some(6),
                anchor_kind: Some(AnchorKind::Label("domain".to_string())),
                ..Default::default()
            }),
            recall_params: RecallTestParams {
                held_out_fraction: 1.0,
                top_k: 6,
                rng_seed: 1,
                min_recall_ratio: 0.0,
            },
        },
        &mut cache,
        &clock,
    )
    .expect("summarize latest");

    assert!(result.kernel_size > 0);
    assert!(result.kernel_only_recall > 0.0);
    assert!(result.kernel_only_recall <= 1.0);

    let rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger");
    assert_eq!(rows.len(), 1);
    let entry = decode(&rows[0].1).expect("decode ledger row");
    let payload: serde_json::Value = serde_json::from_slice(&entry.payload).unwrap();
    assert_eq!(payload["marker"], "SUMMARIZE_INVOKED");
    assert_eq!(
        payload["kernel_only_recall"].as_f64().unwrap() as f32,
        result.kernel_only_recall
    );
}

#[test]
fn aster_bridge_as_of_before_metadata_horizon_fails_closed() {
    let vault = seeded_vault();
    let before = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap();
    let mut cache = ScopeCache::new(8);
    let clock = FixedClock::new(7_000);
    let err = summarize_vault_as_of(
        &vault,
        AsterSummarizeRequest {
            collection: DEFAULT_ASTER_ASSOC_COLLECTION,
            scope: Scope::AllAssociations,
            params: None,
            recall_params: RecallTestParams::default(),
        },
        999,
        &mut cache,
        &clock,
    )
    .expect_err("before horizon must fail");
    let after = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap();

    assert_eq!(err.code, CALYX_TIMETRAVEL_BEFORE_HORIZON);
    assert_eq!(
        before.len(),
        after.len(),
        "no ledger row on fail-closed path"
    );
}
