use std::collections::BTreeMap;

use calyx_anneal::{
    RefreshPriority, RetentionTier, anneal_retention_tier, frequency_kernel_bonus,
    recurrence_schedule_for,
};
use calyx_aster::dedup::{CALYX_DEDUP_MISSING_FREQUENCY, EpochSecs};
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, OccurrenceContext, RetentionPolicy, append_occurrence,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultStore,
};
use proptest::prelude::*;

mod fsv_support;
use fsv_support::vault_id;

#[test]
fn recurrence_schedule_prioritizes_cadence_bands() {
    let clock = FixedClock::new(1_000);

    assert_eq!(
        schedule_for_cadence(1, 1_800, &clock).refresh_priority,
        RefreshPriority::Hot
    );
    assert_eq!(
        schedule_for_cadence(2, 43_200, &clock).refresh_priority,
        RefreshPriority::Warm
    );
    assert_eq!(
        schedule_for_cadence(3, 90_000, &clock).refresh_priority,
        RefreshPriority::Cold
    );

    let vault = vault();
    vault.put(row(4, Some(1.0))).expect("put base");
    let schedule = recurrence_schedule_for(cx(4), &vault, &clock).expect("one-time schedule");
    assert_eq!(schedule.refresh_priority, RefreshPriority::OneTime);
    assert_eq!(schedule.next_expected_t, None);
}

#[test]
fn recurrence_schedule_sets_next_expected_time() {
    let clock = FixedClock::new(1_000);
    let schedule = schedule_for_cadence(5, 1_800, &clock);

    assert_eq!(schedule.next_expected_t, Some(EpochSecs(13_600)));
}

#[test]
fn importance_weight_matches_frequency_kernel_bounds() {
    assert_eq!(frequency_kernel_bonus(0), 0.0);
    assert_eq!(frequency_kernel_bonus(10_000), 1.0);
}

#[test]
fn anneal_retention_tier_maps_priority_to_storage_tier() {
    let clock = FixedClock::new(1_000);
    let hot_vault = vault_with_cadence(6, 1_800);
    let cold_vault = vault_with_cadence(7, 90_000);

    assert_eq!(
        anneal_retention_tier(cx(6), &hot_vault, &clock).expect("hot tier"),
        RetentionTier::Memtable
    );
    assert_eq!(
        anneal_retention_tier(cx(7), &cold_vault, &clock).expect("cold tier"),
        RetentionTier::Archive
    );
}

#[test]
fn zero_frequency_without_cadence_is_one_time_with_zero_importance() {
    let clock = FixedClock::new(1_000);
    let vault = vault();
    vault.put(row(8, Some(0.0))).expect("put base");

    let schedule = recurrence_schedule_for(cx(8), &vault, &clock).expect("schedule");

    assert_eq!(schedule.importance_weight, 0.0);
    assert_eq!(schedule.refresh_priority, RefreshPriority::OneTime);
    assert_eq!(schedule.next_expected_t, None);
}

#[test]
fn missing_frequency_fails_closed_with_dedup_code() {
    let clock = FixedClock::new(1_000);
    let vault = vault();
    vault.put(row(9, None)).expect("put base");

    let error = recurrence_schedule_for(cx(9), &vault, &clock).expect_err("missing frequency");

    assert_eq!(error.code, CALYX_DEDUP_MISSING_FREQUENCY);
}

proptest! {
    #[test]
    fn frequency_importance_stays_bounded(frequency in any::<u64>()) {
        let weight = frequency_kernel_bonus(frequency);
        prop_assert!((0.0..=1.0).contains(&weight));
    }
}

fn schedule_for_cadence(
    seed: u8,
    cadence_secs: i64,
    clock: &FixedClock,
) -> calyx_anneal::RecurrenceSchedule {
    let vault = vault_with_cadence(seed, cadence_secs);
    recurrence_schedule_for(cx(seed), &vault, clock).expect("schedule")
}

fn vault_with_cadence(seed: u8, cadence_secs: i64) -> AsterVault {
    let vault = vault();
    vault.put(row(seed, None)).expect("put base");
    append_occurrence_at(&vault, seed, 10_000);
    append_occurrence_at(&vault, seed, 10_000 + cadence_secs);
    vault
}

fn append_occurrence_at(vault: &AsterVault, seed: u8, time: i64) {
    append_occurrence(
        vault,
        cx(seed),
        EpochSecs(time),
        OccurrenceContext::new(format!("{seed}-{time}")).expect("context"),
        EpochSecs(time),
        RetentionPolicy::default(),
    )
    .expect("append recurrence");
}

fn row(seed: u8, frequency: Option<f64>) -> calyx_core::Constellation {
    let mut scalars = BTreeMap::new();
    if let Some(frequency) = frequency {
        scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    }
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1_000_000,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::<SlotId, SlotVector>::new(),
        scalars,
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn vault() -> AsterVault {
    AsterVault::new(vault_id(), b"anneal-recurrence")
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}
