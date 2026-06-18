//! Constellation data-model structs.

pub mod anchor;
pub mod constellation;
pub mod signal;
pub mod slot;
mod validation;
pub mod vector;

pub use crate::time::Ts;
pub use anchor::{Anchor, AnchorValue};
pub use constellation::{
    Constellation, METADATA_CHUNK_ID, METADATA_DATABASE_NAME, METADATA_SOURCE_EVENT_TIME_RAW,
    METADATA_SOURCE_EVENT_TIME_SECS, METADATA_SOURCE_SEQUENCE, METADATA_TEMPORAL_INACTIVE_REASON,
    METADATA_TEMPORAL_LANE_STATE, TEMPORAL_LANE_ACTIVE, TEMPORAL_LANE_INACTIVE,
    TEMPORAL_MISSING_CREATED_AT,
};
pub use signal::{ConfidenceInterval, CxFlags, InputRef, LedgerRef, Signal};
pub use slot::{LensCost, Panel, Placement, Slot, SlotResource};
pub use validation::CALYX_RECORD_SCHEMA_VIOLATION;
pub use vector::{SlotVector, SparseEntry};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;

    use super::*;
    use crate::{
        AbsentReason, AnchorKind, Asymmetry, CxId, LensId, Modality, QuantPolicy, SlotId, SlotKey,
        SlotShape, SlotState, VaultId,
    };

    proptest! {
        #[test]
        fn constellation_json_roundtrip_is_byte_exact(
            panel_version in 1u32..=u32::MAX,
            confidence_milli in 0u16..=1000,
        ) {
            let constellation = sample_constellation(
                panel_version,
                f32::from(confidence_milli) / 1000.0,
            );
            let first = serde_json::to_vec(&constellation).expect("serialize constellation");
            let decoded: Constellation =
                serde_json::from_slice(&first).expect("deserialize constellation");
            let second = serde_json::to_vec(&decoded).expect("serialize decoded constellation");

            prop_assert_eq!(&first, &second);
            prop_assert_eq!(constellation, decoded);
        }
    }

    #[test]
    fn absent_slot_stays_absent_and_has_no_dense_data() {
        let absent = SlotVector::Absent {
            reason: AbsentReason::Deferred,
        };
        let bytes = serde_json::to_vec(&absent).expect("serialize absent vector");

        assert_eq!(bytes, br#"{"absent":{"reason":"deferred"}}"#);

        let decoded: SlotVector =
            serde_json::from_slice(&bytes).expect("deserialize absent vector");

        assert!(decoded.is_absent());
        assert_eq!(decoded.as_dense(), None);
    }

    #[test]
    fn populated_constellation_bytes_are_stable() {
        let constellation = sample_constellation(7, 0.875);
        let bytes = serde_json::to_vec(&constellation).expect("serialize constellation");
        let decoded: Constellation =
            serde_json::from_slice(&bytes).expect("deserialize constellation");

        assert_eq!(bytes, serde_json::to_vec(&decoded).unwrap());
        assert_eq!(decoded.slots.len(), 2);
        assert_eq!(decoded.chunk_id(), Some("chunk-42/source row"));
        assert_eq!(decoded.database_name(), Some("leapable_db_stage15"));
        assert!(matches!(
            decoded.slots.get(&SlotId::new(2)),
            Some(SlotVector::Absent {
                reason: AbsentReason::LensUnavailable
            })
        ));
    }

    #[test]
    fn panel_bits_about_supports_label_anchor_kind() {
        let slot_id = SlotId::new(3);
        let mut bits_about = BTreeMap::new();
        bits_about.insert(
            AnchorKind::Label("gold".to_string()),
            Signal {
                bits: 0.2,
                ci: ConfidenceInterval {
                    low: 0.18,
                    high: 0.22,
                },
                n: 80,
                estimator: "synthetic_panel".to_string(),
                ts: 1_785_200_001,
            },
        );
        let panel = Panel {
            version: 9,
            slots: vec![Slot {
                slot_id,
                slot_key: SlotKey::new(slot_id, "label-axis"),
                lens_id: LensId::from_bytes([3; 16]),
                shape: SlotShape::Sparse(1024),
                modality: Modality::Structured,
                asymmetry: Asymmetry::None,
                quant: QuantPolicy::Pq { m: 8, nbits: 4 },
                resource: Default::default(),
                axis: Some("label".to_string()),
                retrieval_only: false,
                excluded_from_dedup: false,
                bits_about,
                state: SlotState::Active,
                added_at_panel_version: 9,
            }],
            created_at: 1_785_200_000,
            kernel_ref: Some(ledger_ref(1, [1; 32])),
            guard_ref: Some(ledger_ref(2, [2; 32])),
        };

        let bytes = serde_json::to_vec(&panel).expect("serialize panel");
        assert!(String::from_utf8_lossy(&bytes).contains(r#""label:gold""#));

        let decoded: Panel = serde_json::from_slice(&bytes).expect("deserialize panel");
        assert!(
            decoded.slots[0]
                .bits_about
                .contains_key(&AnchorKind::Label("gold".to_string()))
        );
        assert_eq!(bytes, serde_json::to_vec(&decoded).unwrap());
    }

    fn sample_constellation(panel_version: u32, confidence: f32) -> Constellation {
        let slot_id = SlotId::new(1);
        let mut bits_about = BTreeMap::new();
        bits_about.insert(
            AnchorKind::Reward,
            Signal {
                bits: 0.125,
                ci: ConfidenceInterval {
                    low: 0.100,
                    high: 0.150,
                },
                n: 64,
                estimator: "synthetic_ksg".to_string(),
                ts: 1_785_000_001,
            },
        );

        let slot = Slot {
            slot_id,
            slot_key: SlotKey::new(slot_id, "text-general"),
            lens_id: LensId::from_bytes([2; 16]),
            shape: SlotShape::Dense(3),
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some("semantic".to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about,
            state: SlotState::Active,
            added_at_panel_version: 1,
        };
        let _panel = Panel {
            version: panel_version,
            slots: vec![slot],
            created_at: 1_785_000_000,
            kernel_ref: Some(ledger_ref(9, [9; 32])),
            guard_ref: Some(ledger_ref(10, [10; 32])),
        };

        let mut slots = BTreeMap::new();
        slots.insert(
            slot_id,
            SlotVector::Dense {
                dim: 3,
                data: vec![0.25, 0.5, 0.75],
            },
        );
        slots.insert(
            SlotId::new(2),
            SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            },
        );

        let mut scalars = BTreeMap::new();
        scalars.insert("coverage_delta".to_string(), 0.25);
        let mut metadata = BTreeMap::new();
        metadata.insert(
            METADATA_CHUNK_ID.to_string(),
            "chunk-42/source row".to_string(),
        );
        metadata.insert(
            METADATA_DATABASE_NAME.to_string(),
            "leapable_db_stage15".to_string(),
        );

        Constellation {
            cx_id: CxId::from_bytes([1; 16]),
            vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV"
                .parse::<VaultId>()
                .expect("valid vault id"),
            panel_version,
            created_at: 1_785_000_002,
            input_ref: InputRef {
                hash: [7; 32],
                pointer: Some("zfs://calyx/synthetic/input-1".to_string()),
                redacted: false,
            },
            modality: Modality::Text,
            slots,
            scalars,
            metadata,
            anchors: vec![Anchor {
                kind: AnchorKind::Reward,
                value: AnchorValue::Number(1.0),
                source: "synthetic-oracle".to_string(),
                observed_at: 1_785_000_003,
                confidence,
            }],
            provenance: ledger_ref(11, [11; 32]),
            flags: CxFlags {
                ungrounded: false,
                degraded: false,
                novel_region: false,
                redacted_input: false,
            },
        }
    }

    fn ledger_ref(seq: u64, hash: [u8; 32]) -> LedgerRef {
        LedgerRef { seq, hash }
    }
}
