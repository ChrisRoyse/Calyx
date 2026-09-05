//! FSV for issue #216 — the empirical kernel-recall gate over the **computed** FVS kernel, wired
//! into the CalyxNative admission path.
//!
//! Full-state verification: a real association graph is turned into a genuinely computed FVS kernel
//! (`build_fvs_kernel`), the real Lodestar recall engine measures — over a real corpus of
//! `RecallQuery` rows — the fraction of held-out queries answerable through that computed kernel, and
//! the **measured** ratio (not a caller-supplied number) is wired into the six-tier superiority
//! predicate that gates admissibility at ≥ 0.95. Every artifact is persisted and read back.
//!
//! Synthetic known-truth (X+X=Y):
//! - Four disjoint 3-cycles → the FVS kernel is exactly 4 members (one per cycle).
//! - Happy: the 4-member kernel plus 8 non-members that *duplicate* a smaller-cx_id member's
//!   embedding → every held-out query's nearest neighbour is a member → recall = 1.0 ≥ 0.95 →
//!   admissible, with a kernel strictly smaller (4) than the corpus (12).
//! - Below floor: the same 4-member kernel over a corpus whose 16 extra rows occupy distinct
//!   neighbourhoods → only the 4 members self-recall → recall = 4/20 = 0.20 < 0.95 → the `kernel`
//!   tier fails and the forecast is produced but refused (fail loud, never silently upgraded).

use std::fs;

use calyx_assay::TrustTag;
use calyx_core::{CxId, FixedClock};
use calyx_lodestar::{KernelGraphParams, KernelParams, RecallQuery, RecallTestParams};
use calyx_mincut::{AgreementEdge, FrequencyEntry};
use calyx_poly::Domain;
use calyx_poly::calyx_native::{
    CalyxNativeForecast, CalyxNativeRequest, read_calyx_native_forecast,
};
use calyx_poly::forecast::{ComponentKind, ForecastComponent};
use calyx_poly::kernel_recall_admission::{
    ComputedKernelRecall, ComputedKernelRecallRequest, ERR_KERNEL_EMPTY_MEMBERS,
    ERR_KERNEL_MEMBER_NOT_IN_CORPUS, measure_computed_kernel_recall,
    produce_calyx_native_forecast_with_measured_kernel_recall, read_computed_kernel_recall,
    write_computed_kernel_recall,
};
use calyx_poly::superiority::SuperiorityTiers;
use serde_json::json;

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{named_fsv_root, reset_dir, write_json};

fn cx(i: u8) -> CxId {
    let mut b = [0u8; 16];
    b[15] = i;
    CxId::from_bytes(b)
}
fn edge(a: u8, b: u8) -> AgreementEdge {
    AgreementEdge {
        src: cx(a),
        dst: cx(b),
        agreement: 0.9,
        directional_confidence: 0.9,
    }
}
fn freqs(ids: &[u8]) -> Vec<FrequencyEntry> {
    ids.iter()
        .map(|i| FrequencyEntry {
            cx_id: cx(*i),
            frequency: 1.0,
        })
        .collect()
}
fn onehot(hot: usize, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; dim];
    v[hot % dim] = 1.0;
    v
}
fn kparams() -> KernelParams {
    KernelParams {
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 64,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    }
}
fn rparams() -> RecallTestParams {
    RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 1,
        rng_seed: 216,
        min_recall_ratio: 0.95,
    }
}

/// Four disjoint 3-cycles: {1,2,3},{4,5,6},{7,8,9},{10,11,12}. The FVS is exactly 4 (one per cycle).
fn four_cycle_graph() -> (Vec<AgreementEdge>, Vec<FrequencyEntry>, Vec<CxId>) {
    let agreements = vec![
        edge(1, 2),
        edge(2, 3),
        edge(3, 1),
        edge(4, 5),
        edge(5, 6),
        edge(6, 4),
        edge(7, 8),
        edge(8, 9),
        edge(9, 7),
        edge(10, 11),
        edge(11, 12),
        edge(12, 10),
    ];
    let frequencies = freqs(&(1..=12).collect::<Vec<_>>());
    let anchors = vec![cx(1), cx(4), cx(7), cx(10)];
    (agreements, frequencies, anchors)
}

fn strong_tiers() -> SuperiorityTiers {
    // Every tier passes except kernel-recall, which the measured value overwrites.
    SuperiorityTiers {
        oracle_self_consistency: 0.9,
        panel_sufficient: true,
        kernel_recall_ratio: 0.0, // placeholder — replaced by the measured ratio
        min_kernel_recall_ratio: 0.95,
        calibrated: true,
        goodhart_defended: true,
        mistake_closed: true,
    }
}

fn base_request(domain: &str) -> CalyxNativeRequest {
    CalyxNativeRequest {
        domain: domain.to_string(),
        condition_id: "0xcond216".into(),
        token_id: "tok".into(),
        horizon_bucket: "1h_24h".into(),
        components: vec![
            ForecastComponent::new(
                ComponentKind::KnnBaseRate,
                0.70,
                0.80,
                100,
                TrustTag::Trusted,
                "knn",
            )
            .unwrap(),
            ForecastComponent::new(
                ComponentKind::BitsVote,
                0.75,
                0.90,
                100,
                TrustTag::Trusted,
                "bits",
            )
            .unwrap(),
        ],
        calibration: None,
        raw_confidence: 0.95,
        oracle_flakiness: 0.05,
        oracle_validity: 0.98,
        // #92 derives panel sufficiency from measured bits before evaluating the kernel tier.
        // Keep this fixture sufficient so #216 isolates measured-kernel recall admission.
        panel_bits: 1.0,
        anchor_entropy_bits: 1.0,
        superiority_tiers: strong_tiers(),
        evidence: None,
    }
}

#[test]
fn issue216_computed_kernel_recall_admission_fsv() {
    let (root, keep) = named_fsv_root("POLY_ISSUE216_FSV_ROOT", "poly-issue216-recall");
    reset_dir(&root);
    let clock = FixedClock::new(1);
    let (agreements, frequencies, anchors) = four_cycle_graph();
    let kp = kparams();
    let rp = rparams();

    // ── Happy: strict-subset computed kernel (4) recalls a 12-row corpus at 1.0 → admissible. ──
    let members = [cx(1), cx(4), cx(7), cx(10)];
    let dim = 12;
    let mut happy_corpus: Vec<RecallQuery> = members
        .iter()
        .enumerate()
        .map(|(idx, m)| RecallQuery {
            cx_id: *m,
            vector: onehot(idx, dim),
        })
        .collect();
    for j in 0..8u8 {
        // cx(120+j) > every member cx; duplicates member (j % 4)'s embedding.
        happy_corpus.push(RecallQuery {
            cx_id: cx(120 + j),
            vector: onehot((j % 4) as usize, dim),
        });
    }
    let happy = measure_computed_kernel_recall(&ComputedKernelRecallRequest {
        domain: Domain::Crypto,
        corpus: &happy_corpus,
        agreements: &agreements,
        frequencies: &frequencies,
        anchors: &anchors,
        kernel_params: &kp,
        recall_params: &rp,
    })
    .expect("happy recall measured");

    assert_eq!(
        happy.fvs_kernel.kernel_member_count, 4,
        "four 3-cycles → 4 computed members"
    );
    assert!(
        happy.corpus_len > happy.fvs_kernel.kernel_member_count,
        "kernel must be a strict subset"
    );
    assert!(
        (happy.measured_ratio - 1.0).abs() < 1e-6,
        "computed kernel must recall 1.0, got {}",
        happy.measured_ratio
    );
    assert!(happy.gate_passed, "1.0 ≥ 0.95 must pass the gate");

    // Persist + read back the measurement (FSV source of truth).
    let happy_recall_path =
        write_computed_kernel_recall(&root.join("happy"), &happy).expect("write happy recall");
    let happy_recall_back =
        read_computed_kernel_recall(&happy_recall_path).expect("read happy recall");
    assert_eq!(
        happy_recall_back, happy,
        "computed kernel recall must round-trip exactly"
    );

    // Wire the measured ratio into admission → admissible.
    let happy_forecast = produce_calyx_native_forecast_with_measured_kernel_recall(
        base_request("crypto"),
        &happy,
        &clock,
    )
    .expect("happy forecast");
    let happy_forecast_path =
        calyx_poly::calyx_native::write_calyx_native_forecast(&root.join("happy"), &happy_forecast)
            .expect("write forecast");
    // The CalyxNative forecast's confidence-ceiling fields are f32-derived, so (as the #90 FSV test
    // documents) the on-disk record is verified by provenance hash + admissibility, not byte-exact
    // struct equality.
    let happy_forecast_back =
        read_calyx_native_forecast(&happy_forecast_path).expect("read forecast");
    assert_eq!(
        happy_forecast_back.provenance_hash,
        happy_forecast.provenance_hash
    );
    assert_eq!(happy_forecast_back.admissible, happy_forecast.admissible);
    assert_eq!(
        happy_forecast_back.superiority.pass,
        happy_forecast.superiority.pass
    );
    assert!(
        happy_forecast.admissible,
        "measured recall 1.0 must admit; refusal={}",
        happy_forecast.refusal_reason
    );
    assert!(happy_forecast.superiority.pass);

    // ── Below floor: same computed kernel over a 20-row corpus → recall 0.20 → refused. ────────
    let dimb = 20;
    let mut low_corpus: Vec<RecallQuery> = members
        .iter()
        .enumerate()
        .map(|(idx, m)| RecallQuery {
            cx_id: *m,
            vector: onehot(idx, dimb),
        })
        .collect();
    for j in 0..16u8 {
        low_corpus.push(RecallQuery {
            cx_id: cx(100 + j),
            vector: onehot((j as usize) + 4, dimb),
        });
    }
    let low = measure_computed_kernel_recall(&ComputedKernelRecallRequest {
        domain: Domain::Politics,
        corpus: &low_corpus,
        agreements: &agreements,
        frequencies: &frequencies,
        anchors: &anchors,
        kernel_params: &kp,
        recall_params: &rp,
    })
    .expect("low recall measured");
    assert!(
        (low.measured_ratio - 0.20).abs() < 1e-6,
        "expected 4/20=0.20, got {}",
        low.measured_ratio
    );
    assert!(!low.gate_passed, "0.20 < 0.95 must fail the gate");

    let low_recall_path =
        write_computed_kernel_recall(&root.join("below-floor"), &low).expect("write low recall");
    assert_eq!(
        read_computed_kernel_recall(&low_recall_path).expect("read"),
        low
    );

    let low_forecast = produce_calyx_native_forecast_with_measured_kernel_recall(
        base_request("politics"),
        &low,
        &clock,
    )
    .expect("below-floor forecast is produced (marked non-admissible), never errors");
    let low_forecast_path = calyx_poly::calyx_native::write_calyx_native_forecast(
        &root.join("below-floor"),
        &low_forecast,
    )
    .expect("write forecast");
    let low_forecast_back = read_calyx_native_forecast(&low_forecast_path).expect("read forecast");
    assert_eq!(
        low_forecast_back.provenance_hash,
        low_forecast.provenance_hash
    );
    assert_eq!(
        low_forecast_back.admissible, low_forecast.admissible,
        "on-disk record must prove refusal"
    );
    assert_eq!(
        low_forecast_back.refusal_reason,
        low_forecast.refusal_reason
    );
    assert!(!low_forecast.admissible, "measured recall 0.20 must refuse");
    assert!(
        low_forecast.refusal_reason.contains("kernel"),
        "refusal must name the failing kernel tier, got: {}",
        low_forecast.refusal_reason
    );
    assert!(
        low_forecast
            .superiority
            .failing_tiers
            .iter()
            .any(|t| t == "kernel"),
        "kernel must be the failing tier: {:?}",
        low_forecast.superiority.failing_tiers
    );

    // ── Edge: a computed kernel member missing from the corpus → hard error, fail loud. ────────
    // Drop the first member row from the happy corpus.
    let missing_corpus: Vec<RecallQuery> = happy_corpus
        .iter()
        .filter(|r| r.cx_id != members[0])
        .cloned()
        .collect();
    let missing_err = measure_computed_kernel_recall(&ComputedKernelRecallRequest {
        domain: Domain::Crypto,
        corpus: &missing_corpus,
        agreements: &agreements,
        frequencies: &frequencies,
        anchors: &anchors,
        kernel_params: &kp,
        recall_params: &rp,
    })
    .expect_err("missing member must fail closed");
    assert_eq!(missing_err.code(), ERR_KERNEL_MEMBER_NOT_IN_CORPUS);

    // ── Edge: a pure DAG has no cycles → empty computed kernel → hard error. ───────────────────
    let dag = vec![edge(1, 2), edge(2, 3), edge(3, 4)];
    let dag_freqs = freqs(&[1, 2, 3, 4]);
    let dag_corpus: Vec<RecallQuery> = [1u8, 2, 3, 4]
        .iter()
        .enumerate()
        .map(|(i, n)| RecallQuery {
            cx_id: cx(*n),
            vector: onehot(i, 8),
        })
        .collect();
    let empty_err = measure_computed_kernel_recall(&ComputedKernelRecallRequest {
        domain: Domain::Crypto,
        corpus: &dag_corpus,
        agreements: &dag,
        frequencies: &dag_freqs,
        anchors: &[cx(4)],
        kernel_params: &kp,
        recall_params: &rp,
    })
    .expect_err("empty kernel must fail closed");
    assert_eq!(empty_err.code(), ERR_KERNEL_EMPTY_MEMBERS);

    // ── Evidence log. ─────────────────────────────────────────────────────────────────────────
    let summary = json!({
        "issue": 216,
        "source_of_truth": [
            "computed_kernel_recall_*.json readback",
            "calyx_native_forecast_*.json readback",
            "association graph (computed FVS members)",
        ],
        "happy": recall_evidence(&happy, &happy_forecast),
        "below_floor": recall_evidence(&low, &low_forecast),
        "missing_member_error": missing_err.code(),
        "empty_kernel_error": empty_err.code(),
        "physical_files": physical_files(&root),
    });
    write_json(&root.join("summary.json"), &summary);
    println!(
        "issue216_fsv_summary={}",
        serde_json::to_string_pretty(&summary).unwrap()
    );
    if keep {
        println!("poly_issue216_fsv_root={}", root.display());
    }
}

fn recall_evidence(
    recall: &ComputedKernelRecall,
    forecast: &CalyxNativeForecast,
) -> serde_json::Value {
    json!({
        "computed_kernel_members": recall.fvs_kernel.kernel_member_count,
        "mfvs_size": recall.fvs_kernel.mfvs_size,
        "cycles_in_graph": recall.fvs_kernel.cycles_in_graph,
        "corpus_len": recall.corpus_len,
        "measured_ratio": recall.measured_ratio,
        "gate_passed": recall.gate_passed,
        "n_queries_tested": recall.n_queries_tested,
        "forecast_admissible": forecast.admissible,
        "forecast_refusal": forecast.refusal_reason,
        "superiority_failing_tiers": forecast.superiority.failing_tiers,
    })
}

fn physical_files(root: &std::path::Path) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let bytes = fs::read(&p).unwrap_or_default();
                    out.push(json!({
                        "path": p.display().to_string(),
                        "bytes": bytes.len(),
                        "blake3": blake3::hash(&bytes).to_hex().to_string(),
                    }));
                }
            }
        }
    }
    out.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    out
}
