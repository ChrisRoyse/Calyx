use std::collections::BTreeMap;

use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, OccurrenceContext, RetentionPolicy, append_occurrence,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_lodestar::{
    CALYX_LODESTAR_INVALID_FREQUENCY, CALYX_LODESTAR_MISSING_FREQUENCY, FREQ_WEIGHT,
    KernelGraphParams, KernelScope, TimeWindow, apply_frequency_bonuses, build_kernel_pipeline,
    build_kernel_pipeline_with_frequency, frequency_kernel_bonus, kernel_for_window,
    select_kernel_graph,
};
use calyx_mincut::tarjan_scc;
use calyx_paths::AssocGraph;
use proptest::prelude::*;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

#[test]
fn frequency_kernel_bonus_matches_log_bounds() {
    let expected_one = 2.0_f32.ln() / 10_001.0_f32.ln();
    assert_eq!(frequency_kernel_bonus(0), 0.0);
    assert!((frequency_kernel_bonus(1) - expected_one).abs() < 1e-6);
    assert_eq!(frequency_kernel_bonus(10_000), 1.0);
    assert_eq!(frequency_kernel_bonus(u64::MAX), 1.0);
}

#[test]
fn apply_frequency_bonuses_reads_base_cf_and_preserves_rank_when_betweenness_wins() {
    let vault = AsterVault::new(vault_id(), b"lodestar-freq-rank");
    let a = cx(1);
    let b = cx(2);
    put_base(&vault, a, Some(50.0));
    put_base(&vault, b, Some(1.0));
    let mut graph = graph_with_nodes(&[a, b]);
    graph.add_edge(a, b, 1.0).unwrap();
    let graph = graph.build();
    let mut kernel_graph = scored_graph(&graph, &[(a, 0.8), (b, 0.9)]);

    let reads = apply_frequency_bonuses(&mut kernel_graph, &vault).unwrap();
    let score_a = kernel_graph
        .scores
        .iter()
        .find(|score| score.id == a)
        .unwrap();
    let score_b = kernel_graph
        .scores
        .iter()
        .find(|score| score.id == b)
        .unwrap();

    assert_eq!(
        reads.iter().find(|read| read.cx_id == a).unwrap().frequency,
        50
    );
    assert!(
        (score_a.total_score - (0.8 + FREQ_WEIGHT * f64::from(frequency_kernel_bonus(50)))).abs()
            < 1e-6
    );
    assert!(
        (score_b.total_score - (0.9 + FREQ_WEIGHT * f64::from(frequency_kernel_bonus(1)))).abs()
            < 1e-6
    );
    assert_eq!(kernel_graph.scores[0].id, b);
}

#[test]
fn equal_betweenness_high_frequency_ranks_first() {
    let vault = AsterVault::new(vault_id(), b"lodestar-freq-equal");
    let high = cx(3);
    let low = cx(4);
    put_base(&vault, high, Some(50.0));
    put_base(&vault, low, Some(1.0));
    let graph = graph_with_nodes(&[high, low]).build();
    let mut kernel_graph = scored_graph(&graph, &[(high, 0.8), (low, 0.8)]);

    apply_frequency_bonuses(&mut kernel_graph, &vault).unwrap();

    assert_eq!(kernel_graph.scores[0].id, high);
    assert!(kernel_graph.scores[0].total_score > kernel_graph.scores[1].total_score);
}

#[test]
fn build_kernel_pipeline_with_frequency_applies_bonus_before_selection() {
    let vault = AsterVault::new(vault_id(), b"lodestar-freq-pipeline");
    let low = cx(1);
    let high = cx(9);
    put_base(&vault, low, Some(1.0));
    put_base(&vault, high, Some(10_000.0));
    let graph = graph_with_nodes(&[high, low]).build();
    let params = kernel_params();

    let plain = build_kernel_pipeline(&graph, &[], &params).unwrap();
    let weighted = build_kernel_pipeline_with_frequency(&graph, &[], &params, &vault).unwrap();

    assert_eq!(plain.kernel_graph, vec![low]);
    assert_eq!(weighted.kernel_graph, vec![high]);
}

#[test]
fn kernel_for_window_includes_only_occurrences_inside_half_open_window() {
    let vault = AsterVault::new(vault_id(), b"lodestar-window");
    let a = cx(5);
    let b = cx(6);
    put_base(&vault, a, None);
    put_base(&vault, b, None);
    append_times(&vault, a, &[50, 150, 250]);
    append_times(&vault, b, &[400, 500]);

    let window = TimeWindow::new(100, 300).unwrap();
    let result = kernel_for_window(&vault, &window, 10).unwrap();

    assert_eq!(
        result
            .nodes
            .iter()
            .map(|node| node.cx_id)
            .collect::<Vec<_>>(),
        vec![a]
    );
    assert_eq!(
        result.scope,
        KernelScope::TimeWindow {
            window: TimeWindow {
                start_secs: 100,
                end_secs: 300
            }
        }
    );
}

#[test]
fn empty_window_returns_empty_result_without_panic() {
    let vault = AsterVault::new(vault_id(), b"lodestar-empty-window");
    let a = cx(7);
    put_base(&vault, a, None);
    append_times(&vault, a, &[10, 20]);

    let result = kernel_for_window(&vault, &TimeWindow::new(100, 300).unwrap(), 10).unwrap();

    assert!(result.nodes.is_empty());
    assert_eq!(result.active_node_count, 0);
}

#[test]
fn missing_frequency_warns_and_uses_zero_bonus() {
    let vault = AsterVault::new(vault_id(), b"lodestar-missing-frequency");
    let id = cx(8);
    put_base(&vault, id, None);
    let graph = graph_with_nodes(&[id]).build();
    let mut kernel_graph = scored_graph(&graph, &[(id, 0.5)]);

    apply_frequency_bonuses(&mut kernel_graph, &vault).unwrap();

    assert_eq!(kernel_graph.scores[0].frequency_bonus, 0.0);
    assert!(kernel_graph.warnings[0].starts_with(CALYX_LODESTAR_MISSING_FREQUENCY));
}

#[test]
fn invalid_frequency_fails_closed_with_catalog_code() {
    let vault = AsterVault::new(vault_id(), b"lodestar-invalid-frequency");
    let id = cx(9);
    put_base(&vault, id, Some(1.5));
    let graph = graph_with_nodes(&[id]).build();
    let mut kernel_graph = scored_graph(&graph, &[(id, 0.5)]);

    let error = apply_frequency_bonuses(&mut kernel_graph, &vault).unwrap_err();

    assert_eq!(error.code(), CALYX_LODESTAR_INVALID_FREQUENCY);
}

proptest! {
    #[test]
    fn frequency_kernel_bonus_stays_bounded(value in any::<u64>()) {
        let bonus = frequency_kernel_bonus(value);
        prop_assert!(bonus.is_finite());
        prop_assert!((0.0..=1.0).contains(&bonus));
    }
}

fn scored_graph(
    graph: &AssocGraph,
    betweenness_rows: &[(CxId, f64)],
) -> calyx_lodestar::KernelGraph {
    let betweenness = betweenness_rows.iter().copied().collect::<BTreeMap<_, _>>();
    let params = KernelGraphParams {
        target_fraction: 1.0,
        degree_weight: 0.0,
        betweenness_weight: 1.0,
        groundedness_weight: 0.0,
        ..KernelGraphParams::default()
    };
    select_kernel_graph(graph, &tarjan_scc(graph), &betweenness, &[], &params).unwrap()
}

fn graph_with_nodes(ids: &[CxId]) -> calyx_paths::AssocGraphBuilder {
    let mut builder = AssocGraph::builder();
    for id in ids {
        builder.add_node(*id, 1.0).unwrap();
    }
    builder
}

fn kernel_params() -> calyx_lodestar::KernelParams {
    calyx_lodestar::KernelParams {
        kernel_graph: KernelGraphParams {
            target_fraction: 0.1,
            degree_weight: 0.0,
            betweenness_weight: 1.0,
            groundedness_weight: 0.0,
            ..KernelGraphParams::default()
        },
        ..calyx_lodestar::KernelParams::default()
    }
}

fn append_times(vault: &AsterVault, cx_id: CxId, times: &[i64]) {
    for time in times {
        append_occurrence(
            vault,
            cx_id,
            EpochSecs(*time),
            OccurrenceContext::new(format!("t={time}").into_bytes()).unwrap(),
            EpochSecs(*time),
            RetentionPolicy::default(),
        )
        .unwrap();
    }
}

fn put_base(vault: &AsterVault, cx_id: CxId, frequency: Option<f64>) {
    let mut cx = base_cx(cx_id);
    if let Some(frequency) = frequency {
        cx.scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    }
    vault.put(cx).unwrap();
}

fn base_cx(cx_id: CxId) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 42,
        created_at: 1_786_406_600,
        input_ref: InputRef {
            hash: [cx_id.to_bytes()[0]; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}
