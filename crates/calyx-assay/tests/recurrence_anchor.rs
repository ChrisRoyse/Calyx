use calyx_assay::{
    CALYX_ASSAY_MISSING_OUTCOME_SLOT, Domain, OutcomeAgreement, frequency_anchor_for,
    measure_outcome_agreement, oracle_self_consistency, oracle_self_consistency_from_agreements,
    outcome_agreement_from_observations, outcome_occurrence_context,
};
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, OccurrenceContext, RetentionPolicy, append_occurrence,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, AnchorValue, CxId, VaultStore};
use proptest::prelude::*;

#[path = "recurrence_anchor_support/mod.rs"]
mod recurrence_anchor_support;
use recurrence_anchor_support::{append_outcomes, base_cx, cx_id, vault_id};

#[test]
fn frequency_anchor_reads_base_scalar_without_recurrence_rows() {
    let vault = vault();
    let cx_id = cx_id(1);
    let mut cx = base_cx(cx_id);
    cx.scalars.insert(FREQUENCY_SCALAR.to_string(), 7.0);
    vault.put(cx).expect("put base cx");

    let anchor = frequency_anchor_for(cx_id, &vault).expect("frequency anchor");

    assert_eq!(anchor.cx_id, cx_id);
    assert_eq!(anchor.frequency, 7);
    assert_eq!(anchor.cadence_secs, None);
}

#[test]
fn identical_outcomes_are_consistent() {
    let vault = vault_with_base(cx_id(2));
    append_outcomes(&vault, cx_id(2), &["pass", "pass", "pass", "pass", "pass"]);

    let agreement = measure_outcome_agreement(cx_id(2), &vault).expect("agreement");

    assert_eq!(
        agreement,
        OutcomeAgreement::Consistent {
            agreement_rate: 1.0
        }
    );
}

#[test]
fn three_agree_three_unique_differ_is_flaky() {
    let vault = vault_with_base(cx_id(3));
    append_outcomes(
        &vault,
        cx_id(3),
        &["pass", "pass", "pass", "fail-a", "fail-b", "fail-c"],
    );

    let agreement = measure_outcome_agreement(cx_id(3), &vault).expect("agreement");

    assert_rate(&agreement, 3.0 / 15.0);
    assert!(matches!(agreement, OutcomeAgreement::Flaky { .. }));
}

#[test]
fn two_occurrences_are_insufficient() {
    let vault = vault_with_base(cx_id(4));
    append_outcomes(&vault, cx_id(4), &["pass", "pass"]);

    let agreement = measure_outcome_agreement(cx_id(4), &vault).expect("agreement");

    assert_eq!(agreement, OutcomeAgreement::Insufficient { n: 2 });
}

#[test]
fn domain_without_recurring_cx_ids_is_permissive() {
    let vault = vault();
    put_base_with_frequency(&vault, cx_id(5), 0);
    put_base_with_frequency(&vault, cx_id(6), 2);
    let domain = Domain::new("no-recurring", vec![cx_id(5), cx_id(6)]);

    let score = oracle_self_consistency(&domain, &vault).expect("self consistency");

    assert_eq!(score, 1.0);
}

#[test]
fn domain_self_consistency_averages_recurring_agreement_rates() {
    let vault = vault();
    for id in [cx_id(7), cx_id(8)] {
        vault.put(base_cx(id)).expect("put base");
    }
    append_outcomes(&vault, cx_id(7), &["ok", "ok", "ok"]);
    append_outcomes(
        &vault,
        cx_id(8),
        &["ok", "ok", "ok", "bad-a", "bad-b", "bad-c"],
    );
    let domain = Domain::new("mixed", vec![cx_id(7), cx_id(8)]);

    let score = oracle_self_consistency(&domain, &vault).expect("self consistency");

    assert!((score - 0.6).abs() < f32::EPSILON);
}

#[test]
fn explicit_agreement_rates_average_to_oracle_self_consistency() {
    let agreements = [
        OutcomeAgreement::Consistent {
            agreement_rate: 1.0,
        },
        OutcomeAgreement::Consistent {
            agreement_rate: 0.9,
        },
        OutcomeAgreement::Consistent {
            agreement_rate: 0.8,
        },
        OutcomeAgreement::Insufficient { n: 2 },
    ];

    let score = oracle_self_consistency_from_agreements(&agreements);

    assert!((score - 0.9).abs() < 0.000_001);
}

#[test]
fn all_missing_outcome_contexts_have_no_disagreement() {
    let vault = vault_with_base(cx_id(9));
    for index in 0..3 {
        append_occurrence(
            &vault,
            cx_id(9),
            EpochSecs(1_000 + index),
            OccurrenceContext::new(b"legacy-context".to_vec()).expect("context"),
            EpochSecs(1_000 + index),
            RetentionPolicy::default(),
        )
        .expect("append occurrence");
    }

    let agreement = measure_outcome_agreement(cx_id(9), &vault).expect("agreement");

    assert_eq!(
        agreement,
        OutcomeAgreement::Consistent {
            agreement_rate: 1.0
        }
    );
}

#[test]
fn wrong_outcome_anchor_kind_fails_closed() {
    let vault = vault_with_base(cx_id(10));
    let context = outcome_occurrence_context(AnchorKind::Reward, AnchorValue::Text("pass".into()))
        .expect("context");
    append_occurrence(
        &vault,
        cx_id(10),
        EpochSecs(1_000),
        context,
        EpochSecs(1_000),
        RetentionPolicy::default(),
    )
    .expect("append occurrence");
    append_outcomes(&vault, cx_id(10), &["pass", "pass"]);

    let error = measure_outcome_agreement(cx_id(10), &vault).expect_err("wrong anchor kind");

    assert_eq!(error.code, CALYX_ASSAY_MISSING_OUTCOME_SLOT);
}

proptest! {
    #[test]
    fn agreement_rate_stays_in_unit_interval(values in proptest::collection::vec(0_u8..5, 3..32)) {
        let observations = values
            .into_iter()
            .map(|value| {
                if value == 0 {
                    None
                } else {
                    Some(AnchorValue::Text(format!("value-{value}")))
                }
            })
            .collect::<Vec<_>>();

        let agreement = outcome_agreement_from_observations(&observations);
        let rate = agreement.agreement_rate().expect("sufficient observations");

        prop_assert!((0.0..=1.0).contains(&rate));
    }
}

fn vault() -> AsterVault {
    AsterVault::new(vault_id(), b"assay-recurrence-anchor-tests".to_vec())
}

fn vault_with_base(cx_id: CxId) -> AsterVault {
    let vault = vault();
    vault.put(base_cx(cx_id)).expect("put base");
    vault
}

fn put_base_with_frequency(vault: &AsterVault, cx_id: CxId, frequency: u64) {
    let mut cx = base_cx(cx_id);
    cx.scalars
        .insert(FREQUENCY_SCALAR.to_string(), frequency as f64);
    vault.put(cx).expect("put base");
}

fn assert_rate(agreement: &OutcomeAgreement, expected: f32) {
    let actual = agreement.agreement_rate().expect("agreement rate");
    assert!(
        (actual - expected).abs() < 0.000_001,
        "expected {expected}, got {actual}"
    );
}
