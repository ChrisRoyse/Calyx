//! Reusable deterministic test scaffolding for Calyx crates.

use std::collections::BTreeMap;

use calyx_core::{
    AbsentReason, Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, FixedClock,
    InputRef, LedgerRef, Modality, SlotId, SlotVector, Ts, VaultId,
};
use proptest::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;

/// Default seed for deterministic Calyx tests.
pub const DEFAULT_TEST_SEED: u64 = 0xCA1A_CAFE_D15C_1A11;

/// Default fixed timestamp for deterministic Calyx tests.
pub const DEFAULT_TEST_TS: Ts = 1_785_500_000;

/// Builds a deterministic RNG.
pub fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

/// Builds the standard fixed test clock.
pub fn fixed_clock() -> FixedClock {
    FixedClock::new(DEFAULT_TEST_TS)
}

/// Strategy for stable slot ids.
pub fn slot_id_strategy() -> BoxedStrategy<SlotId> {
    any::<u16>().prop_map(SlotId::new).boxed()
}

/// Strategy for stable constellation ids.
pub fn cx_id_strategy() -> BoxedStrategy<CxId> {
    prop::collection::vec(any::<u8>(), 16)
        .prop_map(|bytes| {
            let mut out = [0; 16];
            out.copy_from_slice(&bytes);
            CxId::from_bytes(out)
        })
        .boxed()
}

/// Strategy for supported input modalities.
pub fn modality_strategy() -> BoxedStrategy<Modality> {
    prop_oneof![
        Just(Modality::Text),
        Just(Modality::Code),
        Just(Modality::Image),
        Just(Modality::Audio),
        Just(Modality::Video),
        Just(Modality::Structured),
        Just(Modality::Mixed),
    ]
    .boxed()
}

/// Strategy for anchor kinds, including labels.
pub fn anchor_kind_strategy() -> BoxedStrategy<AnchorKind> {
    prop_oneof![
        Just(AnchorKind::TestPass),
        Just(AnchorKind::TieFormed),
        Just(AnchorKind::Thumbs),
        "[a-z]{1,8}".prop_map(AnchorKind::Label),
        Just(AnchorKind::Reward),
        Just(AnchorKind::SpeakerMatch),
        Just(AnchorKind::StyleHold),
        Just(AnchorKind::Recurrence),
    ]
    .boxed()
}

/// Strategy for explicit absence reasons.
pub fn absent_reason_strategy() -> BoxedStrategy<AbsentReason> {
    prop_oneof![
        Just(AbsentReason::NotApplicable),
        Just(AbsentReason::Redacted),
        Just(AbsentReason::LensUnavailable),
        Just(AbsentReason::Deferred),
        Just(AbsentReason::LensInactive),
        "[A-Z_]{1,16}".prop_map(AbsentReason::Error),
    ]
    .boxed()
}

/// Strategy for small slot vectors.
pub fn slot_vector_strategy() -> BoxedStrategy<SlotVector> {
    let dense = prop::collection::vec(0u8..=10, 0..4).prop_map(|values| SlotVector::Dense {
        dim: values.len() as u32,
        data: values
            .into_iter()
            .map(|value| f32::from(value) / 10.0)
            .collect(),
    });
    let absent = absent_reason_strategy().prop_map(|reason| SlotVector::Absent { reason });

    prop_oneof![dense, absent].boxed()
}

/// Strategy for small deterministic constellations.
pub fn small_constellation_strategy() -> BoxedStrategy<Constellation> {
    (
        cx_id_strategy(),
        modality_strategy(),
        1u32..16,
        any::<bool>(),
        slot_vector_strategy(),
    )
        .prop_map(|(cx_id, modality, panel_version, redacted, slot_vector)| {
            let mut slots = BTreeMap::new();
            if !redacted {
                slots.insert(SlotId::new(1), slot_vector);
            }

            Constellation {
                cx_id,
                vault_id: test_vault_id(),
                panel_version,
                created_at: DEFAULT_TEST_TS,
                input_ref: InputRef {
                    hash: [3; 32],
                    pointer: (!redacted).then(|| "zfs://calyx/testkit/input".to_string()),
                    redacted,
                },
                modality,
                slots,
                scalars: BTreeMap::new(),
                anchors: (!redacted)
                    .then(|| Anchor {
                        kind: AnchorKind::Reward,
                        value: AnchorValue::Number(1.0),
                        source: "testkit".to_string(),
                        observed_at: DEFAULT_TEST_TS,
                        confidence: 1.0,
                    })
                    .into_iter()
                    .collect(),
                provenance: LedgerRef {
                    seq: 1,
                    hash: [4; 32],
                },
                flags: CxFlags {
                    ungrounded: redacted,
                    degraded: false,
                    novel_region: false,
                    redacted_input: redacted,
                },
            }
        })
        .boxed()
}

fn test_vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse::<VaultId>()
        .expect("valid test vault id")
}

#[cfg(test)]
mod tests {
    use rand::RngCore;

    use super::*;

    #[test]
    fn seeded_rng_replays_exact_bytes() {
        let mut first = seeded_rng(DEFAULT_TEST_SEED);
        let mut second = seeded_rng(DEFAULT_TEST_SEED);
        let mut first_bytes = [0; 32];
        let mut second_bytes = [0; 32];

        first.fill_bytes(&mut first_bytes);
        second.fill_bytes(&mut second_bytes);

        assert_eq!(first_bytes, second_bytes);
    }

    #[test]
    fn fixed_clock_helper_is_stable() {
        assert_eq!(fixed_clock(), FixedClock::new(DEFAULT_TEST_TS));
    }

    proptest! {
        #[test]
        fn slot_id_display_parse_roundtrips(id in slot_id_strategy()) {
            let parsed = id.to_string().parse::<SlotId>().expect("parse slot id");
            prop_assert_eq!(parsed, id);
        }

        #[test]
        fn generated_constellation_serde_roundtrips(cx in small_constellation_strategy()) {
            let first = serde_json::to_vec(&cx).expect("serialize constellation");
            let decoded: Constellation =
                serde_json::from_slice(&first).expect("deserialize constellation");
            let second = serde_json::to_vec(&decoded).expect("serialize decoded constellation");

            prop_assert_eq!(first, second);
            prop_assert_eq!(cx, decoded);
        }

        #[test]
        fn generated_absent_vector_stays_absent(reason in absent_reason_strategy()) {
            let vector = SlotVector::Absent { reason };
            let bytes = serde_json::to_vec(&vector).expect("serialize absent vector");
            let decoded: SlotVector =
                serde_json::from_slice(&bytes).expect("deserialize absent vector");

            prop_assert!(decoded.is_absent());
            prop_assert!(decoded.as_dense().is_none());
        }
    }
}
