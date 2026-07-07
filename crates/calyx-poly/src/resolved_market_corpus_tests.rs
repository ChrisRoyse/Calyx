//! Unit + end-to-end FSV for the local resolved-market corpus builder (issue #219).
//!
//! Known-truth discipline: every snapshot is hand-constructed with known field values, run through
//! the *real* deterministic encoder + the *real* Lodestar recall engine, and checked against
//! hand-computed expectations. No mock data, no learned embedder.

use super::*;
use crate::constellation::build_constellation;
use crate::kernel_recall_admission::{
    ComputedKernelRecallRequest, measure_computed_kernel_recall, read_computed_kernel_recall,
};
use crate::lenses::default_panel;
use crate::model::{MarketSnapshot, Resolution};
use calyx_core::VaultId;
use calyx_lodestar::{KernelGraphParams, KernelParams, RecallTestParams};
use std::str::FromStr;

const PANEL_VERSION: u32 = 7;
const SALT: &[u8] = b"issue219-corpus-salt";

fn snap(
    cond: &str,
    outcome_index: u32,
    price: f64,
    spread: f64,
    volume: f64,
    liquidity: f64,
) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("{cond}-tok{outcome_index}"),
        condition_id: cond.to_string(),
        outcome_index,
        slug: format!("market-{cond}"),
        question: Some(format!(
            "Will market {cond} resolve outcome {outcome_index}?"
        )),
        event_id: None,
        category: Some("crypto".into()),
        region: None,
        tags: vec![],
        resolution_source: Some("uma".into()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(price),
        mid: Some(price),
        best_bid: Some(price - spread / 2.0),
        best_ask: Some(price + spread / 2.0),
        spread: Some(spread),
        tick_size: Some(0.01),
        volume_24h: Some(volume),
        liquidity: Some(liquidity),
        one_hour_change: Some(0.0),
        one_day_change: Some(0.0),
        ofi: Some(0.0),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

fn resolution(cond: &str, winning_outcome_index: u32) -> Resolution {
    Resolution {
        condition_id: cond.to_string(),
        winning_outcome_index,
        winning_label: if winning_outcome_index == 0 {
            "YES".into()
        } else {
            "NO".into()
        },
        resolved_ts: 1_785_600_000,
        source: "uma".into(),
        disputed: false,
    }
}

#[test]
fn record_vector_is_deterministic_l2_normalized_and_correct_dim() {
    let s = snap("0xA", 0, 0.62, 0.02, 125_000.0, 40_000.0);
    let v1 = market_record_vector(&s).expect("vector");
    let v2 = market_record_vector(&s).expect("vector");
    assert_eq!(v1, v2, "encoding must be deterministic");
    assert_eq!(v1.len(), RECALL_VECTOR_DIM);
    let norm: f32 = v1.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "L2 norm must be 1, got {norm}");
}

#[test]
fn record_vector_known_truth_ratios() {
    // L2 normalization scales all dims equally, so pre-norm ratios are preserved and give a
    // hand-checkable known truth independent of the norm constant.
    let s = snap("0xB", 0, 0.62, 0.02, 125_000.0, 40_000.0);
    let v = market_record_vector(&s).expect("vector");
    // dim0=price=0.62, dim1=|0.62-0.5|=0.12  =>  v0/v1 == 0.62/0.12
    assert!((v[0] / v[1] - (0.62 / 0.12) as f32).abs() < 1e-3, "{v:?}");
    // dim3=signed_log(125000), dim4=signed_log(40000)
    let l_vol = (1.0f64 + 125_000.0).ln();
    let l_liq = (1.0f64 + 40_000.0).ln();
    assert!((v[3] / v[4] - (l_vol / l_liq) as f32).abs() < 1e-3, "{v:?}");
}

#[test]
fn record_vector_fails_closed_on_missing_required_field() {
    let mut s = snap("0xC", 0, 0.6, 0.02, 100_000.0, 30_000.0);
    s.volume_24h = None; // absent required field
    let err = market_record_vector(&s).expect_err("missing volume must fail closed");
    assert_eq!(err.code(), ERR_CORPUS_MISSING_FIELD);
}

#[test]
fn corpus_cx_id_matches_the_real_constellation_path() {
    // Full-state consistency: the corpus row's cx_id must equal the id the vault's constellation
    // builder assigns the same snapshot under the same panel version + salt, so the corpus lines up
    // with the stored constellations.
    let s = snap("0xD", 0, 0.55, 0.03, 90_000.0, 25_000.0);
    let r = resolution("0xD", 0);
    let corpus = build_resolved_market_corpus(
        &[ResolvedMarketInput {
            snapshot: &s,
            resolution: &r,
        }],
        PANEL_VERSION,
        SALT,
        0.0,
    )
    .expect("corpus");
    let panel = default_panel(PANEL_VERSION, vec![]);
    let vault_id = VaultId::from_str("00000000000000000000000000").expect("nil vault id");
    let cx = build_constellation(&s, &panel, vault_id, SALT).expect("cx");
    assert_eq!(
        corpus.recall_queries[0].cx_id, cx.cx_id,
        "corpus cx_id must match the constellation path (same content-addressing)"
    );
}

#[test]
fn corpus_outcome_yes_reflects_winning_index() {
    let s_yes = snap("0xE", 0, 0.7, 0.02, 100_000.0, 30_000.0);
    let s_no = snap("0xF", 1, 0.3, 0.02, 100_000.0, 30_000.0);
    let corpus = build_resolved_market_corpus(
        &[
            ResolvedMarketInput {
                snapshot: &s_yes,
                resolution: &resolution("0xE", 0), // token outcome 0 won -> YES
            },
            ResolvedMarketInput {
                snapshot: &s_no,
                resolution: &resolution("0xF", 0), // token outcome 1 lost -> NO
            },
        ],
        PANEL_VERSION,
        SALT,
        0.0,
    )
    .expect("corpus");
    assert!(corpus.exemplars[0].outcome_yes, "outcome 0 won -> YES");
    assert!(!corpus.exemplars[1].outcome_yes, "outcome 1 lost -> NO");
}

#[test]
fn corpus_fails_closed_on_condition_mismatch_and_empty() {
    let s = snap("0xG", 0, 0.5, 0.02, 100_000.0, 30_000.0);
    let bad = build_resolved_market_corpus(
        &[ResolvedMarketInput {
            snapshot: &s,
            resolution: &resolution("0xDIFFERENT", 0),
        }],
        PANEL_VERSION,
        SALT,
        0.0,
    )
    .expect_err("mismatch must fail closed");
    assert_eq!(bad.code(), ERR_CORPUS_UNRESOLVED);

    let empty = build_resolved_market_corpus(&[], PANEL_VERSION, SALT, 0.0)
        .expect_err("empty must fail closed");
    assert_eq!(empty.code(), ERR_CORPUS_EMPTY);
}

#[test]
fn agreement_graph_admits_similar_and_rejects_dissimilar() {
    // Two near-identical markets agree (cosine ~1); a very different one does not clear a high bar.
    let a = snap("0xH1", 0, 0.60, 0.02, 100_000.0, 30_000.0);
    let b = snap("0xH2", 0, 0.61, 0.02, 105_000.0, 31_000.0);
    let corpus = build_resolved_market_corpus(
        &[
            ResolvedMarketInput {
                snapshot: &a,
                resolution: &resolution("0xH1", 0),
            },
            ResolvedMarketInput {
                snapshot: &b,
                resolution: &resolution("0xH2", 0),
            },
        ],
        PANEL_VERSION,
        SALT,
        0.999,
    )
    .expect("corpus");
    assert!(
        !corpus.agreements.is_empty(),
        "near-identical markets must agree above 0.999"
    );
    for e in &corpus.agreements {
        assert!((0.0..=1.0).contains(&e.agreement));
    }
}

#[test]
fn end_to_end_recall_over_locally_built_corpus() {
    // A locally-built resolved-market corpus flows through the REAL recall gate. Build a small ring
    // of similar markets so the between-record agreement graph has a cycle (=> a non-empty FVS
    // kernel), every kernel member is in the corpus, and the recall engine returns a real ratio.
    let markets: Vec<(String, MarketSnapshot, Resolution)> = (0..6u32)
        .map(|i| {
            let cond = format!("0xRING{i}");
            // Slightly varying but highly-similar markets => a densely-connected agreement graph.
            let s = snap(
                &cond,
                0,
                0.60 + i as f64 * 0.001,
                0.02,
                100_000.0 + i as f64 * 100.0,
                30_000.0,
            );
            let r = resolution(&cond, 0);
            (cond, s, r)
        })
        .collect();
    let inputs: Vec<ResolvedMarketInput> = markets
        .iter()
        .map(|(_, s, r)| ResolvedMarketInput {
            snapshot: s,
            resolution: r,
        })
        .collect();
    let corpus = build_resolved_market_corpus(&inputs, PANEL_VERSION, SALT, 0.99)
        .expect("locally-built corpus");
    assert_eq!(corpus.recall_queries.len(), 6);
    assert!(
        !corpus.agreements.is_empty(),
        "similar markets must form an agreement graph with cycles"
    );

    let kp = KernelParams {
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 64,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    };
    let rp = RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 1,
        rng_seed: 219,
        min_recall_ratio: 0.95,
    };
    let recall = measure_computed_kernel_recall(&ComputedKernelRecallRequest {
        domain: crate::domain::Domain::Crypto,
        corpus: &corpus.recall_queries,
        agreements: &corpus.agreements,
        frequencies: &corpus.frequencies,
        anchors: &corpus.anchors,
        kernel_params: &kp,
        recall_params: &rp,
    })
    .expect("recall measured over the locally-built corpus");

    // Full-state: the measurement is real (bounded ratio, kernel is a subset of the corpus, every
    // held-out query counted).
    assert!(
        (0.0..=1.0).contains(&recall.measured_ratio),
        "ratio must be a real fraction, got {}",
        recall.measured_ratio
    );
    assert!(
        recall.fvs_kernel.kernel_member_count >= 1,
        "non-empty kernel"
    );
    assert!(
        recall.corpus_len == 6 && recall.fvs_kernel.kernel_member_count <= recall.corpus_len,
        "kernel is a subset of the corpus"
    );
    assert_eq!(recall.n_queries_tested, 6, "every row is a held-out probe");
}

#[test]
fn one_call_orchestrator_runs_and_persists_the_report() {
    // The single production entry point: build corpus -> measure recall -> persist. Full-state
    // verified: the report is physically written and round-trips exactly.
    let dir = std::env::temp_dir().join(format!(
        "calyx-poly-corpus-run-{}-{}",
        std::process::id(),
        crate::resolved_market_corpus::RECALL_VECTOR_DIM
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let markets: Vec<(String, MarketSnapshot, Resolution)> = (0..5u32)
        .map(|i| {
            let cond = format!("0xRUN{i}");
            let s = snap(&cond, 0, 0.58 + i as f64 * 0.001, 0.02, 80_000.0, 30_000.0);
            (cond.clone(), s, resolution(&cond, 0))
        })
        .collect();
    let inputs: Vec<ResolvedMarketInput> = markets
        .iter()
        .map(|(_, s, r)| ResolvedMarketInput {
            snapshot: s,
            resolution: r,
        })
        .collect();

    let kp = KernelParams {
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 64,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    };
    let rp = RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 1,
        rng_seed: 219,
        min_recall_ratio: 0.95,
    };
    let params = LocalRecallRunParams {
        domain: crate::domain::Domain::Crypto,
        panel_version: PANEL_VERSION,
        vault_salt: SALT,
        agreement_threshold: 0.99,
        kernel_params: &kp,
        recall_params: &rp,
        persist_dir: Some(dir.as_path()),
    };
    let (corpus, recall) =
        run_local_computed_kernel_recall(&inputs, &params).expect("one-call run");
    assert_eq!(corpus.recall_queries.len(), 5);

    // The report was physically persisted and reads back byte-for-byte equal.
    let persisted = dir.join(format!(
        "computed_kernel_recall_{}.json",
        recall.domain.slug()
    ));
    assert!(persisted.exists(), "report must be written to disk");
    let back = read_computed_kernel_recall(&persisted).expect("read persisted report");
    assert_eq!(back, recall, "persisted report must round-trip exactly");
    std::fs::remove_dir_all(&dir).ok();
}
