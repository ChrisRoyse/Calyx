use std::collections::BTreeMap;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, FixedClock, InputRef, LedgerRef, Modality,
    SlotVector, VaultId, VaultStore,
};
use proptest::prelude::*;

use super::*;
use crate::cf::ColumnFamily;
use crate::dedup::{
    CALYX_DEDUP_ANCHOR_CONFLICT, CALYX_DEDUP_DPI_EXCEEDED, CALYX_DEDUP_INVALID_TAU,
    CALYX_DEDUP_NO_REQUIRED_SLOTS, ConflictReason, DedupAction, contested_with_key,
    decode_contested_with,
};

#[test]
fn off_policy_returns_no_match_even_when_exact_exists() {
    let vault = sample_vault();
    let cx = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
    vault.put(cx.clone()).expect("put existing");

    let decision = check_dedup(&cx, &vault, &DedupPolicy::Off, None).expect("dedup");

    assert_eq!(decision, DedupDecision::NoMatch);
}

#[test]
fn exact_policy_rejects_same_cxid_conflicting_anchor() {
    let vault = sample_vault();
    let existing = sample_cx_with_anchors(
        1,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![anchor(
            AnchorKind::SpeakerMatch,
            AnchorValue::Text("speaker-a".to_string()),
        )],
    );
    let mut new = existing.clone();
    new.anchors = vec![anchor(
        AnchorKind::SpeakerMatch,
        AnchorValue::Text("speaker-b".to_string()),
    )];
    vault.put(existing.clone()).expect("put existing");

    let error = check_dedup(&new, &vault, &DedupPolicy::Exact, None)
        .expect_err("same-CxId anchor conflict must fail closed");

    assert_eq!(error.code, CALYX_DEDUP_ANCHOR_CONFLICT);
    assert!(contested_row(&vault, existing.cx_id).is_none());
}

#[test]
fn tct_cosine_rejects_same_cxid_conflicting_anchor_before_self_skip() {
    let vault = sample_vault();
    let existing = sample_cx_with_anchors(
        1,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![anchor(
            AnchorKind::SpeakerMatch,
            AnchorValue::Text("speaker-a".to_string()),
        )],
    );
    let mut new = existing.clone();
    new.anchors = vec![anchor(
        AnchorKind::SpeakerMatch,
        AnchorValue::Text("speaker-b".to_string()),
    )];
    vault.put(existing).expect("put existing");

    let error = check_dedup(&new, &vault, &policy([(slot(0), 0.9)], vec![slot(0)]), None)
        .expect_err("same-CxId anchor conflict must fail before self-skip");

    assert_eq!(error.code, CALYX_DEDUP_ANCHOR_CONFLICT);
}

#[test]
fn identical_vectors_match_with_per_slot_cosine() {
    let vault = sample_vault();
    let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
    let new = sample_cx(2, [(slot(0), dense(vec![1.0, 0.0]))]);
    vault.put(existing.clone()).expect("put existing");

    let decision =
        check_dedup(&new, &vault, &policy([(slot(0), 0.9)], vec![slot(0)]), None).expect("dedup");

    assert_eq!(
        decision,
        DedupDecision::Match {
            existing: existing.cx_id,
            per_slot_cos: vec![(slot(0), 1.0)]
        }
    );
}

#[test]
fn below_tau_returns_no_match() {
    let vault = sample_vault();
    let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
    let new = sample_cx(2, [(slot(0), dense(cos_vector(0.88)))]);
    vault.put(existing).expect("put existing");

    let decision =
        check_dedup(&new, &vault, &policy([(slot(0), 0.9)], vec![slot(0)]), None).expect("dedup");

    assert_eq!(decision, DedupDecision::NoMatch);
}

#[test]
fn all_required_slots_must_pass_independently() {
    let vault = sample_vault();
    let existing = sample_cx(
        1,
        [
            (slot(0), dense(vec![1.0, 0.0])),
            (slot(1), dense(vec![1.0, 0.0])),
        ],
    );
    let new = sample_cx(
        2,
        [
            (slot(0), dense(cos_vector(0.95))),
            (slot(1), dense(cos_vector(0.80))),
        ],
    );
    vault.put(existing).expect("put existing");

    let decision = check_dedup(
        &new,
        &vault,
        &policy([(slot(0), 0.9), (slot(1), 0.9)], vec![slot(0), slot(1)]),
        None,
    )
    .expect("dedup");

    assert_eq!(decision, DedupDecision::NoMatch);
}

#[test]
fn calibrated_missing_profile_fails_closed() {
    let config = TctCosineConfig::new(
        vec![slot(0)],
        TauStrategy::Calibrated,
        DedupAction::Collapse,
    )
    .expect("config");

    let error = resolve_tau(slot(0), &config, None).expect_err("missing profile");

    assert_eq!(error.code, CALYX_DEDUP_MISSING_GUARD_PROFILE);
}

#[test]
fn calibrated_invalid_profile_tau_fails_closed_before_matching() {
    for invalid_tau in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.01, -1.01] {
        let vault = sample_vault();
        let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
        let new = sample_cx(2, [(slot(0), dense(cos_vector(0.95)))]);
        vault.put(existing).expect("put existing");
        let mut profile = BTreeMap::new();
        profile.insert(slot(0), invalid_tau);
        let policy = DedupPolicy::TctCosine(
            TctCosineConfig::new(
                vec![slot(0)],
                TauStrategy::Calibrated,
                DedupAction::Collapse,
            )
            .expect("policy"),
        );

        let error = check_dedup(&new, &vault, &policy, Some(&profile)).expect_err("invalid tau");

        assert_eq!(error.code, CALYX_DEDUP_INVALID_TAU);
    }
}

#[test]
fn bypassed_empty_required_slots_fail_closed_before_matching() {
    let vault = sample_vault();
    let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
    let new = sample_cx(2, [(slot(0), dense(vec![1.0, 0.0]))]);
    vault.put(existing).expect("put existing");
    let config = TctCosineConfig {
        required_slots: Vec::new(),
        tau: TauStrategy::Calibrated,
        action: DedupAction::Collapse,
    };
    let policy = DedupPolicy::TctCosine(config.clone());

    let check_error = check_dedup(&new, &vault, &policy, None).expect_err("empty slots");
    let pass_error =
        cosine_passes_all_required(&new, &new, &config, None).expect_err("empty slots");

    assert_eq!(check_error.code, CALYX_DEDUP_NO_REQUIRED_SLOTS);
    assert_eq!(pass_error.code, CALYX_DEDUP_NO_REQUIRED_SLOTS);
}

#[test]
fn calibrated_profile_tau_matches_without_aster_ward_dependency() {
    let vault = sample_vault();
    let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
    let new = sample_cx(2, [(slot(0), dense(cos_vector(0.95)))]);
    vault.put(existing.clone()).expect("put existing");
    let mut profile = BTreeMap::new();
    profile.insert(slot(0), 0.9);
    let policy = DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![slot(0)],
            TauStrategy::Calibrated,
            DedupAction::Collapse,
        )
        .expect("policy"),
    );

    let decision = check_dedup(&new, &vault, &policy, Some(&profile)).expect("dedup");

    assert_eq!(decision_existing(&decision), Some(existing.cx_id));
    assert!(slot_cosine_close(&decision, slot(0), 0.95));
}

#[test]
fn missing_required_slot_fails_closed() {
    let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
    let new = sample_cx(2, [(slot(1), dense(vec![1.0, 0.0]))]);
    let error = cosine_passes_all_required(
        &new,
        &existing,
        &policy([(slot(0), 0.9)], vec![slot(0)]).tct_config(),
        None,
    )
    .expect_err("missing slot");

    assert_eq!(error.code, CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION);
}

#[test]
fn empty_vault_returns_no_match() {
    let vault = sample_vault();
    let new = sample_cx(2, [(slot(0), dense(vec![1.0, 0.0]))]);

    let decision =
        check_dedup(&new, &vault, &policy([(slot(0), 0.9)], vec![slot(0)]), None).expect("dedup");

    assert_eq!(decision, DedupDecision::NoMatch);
}

#[test]
fn candidate_set_over_dpi_fails_closed_when_exact_not_found() {
    let vault = sample_vault();
    let existing = sample_cx(1, [(slot(0), dense(vec![1.0, 0.0]))]);
    let new = sample_cx(2, [(slot(0), dense(vec![1.0, 0.0]))]);
    vault.put(existing).expect("put existing");

    let error = check_dedup_with_limit(
        &new,
        &vault,
        &policy([(slot(0), 0.9)], vec![slot(0)]),
        None,
        0,
    )
    .expect_err("dpi exceeded");

    assert_eq!(error.code, CALYX_DEDUP_DPI_EXCEEDED);
}

#[test]
fn anchor_conflict_writes_contested_rows_before_cosine() {
    let vault = sample_vault();
    let existing = sample_cx_with_anchors(
        1,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![anchor(
            AnchorKind::SpeakerMatch,
            AnchorValue::Text("speaker-a".to_string()),
        )],
    );
    let new = sample_cx_with_anchors(
        2,
        [],
        vec![anchor(
            AnchorKind::SpeakerMatch,
            AnchorValue::Text("speaker-b".to_string()),
        )],
    );
    vault.put(existing.clone()).expect("put existing");

    let decision =
        check_dedup(&new, &vault, &policy([(slot(0), 0.9)], vec![slot(0)]), None).expect("dedup");

    assert_eq!(
        decision,
        DedupDecision::AnchorConflict {
            existing: existing.cx_id
        }
    );
    assert_contested(&vault, new.cx_id, existing.cx_id);
    assert_contested(&vault, existing.cx_id, new.cx_id);
}

#[test]
fn no_shared_anchor_type_continues_to_cosine_match() {
    let vault = sample_vault();
    let existing = sample_cx_with_anchors(
        1,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![anchor(
            AnchorKind::SpeakerMatch,
            AnchorValue::Text("speaker-a".to_string()),
        )],
    );
    let new = sample_cx_with_anchors(
        2,
        [(slot(0), dense(cos_vector(0.95)))],
        vec![anchor(
            AnchorKind::StyleHold,
            AnchorValue::Vector(cos_vector(0.85)),
        )],
    );
    vault.put(existing.clone()).expect("put existing");

    let decision =
        check_dedup(&new, &vault, &policy([(slot(0), 0.9)], vec![slot(0)]), None).expect("dedup");

    assert_eq!(decision_existing(&decision), Some(existing.cx_id));
}

proptest! {
    #[test]
    fn identical_constellations_always_match(seed in 1u8..=u8::MAX) {
        let vault = sample_vault();
        let slots = [(slot(0), dense(vec![0.25, 0.50, 0.75]))];
        let existing = sample_cx(seed, slots.clone());
        let new = sample_cx(seed.wrapping_add(1), slots);
        vault.put(existing.clone()).expect("put existing");

        let decision = check_dedup(
            &new,
            &vault,
            &policy([(slot(0), 0.9)], vec![slot(0)]),
            None,
        )
        .expect("dedup");

        prop_assert_eq!(decision_existing(&decision), Some(existing.cx_id));
        prop_assert!(slot_cosine_close(&decision, slot(0), 1.0));
    }
}

trait PolicyConfig {
    fn tct_config(&self) -> TctCosineConfig;
}

impl PolicyConfig for DedupPolicy {
    fn tct_config(&self) -> TctCosineConfig {
        match self {
            DedupPolicy::TctCosine(config) => config.clone(),
            DedupPolicy::Off | DedupPolicy::Exact => unreachable!("test policy is tct"),
        }
    }
}

fn decision_existing(decision: &DedupDecision) -> Option<CxId> {
    match decision {
        DedupDecision::Match { existing, .. } => Some(*existing),
        DedupDecision::NoMatch | DedupDecision::AnchorConflict { .. } => None,
    }
}

fn slot_cosine_close(decision: &DedupDecision, slot: SlotId, expected: f32) -> bool {
    match decision {
        DedupDecision::Match { per_slot_cos, .. } => per_slot_cos
            .iter()
            .any(|(actual_slot, actual)| *actual_slot == slot && close(*actual, expected)),
        DedupDecision::NoMatch | DedupDecision::AnchorConflict { .. } => false,
    }
}

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() <= 1.0e-5
}

fn sample_vault() -> AsterVault<FixedClock> {
    AsterVault::with_clock(
        vault_id(),
        b"dedup-engine-test-salt".to_vec(),
        FixedClock::new(1),
    )
}

fn sample_cx<const N: usize>(seed: u8, slots: [(SlotId, SlotVector); N]) -> Constellation {
    sample_cx_with_anchors(seed, slots, Vec::new())
}

fn sample_cx_with_anchors<const N: usize>(
    seed: u8,
    slots: [(SlotId, SlotVector); N],
    anchors: Vec<Anchor>,
) -> Constellation {
    Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("zfs://calyx/dedup-engine/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: slots.into_iter().collect(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors,
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn anchor(kind: AnchorKind, value: AnchorValue) -> Anchor {
    Anchor {
        kind,
        value,
        source: "synthetic-dedup-engine".to_string(),
        observed_at: 1,
        confidence: 1.0,
    }
}

fn assert_contested(vault: &AsterVault<FixedClock>, id: CxId, contested_with: CxId) {
    let bytes = contested_row(vault, id).expect("contested row");
    let decoded = decode_contested_with(&bytes).expect("decode contested");
    assert_eq!(decoded.contested_with, contested_with);
    assert_eq!(decoded.anchor_type, AnchorKind::SpeakerMatch);
    assert_eq!(decoded.reason, ConflictReason::OppositeValue);
}

fn contested_row(vault: &AsterVault<FixedClock>, id: CxId) -> Option<Vec<u8>> {
    vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Online,
            &contested_with_key(id),
        )
        .expect("read contested")
}

fn policy<const N: usize>(tau: [(SlotId, f32); N], required: Vec<SlotId>) -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            required,
            TauStrategy::PerSlot(tau.into_iter().collect()),
            DedupAction::Collapse,
        )
        .expect("policy"),
    )
}

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

fn cos_vector(cos: f32) -> Vec<f32> {
    vec![cos, (1.0 - cos * cos).sqrt()]
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
