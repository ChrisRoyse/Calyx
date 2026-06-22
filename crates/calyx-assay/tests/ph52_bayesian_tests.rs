use std::fs;
use std::path::Path;

use calyx_assay::{
    BetaBernoulli, CALYX_BAYES_INVALID_INTERVAL, GammaPoisson, bayesian_posterior_for_domain,
    bayesian_posterior_key, beta_bernoulli_for_domain, gamma_poisson_for_domain,
    persist_bayesian_posterior,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, VaultId};
use proptest::prelude::*;
use serde_json::json;

mod ph52_support;

use ph52_support::write_readback;

const READBACK_LABEL: &str = "PH52_BAYES_READBACK";

#[test]
fn gamma_poisson_and_beta_bernoulli_match_planted_small_sample() {
    let mut rate = GammaPoisson::default();
    rate.update(10, 5.0).unwrap();
    let rate_ci = rate.credible_interval_95().unwrap();

    let mut consistency = BetaBernoulli::default();
    consistency.update(9, 1).unwrap();
    let consistency_ci = consistency.credible_interval_95().unwrap();
    let p_ge_07 = consistency.reliability_probability(0.7).unwrap();
    let reliable_at_087 = consistency.is_reliable(0.7, 0.87).unwrap();
    let reliable_at_090 = consistency.is_reliable(0.7, 0.90).unwrap();

    println!(
        "mean_rate={:.6} credible_interval=({:.6},{:.6}) next_occurrence={:.6}",
        rate.mean_rate(),
        rate_ci.0,
        rate_ci.1,
        rate.next_occurrence_expected()
    );
    println!(
        "mean_consistency={:.6} credible_interval=({:.6},{:.6}) p_ge_0_7={:.6} reliable_087={} reliable_090={}",
        consistency.mean_consistency(),
        consistency_ci.0,
        consistency_ci.1,
        p_ge_07,
        reliable_at_087,
        reliable_at_090
    );
    write_readback(
        READBACK_LABEL,
        "ph52-bayes-planted.json",
        json!({
            "gamma_poisson": {
                "posterior": rate,
                "mean_rate": rate.mean_rate(),
                "credible_interval_95": rate_ci,
                "next_occurrence_expected": rate.next_occurrence_expected(),
                "true_rate": 2.0,
            },
            "beta_bernoulli": {
                "posterior": consistency,
                "mean_consistency": consistency.mean_consistency(),
                "credible_interval_95": consistency_ci,
                "probability_p_ge_0_7": p_ge_07,
                "reliable_threshold_0_7_confidence_0_87": reliable_at_087,
                "reliable_threshold_0_7_confidence_0_90": reliable_at_090,
            }
        }),
    );

    assert!((rate.mean_rate() - 11.0 / 6.0).abs() < 0.1);
    assert!(rate_ci.0 <= 2.0 && 2.0 <= rate_ci.1);
    assert!((consistency.mean_consistency() - 10.0 / 12.0).abs() < 0.05);
    assert!(0.55 <= consistency_ci.0 && consistency_ci.0 <= 0.7);
    assert!(0.94 <= consistency_ci.1 && consistency_ci.1 <= 0.99);
    assert!(p_ge_07 > 0.87 && p_ge_07 < 0.90);
    assert!(reliable_at_087);
    assert!(!reliable_at_090);
}

#[test]
fn bayesian_posterior_roundtrips_through_aster_assay_cf() {
    let (vault, vault_dir) = vault();
    let domain = domain();
    let before = vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Assay,
            &bayesian_posterior_key(&domain).unwrap(),
        )
        .unwrap();
    let mut rate = GammaPoisson::default();
    rate.update(10, 5.0).unwrap();
    let mut consistency = BetaBernoulli::default();
    consistency.update(9, 1).unwrap();

    let seq = persist_bayesian_posterior(&vault, &domain, rate, consistency).unwrap();
    let after = vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Assay,
            &bayesian_posterior_key(&domain).unwrap(),
        )
        .unwrap()
        .expect("bayesian posterior row");
    let loaded = bayesian_posterior_for_domain(&vault, &domain)
        .unwrap()
        .expect("loaded posterior");
    let default_missing = gamma_poisson_for_domain(&vault, &missing_domain()).unwrap();
    let loaded_beta = beta_bernoulli_for_domain(&vault, &domain).unwrap();

    write_readback(
        READBACK_LABEL,
        "ph52-bayes-aster-row.json",
        json!({
            "source_of_truth": "AsterVault Assay CF row",
            "vault_dir": vault_dir,
            "key_hex": hex(&bayesian_posterior_key(&domain).unwrap()),
            "before_row_present": before.is_some(),
            "after_row_len": after.len(),
            "after_row_utf8": String::from_utf8(after.clone()).unwrap(),
            "seq": seq,
            "loaded": loaded,
            "missing_domain_default": default_missing,
            "loaded_beta": loaded_beta,
        }),
    );

    assert!(before.is_none());
    assert_eq!(seq, 1);
    assert_eq!(loaded.gamma_poisson, rate);
    assert_eq!(loaded.beta_bernoulli, consistency);
    assert_eq!(default_missing, GammaPoisson::default());
    assert_eq!(loaded_beta, consistency);
}

#[test]
fn bayesian_edges_fail_closed_with_codes() {
    let default_rate = GammaPoisson::default();
    let mut one_event = GammaPoisson::default();
    one_event.update(1, 1.0).unwrap();
    let one_event_ci = one_event.credible_interval_95().unwrap();
    let default_consistency = BetaBernoulli::default();
    let serialized = serde_json::to_vec(&default_consistency).unwrap();
    let roundtrip: BetaBernoulli = serde_json::from_slice(&serialized).unwrap();

    write_readback(
        READBACK_LABEL,
        "ph52-bayes-edges.json",
        json!({
            "default_rate": {
                "posterior": default_rate,
                "mean_rate": default_rate.mean_rate(),
                "next_occurrence_expected": default_rate.next_occurrence_expected(),
            },
            "one_event": {
                "posterior": one_event,
                "credible_interval_95": one_event_ci,
            },
            "default_consistency": {
                "posterior": default_consistency,
                "mean_consistency": default_consistency.mean_consistency(),
                "serde_bytes": String::from_utf8(serialized).unwrap(),
                "roundtrip": roundtrip,
            },
            "invalid_interval": invalid_interval_code(),
            "negative_events": negative_events_code(),
            "negative_successes": negative_successes_code(),
            "unreliable_one_success_nine_failures": unreliable_low_success(),
        }),
    );

    assert_eq!(default_rate.mean_rate(), 1.0);
    assert_eq!(default_rate.next_occurrence_expected(), 1.0);
    assert!(one_event_ci.0.is_finite() && one_event_ci.1.is_finite());
    assert!(one_event_ci.1 > one_event_ci.0);
    assert_eq!(default_consistency.mean_consistency(), 0.5);
    assert_eq!(roundtrip, default_consistency);
    assert_eq!(invalid_interval_code(), CALYX_BAYES_INVALID_INTERVAL);
    assert_eq!(negative_events_code(), CALYX_BAYES_INVALID_INTERVAL);
    assert_eq!(negative_successes_code(), CALYX_BAYES_INVALID_INTERVAL);
    assert!(!unreliable_low_success());
}

proptest! {
    #[test]
    fn credible_intervals_contain_means_and_shrink(events in 0u64..25, failures in 0u64..25) {
        let mut small_rate = GammaPoisson::default();
        small_rate.update(events, 5.0).unwrap();
        let mut large_rate = GammaPoisson::default();
        large_rate.update(events * 4, 20.0).unwrap();
        let small_rate_ci = small_rate.credible_interval_95().unwrap();
        let large_rate_ci = large_rate.credible_interval_95().unwrap();
        prop_assert!(small_rate_ci.0 <= small_rate.mean_rate());
        prop_assert!(small_rate.mean_rate() <= small_rate_ci.1);
        prop_assert!(large_rate_ci.0 <= large_rate.mean_rate());
        prop_assert!(large_rate.mean_rate() <= large_rate_ci.1);
        prop_assert!(width(large_rate_ci) <= width(small_rate_ci) + 1e-9);

        let mut small_beta = BetaBernoulli::default();
        small_beta.update(events, failures).unwrap();
        let mut large_beta = BetaBernoulli::default();
        large_beta.update(events * 4, failures * 4).unwrap();
        let small_beta_ci = small_beta.credible_interval_95().unwrap();
        let large_beta_ci = large_beta.credible_interval_95().unwrap();
        prop_assert!(small_beta_ci.0 <= small_beta.mean_consistency());
        prop_assert!(small_beta.mean_consistency() <= small_beta_ci.1);
        prop_assert!(large_beta_ci.0 <= large_beta.mean_consistency());
        prop_assert!(large_beta.mean_consistency() <= large_beta_ci.1);
        prop_assert!(width(large_beta_ci) <= width(small_beta_ci) + 1e-9);
    }
}

fn width(ci: (f64, f64)) -> f64 {
    ci.1 - ci.0
}

fn invalid_interval_code() -> &'static str {
    let mut posterior = GammaPoisson::default();
    posterior.update(1, 0.0).unwrap_err().code
}

fn negative_events_code() -> &'static str {
    let mut posterior = GammaPoisson::default();
    posterior.update_signed(-1, 1.0).unwrap_err().code
}

fn negative_successes_code() -> &'static str {
    let mut posterior = BetaBernoulli::default();
    posterior.update_signed(-1, 0).unwrap_err().code
}

fn unreliable_low_success() -> bool {
    let mut posterior = BetaBernoulli::default();
    posterior.update(1, 9).unwrap();
    posterior.is_reliable(0.7, 0.9).unwrap()
}

fn vault() -> (AsterVault, String) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return (
            AsterVault::new(vault_id(), b"bayesian"),
            "in-memory".to_string(),
        );
    };
    let dir = Path::new(&root).join("bayesian-vault");
    let _ = fs::remove_dir_all(&dir);
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"bayesian", VaultOptions::default()).unwrap();
    (vault, dir.display().to_string())
}

fn domain() -> calyx_assay::Domain {
    calyx_assay::Domain::with_outcome_anchor(
        "ph52-bayes-domain",
        Vec::new(),
        AnchorKind::Label("oracle-consistency".to_string()),
    )
}

fn missing_domain() -> calyx_assay::Domain {
    calyx_assay::Domain::new("missing-domain", Vec::new())
}

fn vault_id() -> VaultId {
    "01J9Y6KQ9Q7P7X7Q7P7X7Q7P7X".parse().unwrap()
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
