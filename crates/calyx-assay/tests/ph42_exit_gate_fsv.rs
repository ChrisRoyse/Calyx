use std::fs;

use calyx_assay::{
    Domain as AssayDomain, default_outcome_anchor, oracle_self_consistency,
    outcome_occurrence_context,
};
use calyx_aster::cf::base_key;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{RetentionPolicy, append_occurrence};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorValue, CxId, VaultStore};
use calyx_lodestar::{
    FREQ_WEIGHT, apply_frequency_bonuses, frequency_kernel_bonus, kernel_weight_rows,
};
use calyx_ward::{Domain as WardDomain, surprise_bits};
use serde_json::{Value, json};

mod ph42_exit_gate_support;
use ph42_exit_gate_support::*;

#[test]
#[ignore = "manual FSV trigger for issue 393"]
fn issue393_ph42_exit_gate_fsv_writes_artifacts() {
    let (root, keep_root) = fsv_root();
    reset_dir(&root);
    println!("issue393_step=reset root={}", root.display());
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue393-ph42-exit-gate",
        VaultOptions::default(),
    )
    .expect("open durable vault");

    println!("issue393_step=before_raw_state");
    let before = raw_state(&vault);
    println!("issue393_step=assay");
    let assay = assay_exit_gate(&vault);
    println!("issue393_step=kernel");
    let kernel = kernel_weight_exit_gate(&vault);
    println!("issue393_step=surprise");
    let surprise = surprise_never_inflates_bits(&vault);
    println!("issue393_step=lead_lag");
    let lead_lag = temporal_lead_lag_directional(&vault);
    vault.flush().expect("flush final");
    println!("issue393_step=after_raw_state");
    let after = raw_state(&vault);

    println!("issue393_step=write_artifacts");
    write_json(root.join("assay-report.json"), &assay);
    write_json(root.join("kernel-weights.json"), &kernel);
    write_json(root.join("ward-novelty.json"), &surprise);
    write_json(root.join("temporal-cross-term.json"), &lead_lag);
    write_json(
        root.join("ph42-exit-gate.json"),
        &json!({
            "issue": 393,
            "commit_hint": "PH42 T07 exit gate",
            "source_of_truth": "durable Aster vault plus PH42 v1 artifacts",
            "vault_dir": vault_dir.display().to_string(),
            "before": before,
            "after": after,
            "artifacts": [
                "assay-report.json",
                "kernel-weights.json",
                "ward-novelty.json",
                "temporal-cross-term.json"
            ]
        }),
    );
    println!("issue393_step=blake3_manifest");
    write_blake3_sums(&root);
    println!("issue393_fsv_root={}", root.display());

    drop(vault);
    if !keep_root {
        fs::remove_dir_all(root).expect("remove temp fsv root");
    }
}

fn assay_exit_gate(vault: &AsterVault) -> Value {
    let agreeing = ids(1, 5);
    let flaky = ids(11, 5);
    let insufficient = ids(21, 5);
    for id in agreeing.iter().chain(&flaky).chain(&insufficient) {
        put_base(vault, *id, None);
    }
    for id in &agreeing {
        append_outcomes(vault, *id, &["stable", "stable", "stable", "stable"]);
    }
    for id in &flaky {
        append_outcomes(vault, *id, &["agree", "agree", "differ", "differ"]);
    }
    for id in &insufficient {
        append_outcomes(vault, *id, &["only-a", "only-b"]);
    }
    vault.flush().expect("flush assay");

    let agreeing_score = oracle_self_consistency(
        &AssayDomain::new("issue393-agreeing", agreeing.clone()),
        vault,
    )
    .expect("agreeing score");
    let flaky_score =
        oracle_self_consistency(&AssayDomain::new("issue393-flaky", flaky.clone()), vault)
            .expect("flaky score");
    let mixed_ids = agreeing.iter().chain(&flaky).copied().collect::<Vec<_>>();
    let mixed_score =
        oracle_self_consistency(&AssayDomain::new("issue393-mixed", mixed_ids), vault)
            .expect("mixed score");
    let insufficient_score = oracle_self_consistency(
        &AssayDomain::new("issue393-insufficient", insufficient.clone()),
        vault,
    )
    .expect("insufficient score");

    assert!(
        agreeing_score >= 0.90,
        "agreeing score was {agreeing_score}"
    );
    assert!(flaky_score <= 0.60, "flaky score was {flaky_score}");
    assert!(
        (0.55..=0.75).contains(&mixed_score),
        "mixed score was {mixed_score}"
    );
    assert_eq!(insufficient_score, 1.0);

    json!({
        "schema_version": 1,
        "surface": "assay-report",
        "artifact_kind": "ph42.assay-report.v1",
        "source_of_truth": "PH42 persisted artifact",
        "issue": 393,
        "trigger": "oracle_self_consistency(domain, vault)",
        "hand_computed_expected": {
            "agreeing_score": 1.0,
            "flaky_score": 2.0 / 6.0,
            "mixed_score": (5.0 + (5.0 * (2.0 / 6.0))) / 10.0,
            "insufficient_score": 1.0
        },
        "actual": {
            "agreeing_score": agreeing_score,
            "flaky_score": flaky_score,
            "mixed_score": mixed_score,
            "insufficient_score": insufficient_score
        },
        "source_of_truth_bytes": raw_state(vault)
    })
}

fn kernel_weight_exit_gate(vault: &AsterVault) -> Value {
    println!("issue393_step=kernel_setup");
    let high = cx(40);
    let low = cx(41);
    let zero_a = cx(42);
    let zero_b = cx(43);
    put_base(vault, high, None);
    put_base(vault, low, None);
    put_base(vault, zero_a, Some(0.0));
    put_base(vault, zero_b, Some(0.0));
    for idx in 0..50 {
        append_time(vault, high, 10_000 + idx);
    }
    append_time(vault, low, 10_005);
    vault.flush().expect("flush kernel");

    println!("issue393_step=kernel_apply");
    let mut graph = kernel_graph(&[high, low], &[(high, 0.80), (low, 0.80)]);
    let reads = apply_frequency_bonuses(&mut graph, vault).expect("apply frequency");
    let weights = kernel_weight_rows(&graph, &reads, 2);
    assert_eq!(weights[0].cx_id, high);
    assert_eq!(weights[0].frequency, 50);
    assert_eq!(weights[1].frequency, 1);
    assert!(weights[0].total_score > weights[1].total_score);

    println!("issue393_step=kernel_zero_edge");
    let mut zero_graph = kernel_graph(&[zero_a, zero_b], &[(zero_a, 0.80), (zero_b, 0.70)]);
    let zero_reads = apply_frequency_bonuses(&mut zero_graph, vault).expect("zero frequency");
    let zero_weights = kernel_weight_rows(&zero_graph, &zero_reads, 2);
    assert!(zero_weights.iter().all(|row| row.frequency_bonus == 0.0));
    assert_eq!(zero_weights.len(), 2);

    println!("issue393_step=kernel_raw_state");
    json!({
        "schema_version": 1,
        "surface": "kernel-weights",
        "artifact_kind": "ph42.kernel-weights.v1",
        "source_of_truth": "PH42 persisted artifact",
        "issue": 393,
        "trigger": "apply_frequency_bonuses(equal_betweenness_kernel_graph, vault)",
        "hand_computed_expected": {
            "high_frequency": 50,
            "low_frequency": 1,
            "base_betweenness": 0.80,
            "freq_weight": FREQ_WEIGHT,
            "high_total": 0.80 + FREQ_WEIGHT * f64::from(frequency_kernel_bonus(50)),
            "low_total": 0.80 + FREQ_WEIGHT * f64::from(frequency_kernel_bonus(1)),
            "expected_rank_1": high.to_string()
        },
        "weights": weights,
        "frequency_reads": reads,
        "edge_all_zero_frequency": {
            "reads": zero_reads,
            "weights": zero_weights
        },
        "source_of_truth_bytes": raw_state(vault)
    })
}

fn surprise_never_inflates_bits(vault: &AsterVault) -> Value {
    let singleton = cx(60);
    let common = cx(61);
    put_base(vault, singleton, Some(1.0));
    put_base(vault, common, None);
    for idx in 0..99 {
        append_time(vault, common, 20_000 + idx);
    }
    vault.flush().expect("flush surprise");

    let domain = WardDomain::new("issue393-surprise", vec![singleton, common]);
    let score = surprise_bits(singleton, &domain, vault).expect("surprise");
    let expected = -(1.0_f32 / 100.0).ln() / 2.0_f32.ln();
    assert!((score.get() - expected).abs() < 1e-5);
    let base_bytes = base_bytes(vault, singleton);
    let f32_be = score.get().to_be_bytes();
    let f64_be = f64::from(score.get()).to_be_bytes();
    let base = vault
        .get(singleton, vault.snapshot())
        .expect("singleton base");
    assert!(!contains_subslice(&base_bytes, &f32_be));
    assert!(!contains_subslice(&base_bytes, &f64_be));
    assert!(
        base.scalars
            .values()
            .all(|value| { (*value - f64::from(score.get())).abs() > 0.01 })
    );

    json!({
        "schema_version": 1,
        "surface": "ward-novelty",
        "artifact_kind": "ph42.ward-novelty.v1",
        "source_of_truth": "PH42 persisted artifact",
        "issue": 393,
        "trigger": "surprise_bits(singleton, domain, vault)",
        "hand_computed_expected": {
            "domain_total_events": 100,
            "singleton_frequency": 1,
            "surprise_bits": expected
        },
        "actual": {
            "singleton_surprise_bits": score.get(),
            "surprise_f32_be_hex": hex(&f32_be),
            "surprise_f64_be_hex": hex(&f64_be),
            "singleton_base_contains_surprise_f32": false,
            "singleton_base_contains_surprise_f64": false
        },
        "source_of_truth_bytes": {
            "singleton_base_key_hex": hex(&base_key(singleton)),
            "singleton_base_blake3": blake3::hash(&base_bytes).to_hex().to_string(),
            "singleton_base_len": base_bytes.len(),
            "singleton_base_hex": hex(&base_bytes),
            "all_cf": raw_state(vault)
        }
    })
}

fn temporal_lead_lag_directional(vault: &AsterVault) -> Value {
    let a = cx(70);
    let b = cx(71);
    put_base(vault, a, None);
    put_base(vault, b, None);
    append_times(vault, a, &[100, 200, 300, 400, 500]);
    append_times(vault, b, &[115, 215, 315, 415, 515]);
    vault.flush().expect("flush lead lag setup");

    let forward = calyx_loom::temporal_cross_term(a, b, vault, 30)
        .expect("forward")
        .expect("forward result");
    let reverse = calyx_loom::temporal_cross_term(b, a, vault, 30)
        .expect("reverse")
        .expect("reverse result");
    vault.flush().expect("flush lead lag");
    assert_eq!(forward.lead_lag_secs, 15.0);
    assert_eq!(reverse.lead_lag_secs, -15.0);
    let forward_bytes = vault
        .read_temporal_xterm(vault.latest_seq(), a, b)
        .expect("read forward")
        .expect("forward row");
    let reverse_bytes = vault
        .read_temporal_xterm(vault.latest_seq(), b, a)
        .expect("read reverse")
        .expect("reverse row");
    assert_eq!(
        calyx_loom::decode_lead_lag_result(&forward_bytes).expect("decode forward"),
        forward
    );
    assert_eq!(
        calyx_loom::decode_lead_lag_result(&reverse_bytes).expect("decode reverse"),
        reverse
    );

    json!({
        "schema_version": 1,
        "surface": "temporal-cross-term",
        "artifact_kind": "ph42.temporal-cross-term.v1",
        "source_of_truth": "PH42 persisted artifact",
        "issue": 393,
        "trigger": "temporal_cross_term(A,B,30) and temporal_cross_term(B,A,30)",
        "hand_computed_expected": {
            "a_times": [100, 200, 300, 400, 500],
            "b_times": [115, 215, 315, 415, 515],
            "forward_delta_secs": 15.0,
            "reverse_delta_secs": -15.0
        },
        "forward": forward,
        "reverse": reverse,
        "source_of_truth_bytes": {
            "forward_key_hex": temporal_key_hex(a, b),
            "forward_value_hex": hex(&forward_bytes),
            "reverse_key_hex": temporal_key_hex(b, a),
            "reverse_value_hex": hex(&reverse_bytes),
            "all_cf": raw_state(vault)
        }
    })
}

fn append_outcomes(vault: &AsterVault, cx_id: CxId, outcomes: &[&str]) {
    for (idx, outcome) in outcomes.iter().enumerate() {
        let context = outcome_occurrence_context(
            default_outcome_anchor(),
            AnchorValue::Text((*outcome).to_string()),
        )
        .expect("outcome context");
        let time = EpochSecs(1_000 + idx as i64);
        append_occurrence(
            vault,
            cx_id,
            time,
            context,
            time,
            RetentionPolicy::default(),
        )
        .expect("append outcome");
    }
}
