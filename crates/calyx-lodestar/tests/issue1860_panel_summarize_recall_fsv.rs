//! Full-State Verification for #1860: the Lodestar summarize recall bridge must
//! consume panel-native graphs (the sealed ordered panel stored in
//! `AsterAssocNodeProps.embeddings`) through deterministic per-lane RRF fusion,
//! not the legacy one-vector `props.embedding` field.
//!
//! Source of Truth: the on-disk Graph + Ledger column families of a durable
//! `AsterVault`. Every panel assertion is re-derived from an INDEPENDENT physical
//! readback (`PhysicalAsterAssocSnapshot::latest` + `PhysicalPlainGraph`), never
//! from the live `summarize` return value alone. Synthetic, hand-seeded vectors;
//! multiple edge cases print SoT ledger-row counts BEFORE and AFTER.
//!
//! Run: `cargo test -p calyx-lodestar --test __calyx_integration_suite_1 issue1860 -- --nocapture`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::{PhysicalPlainGraph, PlainGraph};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CxId, SlotId, VaultId};
use calyx_ledger::decode;
use calyx_lodestar::{
    ASTER_ASSOC_METADATA_KEY, AsterAssocMetadata, AsterAssocNodeProps, AsterSummarizeRequest,
    PANEL_ASTER_ASSOC_COLLECTION, PANEL_RRF_K, PanelVectors, PhysicalAsterAssocSnapshot,
    RecallTestParams, Scope, ScopeCache, SummarizeParams, SummarizeRecall, encode_assoc_node_props,
    rank_panel_candidates, summarize_vault_latest, write_assoc_metadata,
};

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn slot1() -> SlotId {
    SlotId::new(1)
}

fn slot2() -> SlotId {
    SlotId::new(2)
}

/// Deterministic dense lane #1 (dim 4). Non-zero norm, all finite.
fn lane1(seed: u8) -> Vec<f32> {
    vec![seed as f32, 1.0, 0.5, 0.25]
}

/// Deterministic dense lane #2 (dim 3). Distinct geometry from lane #1.
fn lane2(seed: u8) -> Vec<f32> {
    vec![1.0, seed as f32, 0.75]
}

fn panel(seed: u8) -> PanelVectors {
    BTreeMap::from([(slot1(), lane1(seed)), (slot2(), lane2(seed))])
}

fn recall_params() -> RecallTestParams {
    RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 6,
        rng_seed: 1,
        min_recall_ratio: 0.0,
    }
}

fn summarize_params() -> SummarizeParams {
    SummarizeParams {
        max_kernel_size: Some(6),
        anchor_kind: Some(AnchorKind::Label("domain".to_string())),
        ..Default::default()
    }
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("calyx-issue1860-{tag}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The two triangles + bridge topology shared by every case: nodes cx(1..=6),
/// anchors on cx(1) and cx(4). `mutate` may corrupt one node's embeddings to
/// exercise a fail-closed lane.
fn build_panel_vault(
    dir: &Path,
    metadata: &AsterAssocMetadata,
    mutate: impl Fn(u8, &mut AsterAssocNodeProps),
) -> AsterVault {
    let vault =
        AsterVault::new_durable(dir, vault_id(), b"issue1860", VaultOptions::default()).unwrap();
    let graph = PlainGraph::new(&vault, PANEL_ASTER_ASSOC_COLLECTION).unwrap();
    write_assoc_metadata(&vault, PANEL_ASTER_ASSOC_COLLECTION, metadata).unwrap();
    for seed in 1..=6u8 {
        let mut props = AsterAssocNodeProps {
            embedding: None,
            embeddings: panel(seed),
            ts: Some(1_000 + u64::from(seed)),
            anchors: (seed == 1 || seed == 4)
                .then(|| AnchorKind::Label("domain".to_string()))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        mutate(seed, &mut props);
        graph
            .put_node(cx(seed), &encode_assoc_node_props(&props).unwrap())
            .unwrap();
    }
    for (src, dst) in [(1, 2), (2, 3), (3, 1), (4, 5), (5, 6), (6, 4), (3, 4)] {
        graph.put_edge(cx(src), "assoc", cx(dst), b"1").unwrap();
    }
    graph.rebuild_csr(vault.latest_seq()).unwrap();
    vault.flush().unwrap();
    vault
}

fn panel_metadata() -> AsterAssocMetadata {
    AsterAssocMetadata {
        embedding_slots: vec![slot1(), slot2()],
        fusion: Some("rrf".to_string()),
        rrf_k: Some(PANEL_RRF_K),
        ..Default::default()
    }
}

fn ledger_rows(vault: &AsterVault) -> usize {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger")
        .len()
}

/// INDEPENDENT physical readback of the single SUMMARIZE_INVOKED payload from disk.
fn summarize_payload_from_disk(dir: &Path) -> serde_json::Value {
    let reopened =
        AsterVault::open(dir, vault_id(), b"issue1860", VaultOptions::default()).unwrap();
    let rows = reopened
        .scan_cf_at(reopened.latest_seq(), ColumnFamily::Ledger)
        .expect("scan reopened ledger");
    assert_eq!(
        rows.len(),
        1,
        "exactly one provenance row physically on disk"
    );
    let entry = decode(&rows[0].1).expect("decode ledger row");
    serde_json::from_slice(&entry.payload).expect("decode payload json")
}

#[test]
fn issue1860_panel_native_summarize_recall_fsv() {
    let dir = tmp_dir("panel");
    let vault = build_panel_vault(&dir, &panel_metadata(), |_, _| {});

    // ---- Trigger: summarize a panel-native graph. Pre-fix this failed closed
    //      with CALYX_SUMMARIZE_RECALL_MISSING_EMBEDDING because recall_inputs read
    //      only props.embedding (None for every panel node).
    let mut cache = ScopeCache::new(8);
    let clock = calyx_core::SystemClock;
    println!(
        "[panel] BEFORE summarize: on-vault ledger rows = {}",
        ledger_rows(&vault)
    );
    let result = summarize_vault_latest(
        &vault,
        AsterSummarizeRequest {
            collection: PANEL_ASTER_ASSOC_COLLECTION,
            scope: Scope::AllAssociations,
            params: Some(summarize_params()),
            recall_params: recall_params(),
        },
        &mut cache,
        &clock,
    )
    .expect("panel-native summarize must now succeed");
    vault.flush().unwrap();
    println!(
        "[panel] AFTER summarize: kernel_size={} recall={} grounded={} ledger_rows={}",
        result.kernel_size,
        result.kernel_only_recall,
        result.grounded_fraction,
        ledger_rows(&vault)
    );

    assert!(
        result.kernel_size > 0,
        "non-trivial panel graph => non-empty kernel"
    );
    assert!(
        (0.0..=1.0).contains(&result.kernel_only_recall),
        "measured panel recall is a ratio in [0,1]: {}",
        result.kernel_only_recall
    );

    // ---- SoT #1: the SUMMARIZE_INVOKED payload carries byte-auditable per-lane
    //      provenance proving the PANEL path ran (these fields never appear on the
    //      legacy path).
    let payload = summarize_payload_from_disk(&dir);
    assert_eq!(payload["marker"], "SUMMARIZE_INVOKED");
    assert_eq!(payload["embedding_slots"], serde_json::json!([1, 2]));
    assert_eq!(payload["fusion"], "rrf");
    assert_eq!(payload["rrf_k"].as_u64().unwrap(), PANEL_RRF_K as u64);
    assert_eq!(payload["n_lanes"].as_u64().unwrap(), 2);
    assert_eq!(
        payload["kernel_only_recall"].as_f64().unwrap() as f32,
        result.kernel_only_recall
    );

    // ---- SoT #2: physical Graph CF readback — node props carry no legacy vector
    //      and exactly the two contracted lanes; metadata seals the panel contract.
    let physical = PhysicalPlainGraph::open_latest(&dir, PANEL_ASTER_ASSOC_COLLECTION).unwrap();
    for (id, bytes) in physical.node_props().unwrap() {
        let decoded: AsterAssocNodeProps = serde_json::from_slice(&bytes).unwrap();
        assert!(
            decoded.embedding.is_none(),
            "panel node {id} has no legacy embedding"
        );
        assert_eq!(
            decoded.embeddings.keys().copied().collect::<Vec<_>>(),
            vec![slot1(), slot2()],
            "panel node {id} carries exactly the two contracted lanes"
        );
    }
    let meta_bytes = physical
        .get_metadata(ASTER_ASSOC_METADATA_KEY)
        .unwrap()
        .expect("physical metadata bytes");
    let meta: AsterAssocMetadata = serde_json::from_slice(&meta_bytes).unwrap();
    assert_eq!(meta.embedding_slots, vec![slot1(), slot2()]);
    assert_eq!(meta.fusion.as_deref(), Some("rrf"));
    assert_eq!(meta.rrf_k, Some(PANEL_RRF_K));

    // ---- SoT #3: re-derive the recall corpus purely from disk bytes and prove
    //      per-lane evidence (no-flatten RRF) is materialized.
    let snap = PhysicalAsterAssocSnapshot::latest(&dir, PANEL_ASTER_ASSOC_COLLECTION).unwrap();
    let inputs = snap
        .panel_recall_inputs(recall_params())
        .unwrap()
        .expect("physical panel recall inputs");
    match inputs.measurement() {
        SummarizeRecall::Panel { slots, full, .. } => {
            assert_eq!(
                slots,
                vec![slot1(), slot2()],
                "physical readback yields the sealed panel"
            );
            assert_eq!(full.len(), 6, "corpus holds every constellation");
            // Byte-equivalence with the hand-seeded vectors.
            for seed in 1..=6u8 {
                assert_eq!(full.get(&cx(seed)).unwrap(), &panel(seed));
            }
            // Per-lane fusion: querying with node 1's panel returns fused hits whose
            // lanes preserve BOTH slot ids separately — never flattened/averaged.
            let query = full.get(&cx(1)).unwrap().clone();
            let hits = rank_panel_candidates(&query, full, &slots, PANEL_RRF_K).unwrap();
            assert_eq!(hits.len(), 6);
            let top = &hits[0];
            let lane_slots = top.lanes.iter().map(|l| l.slot).collect::<Vec<_>>();
            assert_eq!(
                lane_slots,
                vec![slot1(), slot2()],
                "each fused hit keeps per-lane evidence"
            );
            assert!(
                top.lanes
                    .iter()
                    .all(|l| l.cosine.is_finite() && l.rrf_contribution > 0.0)
            );
            println!(
                "[panel] top fused hit {} score={:.6} lanes={:?}",
                top.cx_id,
                top.score,
                top.lanes
                    .iter()
                    .map(|l| (l.slot.get(), l.rank, l.cosine))
                    .collect::<Vec<_>>()
            );
        }
        SummarizeRecall::Legacy { .. } => {
            panic!("panel graph must not route through the legacy path")
        }
        SummarizeRecall::TypedPanel { .. } => {
            panic!("issue1860 legacy dense-panel fixture must read back as Panel, not TypedPanel")
        }
    }
}

#[test]
fn issue1860_legacy_one_vector_graph_stays_diagnostic_only() {
    let dir = tmp_dir("legacy");
    // Legacy contract: empty embedding_slots, nodes carry the single props.embedding.
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"issue1860", VaultOptions::default()).unwrap();
    let graph = PlainGraph::new(&vault, "default").unwrap();
    write_assoc_metadata(&vault, "default", &AsterAssocMetadata::default()).unwrap();
    for seed in 1..=6u8 {
        let props = AsterAssocNodeProps {
            embedding: Some(vec![seed as f32, 1.0]),
            ts: Some(1_000 + u64::from(seed)),
            anchors: (seed == 1 || seed == 4)
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
    graph.rebuild_csr(vault.latest_seq()).unwrap();
    vault.flush().unwrap();

    let mut cache = ScopeCache::new(8);
    let clock = calyx_core::SystemClock;
    let result = summarize_vault_latest(
        &vault,
        AsterSummarizeRequest {
            collection: "default",
            scope: Scope::AllAssociations,
            params: Some(summarize_params()),
            recall_params: recall_params(),
        },
        &mut cache,
        &clock,
    )
    .expect("legacy one-vector summarize still works (diagnostic-only)");
    vault.flush().unwrap();

    assert!(result.kernel_size > 0);
    assert!((0.0..=1.0).contains(&result.kernel_only_recall));
    // The legacy payload keeps the historic shape — no panel provenance fields.
    let payload = summarize_payload_from_disk(&dir);
    assert_eq!(payload["marker"], "SUMMARIZE_INVOKED");
    assert!(
        payload.get("embedding_slots").is_none(),
        "legacy payload carries no panel lanes"
    );
    assert!(payload.get("fusion").is_none());
    println!(
        "[legacy] diagnostic-only recall={} (no panel fields in payload)",
        result.kernel_only_recall
    );
}

/// A panel graph with a corrupted lane must fail closed with a {code,message,
/// remediation} error and write NO SUMMARIZE_INVOKED row. `tag` names the case,
/// `mutate` corrupts one node.
fn assert_lane_fails_closed(
    tag: &str,
    metadata: AsterAssocMetadata,
    mutate: impl Fn(u8, &mut AsterAssocNodeProps),
) {
    let dir = tmp_dir(tag);
    let vault = build_panel_vault(&dir, &metadata, mutate);
    let before = ledger_rows(&vault);
    println!("[{tag}] BEFORE summarize: ledger rows = {before}");

    let mut cache = ScopeCache::new(8);
    let clock = calyx_core::SystemClock;
    let err = summarize_vault_latest(
        &vault,
        AsterSummarizeRequest {
            collection: PANEL_ASTER_ASSOC_COLLECTION,
            scope: Scope::AllAssociations,
            params: Some(summarize_params()),
            recall_params: recall_params(),
        },
        &mut cache,
        &clock,
    )
    .expect_err("corrupt panel lane must fail closed");
    vault.flush().unwrap();
    let after = ledger_rows(&vault);
    println!(
        "[{tag}] AFTER summarize: ledger rows = {after} err.code={} msg={}",
        err.code, err.message
    );

    assert!(!err.code.is_empty(), "fail-closed error carries a code");
    assert!(
        !err.message.is_empty(),
        "fail-closed error states what failed"
    );
    assert!(
        !err.remediation.is_empty(),
        "fail-closed error states how to fix it"
    );
    assert_eq!(
        after, before,
        "[{tag}] no SUMMARIZE_INVOKED row on a fail-closed evaluation"
    );
}

#[test]
fn issue1860_missing_lane_fails_closed() {
    // Node 3 drops slot #2 entirely.
    assert_lane_fails_closed("missing-lane", panel_metadata(), |seed, props| {
        if seed == 3 {
            props.embeddings.remove(&slot2());
        }
    });
}

#[test]
fn issue1860_extra_lane_fails_closed() {
    // Node 5 carries an uncontracted slot #3.
    assert_lane_fails_closed("extra-lane", panel_metadata(), |seed, props| {
        if seed == 5 {
            props.embeddings.insert(SlotId::new(3), vec![1.0, 2.0, 3.0]);
        }
    });
}

#[test]
fn issue1860_nonfinite_lane_fails_closed() {
    // Node 2 injects a NaN into lane #1.
    assert_lane_fails_closed("nan-lane", panel_metadata(), |seed, props| {
        if seed == 2 {
            props
                .embeddings
                .insert(slot1(), vec![f32::NAN, 1.0, 0.5, 0.25]);
        }
    });
}

#[test]
fn issue1860_ambiguous_contract_fails_closed() {
    // Both a legacy embedding_slot AND a panel embedding_slots => ambiguous.
    let mut metadata = panel_metadata();
    metadata.embedding_slot = Some(slot1());
    let dir = tmp_dir("ambiguous");
    let vault = build_panel_vault(&dir, &metadata, |_, _| {});
    let before = ledger_rows(&vault);
    let mut cache = ScopeCache::new(8);
    let clock = calyx_core::SystemClock;
    let err = summarize_vault_latest(
        &vault,
        AsterSummarizeRequest {
            collection: PANEL_ASTER_ASSOC_COLLECTION,
            scope: Scope::AllAssociations,
            params: Some(summarize_params()),
            recall_params: recall_params(),
        },
        &mut cache,
        &clock,
    )
    .expect_err("ambiguous contract must fail closed");
    vault.flush().unwrap();
    assert_eq!(err.code, "CALYX_ASSOC_BRIDGE_AMBIGUOUS_CONTRACT");
    assert!(!err.remediation.is_empty());
    assert_eq!(ledger_rows(&vault), before, "no row on ambiguous contract");
    println!("[ambiguous] err.code={} (no ledger row)", err.code);
}

#[test]
fn issue1860_unsupported_fusion_fails_closed() {
    // Panel slots present but fusion is not the sealed rrf/PANEL_RRF_K contract.
    let mut metadata = panel_metadata();
    metadata.fusion = Some("mean".to_string());
    let dir = tmp_dir("fusion");
    let vault = build_panel_vault(&dir, &metadata, |_, _| {});
    let before = ledger_rows(&vault);
    let mut cache = ScopeCache::new(8);
    let clock = calyx_core::SystemClock;
    let err = summarize_vault_latest(
        &vault,
        AsterSummarizeRequest {
            collection: PANEL_ASTER_ASSOC_COLLECTION,
            scope: Scope::AllAssociations,
            params: Some(summarize_params()),
            recall_params: recall_params(),
        },
        &mut cache,
        &clock,
    )
    .expect_err("unsupported fusion must fail closed");
    vault.flush().unwrap();
    assert_eq!(err.code, "CALYX_ASSOC_BRIDGE_UNSUPPORTED_FUSION");
    assert_eq!(ledger_rows(&vault), before, "no row on unsupported fusion");
    println!("[fusion] err.code={} (no ledger row)", err.code);
}
