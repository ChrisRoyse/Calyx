//! EPIC #210 "Compose" — Calyx-native forecast producer end-to-end, Full State Verification.
//!
//! Source of truth: the persisted `calyx_native_forecast_*.json` on disk, read back separately. One
//! synthetic scenario with a *known* structure drives the whole pipeline: real kNN-of-resolved base
//! rate (#81) + real per-slot bits vote (#84) → reliability-weighted blend (#85) → calibration
//! de-bias (#86) → confidence ceiling (#87) → six-tier superiority (#88) → persisted forecast (#90),
//! plus the computed FVS kernel (#82) and mispricing flag (#89) as sibling artifacts.

use std::path::Path;

use calyx_assay::TrustTag;
use calyx_core::{CxId, FixedClock};
use calyx_lodestar::KernelParams;
use calyx_mincut::{AgreementEdge, FrequencyEntry};
use calyx_poly::bits_vote::{SlotVoteInput, bits_vote};
use calyx_poly::calyx_native::{
    CalyxNativeRequest, produce_calyx_native_forecast, read_calyx_native_forecast,
    write_calyx_native_forecast,
};
use calyx_poly::forecast::{ComponentKind, ForecastComponent};
use calyx_poly::forecast_calibration::{fit_calibration_slope, horizon_bucket};
use calyx_poly::kernel_forecast::build_fvs_kernel;
use calyx_poly::knn_base_rate::{ResolvedExemplar, knn_base_rate};
use calyx_poly::mispricing::detect_mispricing;
use calyx_poly::superiority::SuperiorityTiers;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::json;

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{named_fsv_root, reset_dir, write_blake3sums, write_json};

fn gaussian(rng: &mut ChaCha8Rng) -> f32 {
    use std::f64::consts::PI;
    let u1 = rng.random::<f64>().max(1e-12);
    let u2 = rng.random::<f64>();
    ((-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()) as f32
}

fn cx(i: u32) -> CxId {
    let mut b = [0u8; 16];
    b[12..16].copy_from_slice(&i.to_be_bytes());
    CxId::from_bytes(b)
}

/// Builds a resolved history where an "up" market (feature ~ +2) resolves YES ~90% and a "down"
/// market (~ −2) resolves NO ~90%. Returns kNN exemplars, aligned bits-vote slots + labels, and
/// miscalibrated (p_raw, outcome) calibration pairs.
struct Scenario {
    exemplars: Vec<ResolvedExemplar>,
    signal: Vec<f32>,
    noise: Vec<f32>,
    labels: Vec<bool>,
    calibration_pairs: Vec<(f64, bool)>,
}

fn scenario(seed: u64, n: usize) -> Scenario {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut exemplars = Vec::new();
    let mut signal = Vec::new();
    let mut noise = Vec::new();
    let mut labels = Vec::new();
    let mut calibration_pairs = Vec::new();
    for i in 0..n {
        let up = i % 2 == 0;
        // 90% of "up" resolve YES, 90% of "down" resolve NO.
        let flip = (i / 2) % 10 == 0;
        let outcome = if up { !flip } else { flip };
        let feat = if up { 2.0 } else { -2.0 } + 0.4 * gaussian(&mut rng);
        exemplars.push(ResolvedExemplar {
            cx_id: cx(i as u32 + 1),
            vector: vec![feat, 0.3 * gaussian(&mut rng)],
            outcome_yes: outcome,
        });
        signal.push(feat);
        noise.push(gaussian(&mut rng));
        labels.push(outcome);
        // A separate calibration history from an under-confident forecaster whose stated
        // probabilities span a realistic range but whose true rates are more extreme (so the fitted
        // slope stretches probabilities outward, b > 1, without wild extrapolation).
        let bucket = i % 4;
        let p_raw = [0.30, 0.45, 0.55, 0.70][bucket];
        let true_rate = [0.15, 0.40, 0.60, 0.85][bucket];
        let cal_outcome = ((i / 4) % 20) as f64 / 20.0 < true_rate;
        calibration_pairs.push((p_raw, cal_outcome));
    }
    Scenario {
        exemplars,
        signal,
        noise,
        labels,
        calibration_pairs,
    }
}

fn strong_tiers() -> SuperiorityTiers {
    SuperiorityTiers {
        oracle_self_consistency: 0.9,
        panel_sufficient: true,
        kernel_recall_ratio: 0.97,
        min_kernel_recall_ratio: 0.95,
        calibrated: true,
        goodhart_defended: true,
        mistake_closed: true,
    }
}

#[test]
fn issue_calyx_native_forecast_fsv() {
    let (root, _keep) = named_fsv_root("POLY_CALYX_NATIVE_FSV_ROOT", "poly-calyx-native-forecast");
    reset_dir(&root);
    let clock = FixedClock::new(1_785_400_000);

    let admitted = happy_admissible_forecast(&root, &clock);
    edge_failing_tier_refuses(&root, &clock, &admitted);
    edge_provisional_evidence_refuses(&root, &clock);
    computed_fvs_kernel_artifact(&root);
    mispricing_artifact(&root);

    write_blake3sums(&root);
}

/// Happy path: a live "up" market. kNN neighbors resolve YES, the bits vote points YES, the blend is
/// high, calibration stretches it, the ceiling holds it < 1, all six tiers pass → admissible. Persist
/// and read back.
fn happy_admissible_forecast(root: &Path, clock: &FixedClock) -> f64 {
    let sc = scenario(21_085, 160);

    // #81 kNN base rate: a live up-market query near the YES cluster.
    let knn = knn_base_rate(&sc.exemplars, &[2.2, 0.0], 15).expect("knn base rate");
    // #84 bits vote: informative signal slot + noise slot; the query is on the YES side of signal.
    let slots = vec![
        SlotVoteInput {
            key: "signal".into(),
            train_values: sc.signal.clone(),
            query_value: 2.2,
        },
        SlotVoteInput {
            key: "noise".into(),
            train_values: sc.noise.clone(),
            query_value: 0.0,
        },
    ];
    let bv = bits_vote(&slots, &sc.labels, 1.0, 3).expect("bits vote");
    // #86 calibration slope on the under-confident raw history.
    let slope = fit_calibration_slope("crypto", horizon_bucket(43_200.0), &sc.calibration_pairs)
        .expect("calibration");

    eprintln!(
        "[calyx-native] knn.p_yes={:.3} knn.reliab={:.3} | bits.p_yes={:.3} bits.reliab={:.3} bits.total_bits={:.3} | slope.b={:.3}",
        knn.p_yes, knn.reliability, bv.p_yes, bv.reliability, bv.total_bits, slope.b
    );
    assert!(
        knn.p_yes > 0.65,
        "up query → majority-YES kNN base rate, got {}",
        knn.p_yes
    );
    assert!(
        bv.p_yes > 0.65,
        "up query → YES bits vote (shrunk by information), got {}",
        bv.p_yes
    );

    let components = vec![
        ForecastComponent::new(
            ComponentKind::KnnBaseRate,
            knn.p_yes,
            knn.reliability,
            knn.k,
            TrustTag::Trusted,
            "knn",
        )
        .unwrap(),
        ForecastComponent::new(
            ComponentKind::BitsVote,
            bv.p_yes,
            bv.reliability,
            bv.n_train,
            TrustTag::Trusted,
            "bits",
        )
        .unwrap(),
    ];
    let req = CalyxNativeRequest {
        domain: "crypto".into(),
        condition_id: "0xup".into(),
        token_id: "tokYES".into(),
        horizon_bucket: horizon_bucket(43_200.0).to_string(),
        components,
        calibration: Some(slope),
        raw_confidence: 0.95,
        oracle_flakiness: 0.05,
        oracle_validity: 0.98,
        // #92: the CalyxNative producer now derives the honesty gate from measured panel bits.
        // This happy-path fixture must provide a sufficient panel, while `bv.total_bits` remains
        // the per-slot vote component's reliability evidence.
        panel_bits: 1.0,
        anchor_entropy_bits: 1.0,
        superiority_tiers: strong_tiers(),
        evidence: None,
    };
    let forecast = produce_calyx_native_forecast(&req, clock).expect("produce");

    eprintln!(
        "[calyx-native] p_raw={:.4} p_model={:.4} confidence={:.4} binding={} admissible={} trust={:?}",
        forecast.p_raw,
        forecast.p_model,
        forecast.confidence,
        forecast.confidence_ceiling.binding,
        forecast.admissible,
        forecast.trust
    );
    assert_eq!(
        forecast.source, "calyx_native",
        "must be tagged CalyxNative"
    );
    assert!(
        forecast.p_model > 0.7,
        "blended YES forecast, got {}",
        forecast.p_model
    );
    assert!(
        forecast.p_model > forecast.p_raw,
        "calibration must stretch the under-confident raw up"
    );
    assert!(forecast.confidence < 1.0, "confidence never reaches 1");
    assert!(
        forecast.admissible,
        "strong forecast admissible: {}",
        forecast.refusal_reason
    );
    assert_eq!(forecast.trust, TrustTag::Trusted);

    let path = write_calyx_native_forecast(root, &forecast).expect("write");
    let readback = read_calyx_native_forecast(&path).expect("read");
    // Identity: the provenance hash is computed from the in-memory values and stored verbatim, so a
    // matching readback hash proves the on-disk record is exactly the intended forecast. (Exact
    // struct equality is avoided because f32→f64-promoted fields can differ by one ULP across a JSON
    // round-trip; the numeric fields are checked within a tight tolerance instead.)
    assert_eq!(
        readback.provenance_hash, forecast.provenance_hash,
        "on-disk provenance must match"
    );
    assert_eq!(readback.source, forecast.source);
    assert_eq!(readback.admissible, forecast.admissible);
    assert_eq!(readback.trust, forecast.trust);
    assert!(
        (readback.p_model - forecast.p_model).abs() < 1e-9,
        "p_model round-trip"
    );
    assert!(
        (readback.confidence - forecast.confidence).abs() < 1e-9,
        "confidence round-trip"
    );
    assert_eq!(readback.components.len(), forecast.components.len());
    assert!(path.exists());

    write_json(
        &root.join("happy_summary.json"),
        &json!({
            "artifact_path": path.display().to_string(),
            "knn_p_yes": knn.p_yes,
            "bits_p_yes": bv.p_yes,
            "p_raw": forecast.p_raw,
            "p_model": forecast.p_model,
            "confidence": forecast.confidence,
            "ceiling_binding": forecast.confidence_ceiling.binding,
            "admissible": forecast.admissible,
            "superiority_pass": forecast.superiority.pass,
            "provenance_hash": forecast.provenance_hash,
        }),
    );
    forecast.p_model
}

/// Edge: the same strong forecast but the kernel-recall tier fails → non-admissible, and the on-disk
/// record proves the refusal.
fn edge_failing_tier_refuses(root: &Path, clock: &FixedClock, _admitted_p: &f64) {
    let sc = scenario(21_086, 160);
    let knn = knn_base_rate(&sc.exemplars, &[2.2, 0.0], 15).unwrap();
    let components = vec![
        ForecastComponent::new(
            ComponentKind::KnnBaseRate,
            knn.p_yes,
            knn.reliability,
            knn.k,
            TrustTag::Trusted,
            "knn",
        )
        .unwrap(),
    ];
    let mut tiers = strong_tiers();
    tiers.kernel_recall_ratio = 0.50; // below the 0.95 floor
    let req = CalyxNativeRequest {
        domain: "crypto".into(),
        condition_id: "0xnokernel".into(),
        token_id: "tok".into(),
        horizon_bucket: "1h_24h".into(),
        components,
        calibration: None,
        raw_confidence: 0.95,
        oracle_flakiness: 0.05,
        oracle_validity: 0.98,
        panel_bits: 1.0,
        anchor_entropy_bits: 1.0,
        superiority_tiers: tiers,
        evidence: None,
    };
    let forecast = produce_calyx_native_forecast(&req, clock).unwrap();
    assert!(!forecast.admissible);
    assert!(forecast.refusal_reason.contains("kernel"));
    let path = write_calyx_native_forecast(root, &forecast).unwrap();
    assert!(
        !read_calyx_native_forecast(&path).unwrap().admissible,
        "on-disk record proves refusal"
    );
}

/// Edge: proxy-grounded (Provisional) component → refused even when all tiers pass.
fn edge_provisional_evidence_refuses(root: &Path, clock: &FixedClock) {
    let comp = ForecastComponent::new(
        ComponentKind::KnnBaseRate,
        0.8,
        0.7,
        20,
        TrustTag::Provisional,
        "proxy",
    )
    .unwrap();
    let req = CalyxNativeRequest {
        domain: "crypto".into(),
        condition_id: "0xproxy".into(),
        token_id: "tok".into(),
        horizon_bucket: "1h_24h".into(),
        components: vec![comp],
        calibration: None,
        raw_confidence: 0.95,
        oracle_flakiness: 0.05,
        oracle_validity: 0.98,
        panel_bits: 1.0,
        anchor_entropy_bits: 1.0,
        superiority_tiers: strong_tiers(),
        evidence: None,
    };
    let forecast = produce_calyx_native_forecast(&req, clock).unwrap();
    assert_eq!(forecast.trust, TrustTag::Provisional);
    assert!(!forecast.admissible && forecast.refusal_reason.contains("provisional"));
    let path = write_calyx_native_forecast(root, &forecast).unwrap();
    assert_eq!(
        read_calyx_native_forecast(&path).unwrap().trust,
        TrustTag::Provisional
    );
}

/// #82: the computed minimum feedback vertex set over a synthetic association graph, persisted.
fn computed_fvs_kernel_artifact(root: &Path) {
    fn edge(a: u32, b: u32) -> AgreementEdge {
        AgreementEdge {
            src: cx(a),
            dst: cx(b),
            agreement: 0.9,
            directional_confidence: 0.9,
        }
    }
    // Two disjoint 3-cycles → minimum FVS is 2.
    let agreements = vec![
        edge(1, 2),
        edge(2, 3),
        edge(3, 1),
        edge(4, 5),
        edge(5, 6),
        edge(6, 4),
    ];
    let freqs: Vec<FrequencyEntry> = (1..=6)
        .map(|i| FrequencyEntry {
            cx_id: cx(i),
            frequency: 1.0,
        })
        .collect();
    let kernel = build_fvs_kernel(&agreements, &freqs, &[cx(1)], &KernelParams::default()).unwrap();
    assert_eq!(kernel.cycles_in_graph, 2);
    assert_eq!(
        kernel.mfvs_size, 2,
        "two disjoint cycles need a 2-node feedback vertex set"
    );
    write_json(
        &root.join("fvs_kernel.json"),
        &json!({
            "graph_nodes": kernel.graph_nodes,
            "cycles_in_graph": kernel.cycles_in_graph,
            "mfvs_size": kernel.mfvs_size,
            "mfvs_status": kernel.mfvs_status,
            "estimator_provenance": kernel.estimator_provenance,
        }),
    );
}

/// #89: mispricing flag from the kNN neighbor consensus vs a divergent market price.
fn mispricing_artifact(root: &Path) {
    let sc = scenario(21_089, 160);
    // A live "down" market (neighbors resolve NO) but priced high → overpriced.
    let knn = knn_base_rate(&sc.exemplars, &[-2.2, 0.0], 15).unwrap();
    let flag = detect_mispricing(&knn, 0.80, 0.15).unwrap();
    assert!(
        flag.flagged && flag.direction == "overpriced",
        "flag={flag:?}"
    );
    write_json(
        &root.join("mispricing.json"),
        &json!({
            "market_price": flag.market_price,
            "neighbor_consensus": flag.neighbor_consensus,
            "divergence": flag.divergence,
            "flagged": flag.flagged,
            "direction": flag.direction,
        }),
    );
}
