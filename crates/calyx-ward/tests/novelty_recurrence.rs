use calyx_aster::dedup::EpochSecs;
use calyx_aster::vault::AsterVault;
use calyx_core::FixedClock;
use calyx_ward::{
    CALYX_WARD_MISSING_FREQUENCY, Domain, NoveltyAction, NoveltySignal, SurpriseScore,
    classify_novelty, novelty::surprise_score_from_counts, novelty_action_for_signal,
    overdue_recurrence_scan, surprise_bits,
};
use proptest::prelude::*;

#[path = "novelty_recurrence_support/mod.rs"]
mod novelty_recurrence_support;
use novelty_recurrence_support::{append_times, cx, put_base, vault_id};

#[test]
fn frequency_zero_and_one_are_non_recurring() {
    let vault = AsterVault::new(vault_id(), b"ward-novelty-non-recurring");
    put_base(&vault, cx(1), Some(0.0));
    put_base(&vault, cx(2), Some(1.0));
    let clock = FixedClock::new(1_350_000);

    assert_eq!(
        classify_novelty(cx(1), &vault, &clock).unwrap(),
        NoveltySignal::NonRecurring
    );
    assert_eq!(
        classify_novelty(cx(2), &vault, &clock).unwrap(),
        NoveltySignal::NonRecurring
    );
}

#[test]
fn recurring_event_past_two_cadences_is_overdue() {
    let vault = AsterVault::new(vault_id(), b"ward-novelty-overdue");
    let id = cx(3);
    put_base(&vault, id, None);
    append_times(
        &vault,
        id,
        &[100, 200, 300, 400, 500, 600, 700, 800, 900, 1_000],
    );
    let clock = FixedClock::new(1_350_000);

    let signal = classify_novelty(id, &vault, &clock).unwrap();

    assert_eq!(
        signal,
        NoveltySignal::OverdueRecurrence {
            expected_t: EpochSecs(1_100),
            overdue_by_secs: 250
        }
    );
}

#[test]
fn surprise_bits_match_hand_computed_domain_probabilities() {
    let vault = AsterVault::new(vault_id(), b"ward-novelty-surprise");
    let rare = cx(4);
    let common = cx(5);
    let filler = cx(6);
    put_base(&vault, rare, Some(1.0));
    put_base(&vault, common, Some(50.0));
    put_base(&vault, filler, Some(49.0));
    let domain = Domain::new("hundred-events", vec![rare, common, filler]);

    let rare_score = surprise_bits(rare, &domain, &vault).unwrap();
    let common_score = surprise_bits(common, &domain, &vault).unwrap();

    assert!((rare_score.get() - 6.643_856).abs() < 1e-5);
    assert!((common_score.get() - 1.0).abs() < 1e-6);
}

#[test]
fn empty_domain_smooths_to_zero_surprise() {
    let vault = AsterVault::new(vault_id(), b"ward-novelty-empty-domain");
    let score = surprise_bits(cx(7), &Domain::new("empty", Vec::new()), &vault).unwrap();

    assert_eq!(score, SurpriseScore::new(0.0).unwrap());
}

#[test]
fn missing_frequency_fails_closed() {
    let vault = AsterVault::new(vault_id(), b"ward-novelty-missing-frequency");
    let id = cx(8);
    put_base(&vault, id, None);
    let clock = FixedClock::new(1_350_000);

    let error = classify_novelty(id, &vault, &clock).unwrap_err();

    assert_eq!(error.code(), CALYX_WARD_MISSING_FREQUENCY);
}

#[test]
fn overdue_scan_and_action_mapping_surface_ward_novelty_routes() {
    let vault = AsterVault::new(vault_id(), b"ward-novelty-scan");
    let overdue = cx(9);
    let singleton = cx(10);
    put_base(&vault, overdue, None);
    append_times(&vault, overdue, &[100, 200, 300]);
    put_base(&vault, singleton, Some(1.0));
    let domain = Domain::new("scan", vec![overdue, singleton]);
    let clock = FixedClock::new(700_000);

    let rows = overdue_recurrence_scan(&domain, &vault, &clock).unwrap();
    let singleton_signal = classify_novelty(singleton, &vault, &clock).unwrap();
    let anomaly = NoveltySignal::Anomaly {
        surprise_bits: SurpriseScore::new(6.0).unwrap(),
    };

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, overdue);
    assert_eq!(
        novelty_action_for_signal(&singleton_signal),
        Some(NoveltyAction::NewRegion)
    );
    assert_eq!(
        novelty_action_for_signal(&anomaly),
        Some(NoveltyAction::Quarantine)
    );
}

proptest! {
    #[test]
    fn surprise_score_is_non_negative(frequency in any::<u64>(), total in any::<u64>()) {
        let score = surprise_score_from_counts(frequency, total).unwrap();
        prop_assert!(score.get().is_finite());
        prop_assert!(score.get() >= 0.0);
    }
}
