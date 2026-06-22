use std::collections::BTreeMap;
use std::fs;

use calyx_assay::{
    AssayCacheKey, AssayGate, AssayStore, AssaySubject, DeficitRoutingContext, EstimatorKind,
    InMemoryDeficitSink, MIN_ASSAY_SAMPLES, StratumBits, TrustTag, admit_lens,
    admit_lens_with_strata, bits_report, bootstrap_mean_ci, entropy_bits, ksg_mi_continuous,
    logistic_probe_mi, panel_sufficiency, panel_sufficiency_with_context,
    partitioned_histogram_nmi, per_sensor_attribution, project_cpu, project_gpu, stable_rank,
    stratified_bits,
};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::AnchorKind;
use calyx_loom::{
    AbundanceReport, CALYX_LOOM_FORGE_UNAVAILABLE, CeilingEstimate, CrossTermKind, CrossTermValue,
    LoomStore, MaterializationAction, MaterializationPlan, NeffEstimate, Severity,
    StaticPairGainGate, agreement_batch_cpu, agreement_batch_gpu, agreement_scalar,
    detect_blind_spot, plan_cross_terms,
};
use serde_json::json;

mod stage5_helpers;
use stage5_helpers::*;
#[test]
fn loom_cross_terms_materialization_and_reports_work() {
    let a = vec![1.0, 0.0];
    let b = vec![0.5, 3.0_f32.sqrt() * 0.5];
    let agreement = agreement_scalar(&a, &b).unwrap();
    assert!((agreement - 0.5).abs() < 1.0e-4);
    let cpu = agreement_batch_cpu(&[(&a, &b)]).unwrap();
    let gpu = agreement_batch_gpu(&[(&a, &b)]).unwrap_err();
    assert_eq!(gpu.code, CALYX_LOOM_FORGE_UNAVAILABLE);
    assert!((cpu[0] - agreement).abs() <= 1.0e-6);

    let mut store = LoomStore::new(8);
    let slots = two_slot_map(a.clone(), b.clone());
    store.weave(cx(1), &slots).unwrap();
    assert_eq!(store.xterm_count(), 1);
    assert_eq!(store.cache_count(), 0);
    let lazy = store
        .cross_term(cx(1), slot(1), slot(2), CrossTermKind::Delta, &slots)
        .unwrap();
    assert_eq!(store.xterm_count(), 1);
    assert_eq!(store.cache_count(), 1);
    assert_eq!(
        lazy,
        CrossTermValue::Vector(vec![0.5, -3.0_f32.sqrt() * 0.5])
    );

    let lens_slots: Vec<_> = (0..13).map(slot).collect();
    let low_gain = StaticPairGainGate { gain_bits: 0.0 };
    let plan = plan_cross_terms(&lens_slots, &low_gain);
    assert_eq!(plan.materialized_count(), 78);
    assert_eq!(
        plan.entries
            .iter()
            .filter(|entry| entry.action == MaterializationAction::LazyCache)
            .count(),
        234
    );

    let high_gain = StaticPairGainGate { gain_bits: 0.08 };
    let high_gain_plan = plan_cross_terms(&lens_slots, &high_gain);
    assert_eq!(high_gain_plan.materialized_count(), 156);
    assert_eq!(
        plan_count(
            &high_gain_plan,
            CrossTermKind::Interaction,
            MaterializationAction::EagerStore
        ),
        78
    );
    assert_eq!(
        plan_count(
            &high_gain_plan,
            CrossTermKind::Delta,
            MaterializationAction::LazyCache
        ),
        78
    );
    assert_eq!(
        plan_count(
            &high_gain_plan,
            CrossTermKind::Concat,
            MaterializationAction::LazyCache
        ),
        78
    );

    let mut graph_store = LoomStore::new(16);
    for i in 0..50 {
        graph_store
            .weave(
                cx(i as u8),
                &two_slot_map(vec![1.0, 0.0], vec![0.75, 0.661_437_8]),
            )
            .unwrap();
    }
    let graph = graph_store.agreement_graph();
    assert_eq!(graph[0].n, 50);
    assert!((graph[0].mean_agreement - 0.75).abs() < 0.01);

    let alert = detect_blind_spot(cx(8), slot(1), slot(2), 0.95, 0.10).unwrap();
    assert_eq!(alert.severity, Severity::High);
    assert!((alert.delta - 0.85).abs() < 0.01);

    let mut abundance_store = LoomStore::new(16);
    let slots_13 = slot_map_13();
    for i in 0..50 {
        abundance_store.weave(cx(i as u8), &slots_13).unwrap();
    }
    let report = AbundanceReport::new(
        13,
        50,
        abundance_store.xterm_count(),
        NeffEstimate::Computed {
            value: 3.0,
            ci_low: 2.8,
            ci_high: 3.2,
        },
        CeilingEstimate::Computed { bits: 1.0 },
        abundance_store.measured_count(),
        abundance_store.xterm_count(),
    );
    assert_eq!(report.c_n2_upper_bound, 78);
    assert_eq!(report.materialized, 3_900);
    assert_eq!(report.measured_count, 650);
    assert_eq!(report.derived_count, 3_900);
    assert_eq!(report.meaning_compression_yield, 78.0);
    assert!(
        AbundanceReport::new(
            13,
            0,
            0,
            NeffEstimate::Provisional { value: 0.0 },
            CeilingEstimate::Provisional { bits: 0.0 },
            0,
            0,
        )
        .meaning_compression_yield
        .is_nan()
    );
}

#[test]
fn assay_estimators_contracts_sufficiency_and_store_work() {
    let (x, y) = correlated_samples(120);
    let estimate = ksg_mi_continuous(&x, &y, 3).unwrap();
    let known = gaussian_mi_bits(&x, &y);
    assert!(estimate.bits > 0.05);
    assert!(
        estimate.ci_low <= known && known <= estimate.ci_high,
        "known={known}, estimate={estimate:?}"
    );
    let short = ksg_mi_continuous(&x[..30], &y[..30], 3).unwrap_err();
    assert_eq!(short.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    let (mut ragged_x, ragged_y) = correlated_samples(MIN_ASSAY_SAMPLES);
    ragged_x[0].push(0.25);
    let ragged = ksg_mi_continuous(&ragged_x, &ragged_y, 3).unwrap_err();
    assert_eq!(ragged.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let projected = project_cpu(&high_dim_matrix(200, 1_536), 42);
    let gpu = project_gpu(&high_dim_matrix(200, 1_536), 42).unwrap_err();
    assert_eq!(projected.output_dim, 16);
    assert_eq!(gpu.code, "CALYX_FORGE_DEVICE_UNAVAILABLE");

    let redundant_x: Vec<f32> = (0..100).map(|i| (i % 10) as f32).collect();
    let redundant = partitioned_histogram_nmi(&redundant_x, &redundant_x, 10).unwrap();
    let independent_y: Vec<f32> = (0..100).map(|i| (i / 10) as f32).collect();
    let independent = partitioned_histogram_nmi(&redundant_x, &independent_y, 10).unwrap();
    assert!(redundant.nmi >= 0.8);
    assert!(independent.nmi <= 0.1);

    let (separable_samples, labels) = binary_samples(true);
    let separated = logistic_probe_mi(&separable_samples, &labels).unwrap();
    assert!(separated.estimate.bits > 0.95);
    assert_eq!(separated.selected_field, "logistic_probe");
    let (flat_samples, flat_labels) = binary_samples(false);
    let flat = logistic_probe_mi(&flat_samples, &flat_labels).unwrap();
    assert!(flat.estimate.bits <= 0.01);
    let (mut nonfinite_samples, nonfinite_labels) = binary_samples(true);
    nonfinite_samples[0][0] = f32::NAN;
    let nonfinite = logistic_probe_mi(&nonfinite_samples, &nonfinite_labels).unwrap_err();
    assert_eq!(nonfinite.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    assert_eq!(
        admit_lens(0.01, 0.1).unwrap_err().code,
        "CALYX_ASSAY_LOW_SIGNAL"
    );
    assert_eq!(
        admit_lens(0.2, 0.7).unwrap_err().code,
        "CALYX_ASSAY_REDUNDANT"
    );
    let strata = stratified_bits(
        0.01,
        vec![StratumBits {
            name: "rare-recurrence".to_string(),
            bits: 0.18,
            frequency: 0.02,
            sole_carrier: true,
        }],
    );
    let admitted = admit_lens_with_strata(&strata, 0.2).unwrap();
    assert!(admitted.stratified_override);
    assert!(strata.no_frequency_multiplier);

    let rank = stable_rank(&block_redundancy_matrix(9, 3));
    assert!((2.5..=4.0).contains(&rank.n_eff));

    let attributions = per_sensor_attribution(&[(slot(1), 0.04), (slot(2), 0.42)], 0.10);
    let bits = bits_report(attributions.clone(), TrustTag::Provisional);
    assert_eq!(bits.trust, TrustTag::Provisional);
    assert!(bits.slots[1].sole_carrier);
    let entropy = entropy_bits(&[false, true, false, true]);
    let sufficiency = panel_sufficiency(0.45, entropy, &attributions, TrustTag::Provisional);
    assert!(!sufficiency.sufficient);
    let mut sink = InMemoryDeficitSink::default();
    sufficiency.route_to(&mut sink);
    assert_eq!(sink.routed.len(), 2);
    assert_eq!(sink.routed[0].panel_id, "panel:unspecified");
    assert_eq!(sink.routed[0].computed_at_seq, 0);
    assert!(!sink.routed[0].per_slot_gaps.is_empty());

    let gate = AssayGate::default();
    let signal = gate.lens_signal(&separable_samples, &labels).unwrap();
    assert!(signal.estimate.bits > 0.95);
    let strict_gate = AssayGate {
        min_samples: MIN_ASSAY_SAMPLES + 1,
    };
    assert_eq!(
        strict_gate
            .lens_signal(&separable_samples, &labels)
            .unwrap_err()
            .code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    let pair_gain = gate
        .pair_gain(&separable_samples, &flat_samples, &labels)
        .unwrap();
    assert_eq!(pair_gain.n_samples, MIN_ASSAY_SAMPLES);
    let (left_pair, right_pair, pair_labels) = complementary_pair_samples();
    let planted_gain = gate
        .pair_gain(&left_pair, &right_pair, &pair_labels)
        .unwrap();
    assert!(planted_gain.gain_bits > 0.05);

    let mut store = AssayStore::default();
    let key = AssayCacheKey::scoped(5, "shard-a", assay_vault(), AnchorKind::Reward);
    let subject = AssaySubject::Lens { slot: slot(2) };
    store.put(
        key.clone(),
        subject.clone(),
        signal.estimate.clone(),
        "synthetic planted pass/fail anchor",
        7,
    );
    assert!(store.cache_hit(&key, &subject));
    assert_eq!(
        store.get(&key, &subject).unwrap().provenance,
        "synthetic planted pass/fail anchor"
    );
    assert_eq!(store.invalidate_panel(5), 1);
    assert!(store.is_empty());

    assert!(
        bootstrap_mean_ci(&[0.8, 1.0, 1.2, 1.0], 64, 9)
            .unwrap()
            .ci_low
            <= 1.0
    );
    assert_eq!(estimate.estimator, EstimatorKind::Ksg);
}

#[test]
#[ignore = "manual FSV writes Stage 5 source-of-truth readbacks"]
fn stage5_full_stack_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let cf_root = root.join("stage5-aster-cf");
    let _ = fs::remove_dir_all(&cf_root);
    let mut cf_router = CfRouter::open(&cf_root, 1_048_576).unwrap();
    let mut readback = BTreeMap::new();

    let mut loom = LoomStore::new(32);
    let slots = slot_map_13();
    for i in 0..50 {
        loom.weave(cx(i as u8), &slots).unwrap();
    }
    let lens_slots: Vec<_> = (0..13).map(slot).collect();
    let low_gain_plan = plan_cross_terms(&lens_slots, &StaticPairGainGate { gain_bits: 0.0 });
    let high_gain_plan = plan_cross_terms(&lens_slots, &StaticPairGainGate { gain_bits: 0.08 });
    let alert = detect_blind_spot(cx(8), slot(1), slot(2), 0.95, 0.10).unwrap();
    let lazy_before = loom.xterm_count();
    let lazy_value = loom
        .cross_term(cx(1), slot(1), slot(2), CrossTermKind::Delta, &slots)
        .unwrap();
    let xterm_persisted = loom.persist_xterms_to_aster(&mut cf_router).unwrap();
    let persisted_loom = LoomStore::load_xterms_from_aster(&cf_router, 32).unwrap();
    let xterm = json!({
        "cf_root": cf_root.join("cf/xterm").display().to_string(),
        "xterm_rows": persisted_loom.xterm_count(),
        "persisted_rows": xterm_persisted,
        "sst_files": cf_router.level_file_count(ColumnFamily::XTerm),
        "raw_cf_rows": cf_router.iter_cf(ColumnFamily::XTerm).unwrap().len(),
        "lazy_before_rows": lazy_before,
        "lazy_after_rows": persisted_loom.xterm_count(),
        "lazy_cache_rows": loom.cache_count(),
        "lazy_delta": lazy_value,
        "agreement_edges": persisted_loom.agreement_graph(),
        "measured_tags": loom.measured_count(),
        "low_gain_materialized": low_gain_plan.materialized_count(),
        "high_gain_materialized": high_gain_plan.materialized_count(),
        "high_gain_interaction_eager": plan_count(&high_gain_plan, CrossTermKind::Interaction, MaterializationAction::EagerStore),
        "high_gain_delta_lazy": plan_count(&high_gain_plan, CrossTermKind::Delta, MaterializationAction::LazyCache),
        "high_gain_concat_lazy": plan_count(&high_gain_plan, CrossTermKind::Concat, MaterializationAction::LazyCache),
        "low_gain_lazy": low_gain_plan.entries.iter().filter(|entry| entry.action == MaterializationAction::LazyCache).count(),
        "agreement_gpu": agreement_gpu_readback(),
        "blind_spot": alert,
    });
    fs::write(
        root.join("xterm-cf-readback.json"),
        serde_json::to_vec_pretty(&xterm).unwrap(),
    )
    .unwrap();
    readback.insert("xterm_cf", xterm);

    let (samples, labels) = binary_samples(true);
    let (left_pair, right_pair, pair_labels) = complementary_pair_samples();
    let gate = AssayGate::default();
    let signal = gate.lens_signal(&samples, &labels).unwrap();
    let strict_min_samples_error = AssayGate {
        min_samples: MIN_ASSAY_SAMPLES + 1,
    }
    .lens_signal(&samples, &labels)
    .unwrap_err();
    let pair_gain = gate
        .pair_gain(&left_pair, &right_pair, &pair_labels)
        .unwrap();
    let (ksg_x, ksg_y) = correlated_samples(120);
    let ksg = ksg_mi_continuous(&ksg_x, &ksg_y, 3).unwrap();
    let ksg_known = gaussian_mi_bits(&ksg_x, &ksg_y);
    let ksg_known_inside_ci = ksg.ci_low <= ksg_known && ksg_known <= ksg.ci_high;
    let ksg_short = ksg_mi_continuous(&ksg_x[..30], &ksg_y[..30], 3).unwrap_err();
    let (mut ragged_x, ragged_y) = correlated_samples(MIN_ASSAY_SAMPLES);
    ragged_x[0].push(0.25);
    let ksg_ragged = ksg_mi_continuous(&ragged_x, &ragged_y, 3).unwrap_err();
    let (mut nonfinite_samples, nonfinite_labels) = binary_samples(true);
    nonfinite_samples[0][0] = f32::INFINITY;
    let logistic_non_finite = logistic_probe_mi(&nonfinite_samples, &nonfinite_labels).unwrap_err();
    let matrix = high_dim_matrix(200, 1_536);
    let projected = project_cpu(&matrix, 42);
    let projected_gpu_error = project_gpu(&matrix, 42).unwrap_err();
    let redundant_x: Vec<f32> = (0..100).map(|i| (i % 10) as f32).collect();
    let independent_y: Vec<f32> = (0..100).map(|i| (i / 10) as f32).collect();
    let redundant_nmi = partitioned_histogram_nmi(&redundant_x, &redundant_x, 10).unwrap();
    let independent_nmi = partitioned_histogram_nmi(&redundant_x, &independent_y, 10).unwrap();
    let nmi_exact_quorum = partitioned_histogram_nmi(
        &redundant_x[..MIN_ASSAY_SAMPLES],
        &redundant_x[..MIN_ASSAY_SAMPLES],
        10,
    )
    .unwrap();
    let nmi_empty = partitioned_histogram_nmi(&[], &[], 10).unwrap_err();
    let nmi_short =
        partitioned_histogram_nmi(&redundant_x[..30], &redundant_x[..30], 10).unwrap_err();
    let mut nmi_nonfinite_x = redundant_x.clone();
    nmi_nonfinite_x[7] = f32::NAN;
    let nmi_nonfinite = partitioned_histogram_nmi(&nmi_nonfinite_x, &redundant_x, 10).unwrap_err();
    let strata = stratified_bits(
        0.01,
        vec![StratumBits {
            name: "rare-recurrence".to_string(),
            bits: 0.18,
            frequency: 0.02,
            sole_carrier: true,
        }],
    );
    let stratified_admission = admit_lens_with_strata(&strata, 0.2).unwrap();
    let rank = stable_rank(&block_redundancy_matrix(9, 3));
    let attributions = per_sensor_attribution(&[(slot(1), 0.04), (slot(2), 0.42)], 0.10);
    let bits = bits_report(attributions.clone(), TrustTag::Provisional);
    let sufficiency = panel_sufficiency_with_context(
        0.45,
        1.0,
        &attributions,
        TrustTag::Provisional,
        DeficitRoutingContext {
            panel_id: "stage5-panel-v1".to_string(),
            anchor: AnchorKind::Label("stage5-passfail".to_string()),
            computed_at_seq: 101,
            observation_scope: None,
        },
    );
    let mut sink = InMemoryDeficitSink::default();
    sufficiency.route_to(&mut sink);
    let mut assay_store = AssayStore::default();
    let key = AssayCacheKey::scoped(
        5,
        "stage5-synthetic",
        assay_vault(),
        AnchorKind::Label("stage5-passfail".to_string()),
    );
    assay_store.put(
        key.clone(),
        AssaySubject::Lens { slot: slot(2) },
        signal.estimate.clone(),
        "FSV planted binary anchor",
        100,
    );
    let assay_persisted = assay_store.persist_to_aster(&mut cf_router).unwrap();
    let loaded_assay = AssayStore::load_from_aster(&cf_router).unwrap();
    let assay = json!({
        "cf_root": cf_root.join("cf/assay").display().to_string(),
        "rows": loaded_assay.rows(),
        "persisted_rows": assay_persisted,
        "sst_files": cf_router.level_file_count(ColumnFamily::Assay),
        "raw_cf_rows": cf_router.iter_cf(ColumnFamily::Assay).unwrap().len(),
        "cache_hit": loaded_assay.cache_hit(&key, &AssaySubject::Lens { slot: slot(2) }),
        "all_rows_scoped": loaded_assay.rows().iter().all(|row| row.cache_key.vault_id.is_some()),
        "vault_scope": key.vault_id.as_ref().unwrap().to_string(),
        "anchor_scope": key.anchor.clone(),
        "logistic_bits": signal.estimate.bits,
        "strict_min_samples_error": strict_min_samples_error.code,
        "pair_gain": pair_gain,
        "ksg": {"estimate": ksg, "known_bits": ksg_known, "known_inside_ci": ksg_known_inside_ci},
        "insufficient_samples_error": ksg_short.code,
        "ragged_samples_error": ksg_ragged.code,
        "non_finite_samples_error": logistic_non_finite.code,
        "projection": {"rows": projected.input_rows, "input_dim": projected.input_dim, "output_dim": projected.output_dim, "gpu_error": projected_gpu_error.code},
        "nmi": {
            "redundant": redundant_nmi,
            "independent": independent_nmi,
            "exact_quorum_samples": nmi_exact_quorum.n_samples,
            "empty_error": nmi_empty.code,
            "short_error": nmi_short.code,
            "non_finite_error": nmi_nonfinite.code,
        },
        "stratified": {"bits": strata, "admission": stratified_admission},
        "n_eff": rank,
        "bits_report": bits,
        "sufficiency": sufficiency,
        "deficit_routing": sink.routed,
        "low_signal_error": admit_lens(0.01, 0.1).unwrap_err().code,
        "redundant_error": admit_lens(0.2, 0.7).unwrap_err().code,
        "non_finite_signal_error": admit_lens(f32::NAN, 0.1).unwrap_err().code,
        "non_finite_corr_error": admit_lens(0.2, f32::INFINITY).unwrap_err().code,
    });
    fs::write(
        root.join("assay-cf-readback.json"),
        serde_json::to_vec_pretty(&assay).unwrap(),
    )
    .unwrap();
    readback.insert("assay_cf", assay);

    let abundance = AbundanceReport::new(
        13,
        50,
        loom.xterm_count(),
        NeffEstimate::Computed {
            value: rank.n_eff,
            ci_low: 2.5,
            ci_high: 4.0,
        },
        CeilingEstimate::Computed { bits: 1.0 },
        loom.measured_count(),
        loom.xterm_count(),
    );
    let zero_abundance = AbundanceReport::new(
        13,
        0,
        0,
        NeffEstimate::Provisional { value: 0.0 },
        CeilingEstimate::Provisional { bits: 0.0 },
        0,
        0,
    );
    readback.insert(
        "abundance_report",
        json!({
            "report": abundance,
            "zero_constellation_yield_is_nan": zero_abundance.meaning_compression_yield.is_nan(),
        }),
    );
    let path = root.join("stage5-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("STAGE5_READBACK={}", path.display());
    println!("STAGE5_XTERM_ROWS={}", loom.xterm_count());
    println!("STAGE5_ASSAY_ROWS={}", loaded_assay.len());
}

fn plan_count(
    plan: &MaterializationPlan,
    kind: CrossTermKind,
    action: MaterializationAction,
) -> usize {
    plan.entries
        .iter()
        .filter(|entry| entry.kind == kind && entry.action == action)
        .count()
}
