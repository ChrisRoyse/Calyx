use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CalyxError, Constellation, CxId, Result, dense_cosine,
};
use serde::{Deserialize, Serialize};

use super::{DedupAction, DedupPolicy};

pub const ANCHOR_VECTOR_TAU: f32 = 0.70;

const CONTESTED_WITH_KEY_PREFIX: &[u8] = b"dedup:contested_with:";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AnchorConflictResult {
    Compatible,
    Conflicting {
        anchor_type: AnchorKind,
        reason: ConflictReason,
    },
    NoAnchor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConflictReason {
    OppositeValue,
    IncompatibleVector { cos: f32 },
    ExclusiveTag,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContestedWith {
    pub contested_with: CxId,
    pub anchor_type: AnchorKind,
    pub reason: ConflictReason,
}

pub fn check_anchor_conflict(
    new_cx: &Constellation,
    existing_cx: &Constellation,
) -> AnchorConflictResult {
    let mut shared_anchor = false;
    for new_anchor in &new_cx.anchors {
        for existing_anchor in existing_cx
            .anchors
            .iter()
            .filter(|anchor| anchor.kind == new_anchor.kind)
        {
            shared_anchor = true;
            if let Some(reason) = conflict_reason(new_anchor, existing_anchor) {
                return AnchorConflictResult::Conflicting {
                    anchor_type: new_anchor.kind.clone(),
                    reason,
                };
            }
        }
    }
    if shared_anchor {
        AnchorConflictResult::Compatible
    } else {
        AnchorConflictResult::NoAnchor
    }
}

pub(crate) fn is_recurrence_series_policy(policy: &DedupPolicy) -> bool {
    matches!(
        policy,
        DedupPolicy::TctCosine(config) if config.action == DedupAction::RecurrenceSeries
    )
}

pub fn contested_with_key(cx_id: CxId) -> Vec<u8> {
    let mut key = Vec::with_capacity(CONTESTED_WITH_KEY_PREFIX.len() + 16);
    key.extend_from_slice(CONTESTED_WITH_KEY_PREFIX);
    key.extend_from_slice(cx_id.as_bytes());
    key
}

pub fn encode_contested_with(value: &ContestedWith) -> Result<Vec<u8>> {
    serde_json::to_vec(value)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("encode contested row: {error}")))
}

pub fn decode_contested_with(bytes: &[u8]) -> Result<ContestedWith> {
    serde_json::from_slice(bytes)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("decode contested row: {error}")))
}

fn conflict_reason(new_anchor: &Anchor, existing_anchor: &Anchor) -> Option<ConflictReason> {
    if new_anchor.value == existing_anchor.value {
        return None;
    }
    match &new_anchor.kind {
        AnchorKind::SpeakerMatch => Some(ConflictReason::OppositeValue),
        AnchorKind::StyleHold => style_conflict_reason(&new_anchor.value, &existing_anchor.value),
        AnchorKind::Label(_) => Some(ConflictReason::ExclusiveTag),
        _ => Some(ConflictReason::ExclusiveTag),
    }
}

fn style_conflict_reason(left: &AnchorValue, right: &AnchorValue) -> Option<ConflictReason> {
    let (AnchorValue::Vector(left), AnchorValue::Vector(right)) = (left, right) else {
        return Some(ConflictReason::ExclusiveTag);
    };
    let cos = dense_cosine(left, right)
        .filter(|cos| cos.is_finite())
        .unwrap_or(-1.0);
    (cos < ANCHOR_VECTOR_TAU).then_some(ConflictReason::IncompatibleVector { cos })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use calyx_core::{AnchorValue, CxFlags, InputRef, LedgerRef, Modality, SlotVector, VaultId};
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn no_shared_anchor_returns_no_anchor() {
        let new = sample_cx(1, vec![anchor(AnchorKind::SpeakerMatch, text("speaker-a"))]);
        let existing = sample_cx(2, vec![anchor(AnchorKind::StyleHold, vector_at_cos(0.8))]);

        assert_eq!(
            check_anchor_conflict(&new, &existing),
            AnchorConflictResult::NoAnchor
        );
    }

    #[test]
    fn empty_new_anchors_return_no_anchor() {
        let new = sample_cx(1, Vec::new());
        let existing = sample_cx(2, vec![anchor(AnchorKind::SpeakerMatch, text("speaker-a"))]);

        assert_eq!(
            check_anchor_conflict(&new, &existing),
            AnchorConflictResult::NoAnchor
        );
    }

    #[test]
    fn speaker_mismatch_is_opposite_value_conflict() {
        let new = sample_cx(1, vec![anchor(AnchorKind::SpeakerMatch, text("speaker-a"))]);
        let existing = sample_cx(2, vec![anchor(AnchorKind::SpeakerMatch, text("speaker-b"))]);

        assert_eq!(
            check_anchor_conflict(&new, &existing),
            AnchorConflictResult::Conflicting {
                anchor_type: AnchorKind::SpeakerMatch,
                reason: ConflictReason::OppositeValue,
            }
        );
    }

    #[test]
    fn style_vector_below_tau_conflicts() {
        let new = sample_cx(1, vec![anchor(AnchorKind::StyleHold, vector_at_cos(1.0))]);
        let existing = sample_cx(2, vec![anchor(AnchorKind::StyleHold, vector_at_cos(0.65))]);

        let AnchorConflictResult::Conflicting { reason, .. } =
            check_anchor_conflict(&new, &existing)
        else {
            panic!("expected conflict");
        };
        let ConflictReason::IncompatibleVector { cos } = reason else {
            panic!("expected vector conflict");
        };
        assert!((cos - 0.65).abs() <= 1.0e-5);
    }

    #[test]
    fn non_finite_style_vector_conflicts_fail_closed() {
        let new = sample_cx(
            1,
            vec![anchor(
                AnchorKind::StyleHold,
                AnchorValue::Vector(vec![f32::NAN, 1.0]),
            )],
        );
        let existing = sample_cx(2, vec![anchor(AnchorKind::StyleHold, vector_at_cos(0.85))]);

        let AnchorConflictResult::Conflicting { reason, .. } =
            check_anchor_conflict(&new, &existing)
        else {
            panic!("expected conflict");
        };
        assert_eq!(reason, ConflictReason::IncompatibleVector { cos: -1.0 });
    }

    #[test]
    fn style_vector_at_or_above_tau_is_compatible() {
        let new = sample_cx(1, vec![anchor(AnchorKind::StyleHold, vector_at_cos(1.0))]);
        let existing = sample_cx(2, vec![anchor(AnchorKind::StyleHold, vector_at_cos(0.85))]);

        assert_eq!(
            check_anchor_conflict(&new, &existing),
            AnchorConflictResult::Compatible
        );
    }

    #[test]
    fn exclusive_label_mismatch_conflicts() {
        let kind = AnchorKind::Label("exclusive_tag".to_string());
        let new = sample_cx(1, vec![anchor(kind.clone(), text("tag-a"))]);
        let existing = sample_cx(2, vec![anchor(kind.clone(), text("tag-b"))]);

        assert_eq!(
            check_anchor_conflict(&new, &existing),
            AnchorConflictResult::Conflicting {
                anchor_type: kind,
                reason: ConflictReason::ExclusiveTag,
            }
        );
    }

    proptest! {
        #[test]
        fn identical_speaker_anchors_are_compatible(label in "[a-z]{1,16}") {
            let left = sample_cx(1, vec![anchor(AnchorKind::SpeakerMatch, text(&label))]);
            let right = sample_cx(2, vec![anchor(AnchorKind::SpeakerMatch, text(&label))]);

            prop_assert_eq!(check_anchor_conflict(&left, &right), AnchorConflictResult::Compatible);
        }
    }

    fn sample_cx(seed: u8, anchors: Vec<Anchor>) -> Constellation {
        Constellation {
            cx_id: CxId::from_bytes([seed; 16]),
            vault_id: vault_id(),
            panel_version: 41,
            created_at: u64::from(seed),
            input_ref: InputRef {
                hash: [seed; 32],
                pointer: Some(format!("synthetic://ph41/anchor-conflict/{seed}")),
                redacted: false,
            },
            modality: Modality::Text,
            slots: BTreeMap::from([(
                calyx_core::SlotId::new(0),
                SlotVector::Dense {
                    dim: 2,
                    data: vec![1.0, 0.0],
                },
            )]),
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
            source: "synthetic-anchor-conflict".to_string(),
            observed_at: 1,
            confidence: 1.0,
        }
    }

    fn text(value: &str) -> AnchorValue {
        AnchorValue::Text(value.to_string())
    }

    fn vector_at_cos(cos: f32) -> AnchorValue {
        AnchorValue::Vector(vec![cos, (1.0 - cos * cos).sqrt()])
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
    }
}
