//! Closed shared enum vocabulary for Calyx engines.

use serde::{Deserialize, Serialize};

use crate::SlotId;

/// Input modality for a constellation or lens slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    /// Natural language text.
    Text,
    /// Source code or structured code artifacts.
    Code,
    /// Still images.
    Image,
    /// Audio streams or clips.
    Audio,
    /// Video streams or clips.
    Video,
    /// Protein or peptide sequences.
    Protein,
    /// DNA or nucleotide sequences.
    Dna,
    /// Molecules encoded as SMILES or related chemistry strings.
    Molecule,
    /// Structured records, tables, or typed objects.
    Structured,
    /// Mixed or multi-modal inputs.
    Mixed,
}

/// Physical vector shape produced by a lens slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotShape {
    /// Dense vector with a fixed dimension.
    Dense(u32),
    /// Sparse vector with a fixed ambient dimension.
    Sparse(u32),
    /// Multi-vector token representation with a fixed per-token dimension.
    Multi { token_dim: u32 },
}

/// Directional relationship for asymmetric slot pairings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Asymmetry {
    /// Symmetric or direction-free interpretation.
    None,
    /// Directed dual-slot relation.
    Dual { a: SlotId, b: SlotId },
}

/// Quantization policy attached to a slot or vector block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantPolicy {
    /// Store unquantized values.
    None,
    /// TurboQuant storage with `bits_per_channel_x2` where 7 means 3.5 bpc.
    TurboQuant { bits_per_channel_x2: u8 },
    /// Blackwell microscaling FP4 storage, with MXFP8 used as the safe fallback.
    MxFp4,
    /// Product quantization with `m` subquantizers and `nbits` codebook bits.
    Pq { m: u8, nbits: u8 },
    /// Float8 storage.
    Float8,
    /// Binary storage.
    Binary,
}

impl QuantPolicy {
    /// Quality-neutral TurboQuant default from PRD 23 section 4.1.
    pub const fn turboquant_default() -> Self {
        Self::TurboQuant {
            bits_per_channel_x2: 7,
        }
    }
}

/// Grounded outcome axis for anchors and Assay bits.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorKind {
    /// Deterministic test or oracle passed.
    TestPass,
    /// A tie or relationship was formed.
    TieFormed,
    /// User thumb signal.
    Thumbs,
    /// Named label axis.
    Label(String),
    /// Reward or outcome score.
    Reward,
    /// Speaker identity match.
    SpeakerMatch,
    /// Style/persona hold under pressure.
    StyleHold,
    /// Recurring event or outcome series.
    Recurrence,
}

/// Lifecycle state for a panel slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotState {
    /// Slot participates in new ingests and reads.
    Active,
    /// Slot is parked but remains interpretable for old constellations.
    Parked,
    /// Slot is tombstoned for future use but historical data remains readable.
    Retired,
}

/// Explicit reason a slot vector is absent.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbsentReason {
    /// Slot does not apply to this input or modality.
    NotApplicable,
    /// Source input or output was redacted by policy.
    Redacted,
    /// Lens was unavailable at ingest time.
    LensUnavailable,
    /// Slot is intentionally deferred for lazy backfill.
    Deferred,
    /// Slot is absent because the producing lens is parked or retired.
    LensInactive,
    /// Slot production failed with a stable code or short reason.
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T>(value: T, expected_json: &[u8])
    where
        T: core::fmt::Debug + PartialEq + Serialize + for<'de> Deserialize<'de>,
    {
        let bytes = serde_json::to_vec(&value).expect("serialize enum");
        assert_eq!(bytes, expected_json);
        let decoded = serde_json::from_slice(&bytes).expect("deserialize enum");
        assert_eq!(value, decoded);
    }

    #[test]
    fn enum_serialization_bytes_are_stable() {
        roundtrip(Modality::Text, br#""text""#);
        roundtrip(SlotShape::Dense(768), br#"{"dense":768}"#);
        roundtrip(
            SlotShape::Multi { token_dim: 128 },
            br#"{"multi":{"token_dim":128}}"#,
        );
        roundtrip(
            Asymmetry::Dual {
                a: SlotId::new(1),
                b: SlotId::new(2),
            },
            br#"{"dual":{"a":1,"b":2}}"#,
        );
        roundtrip(
            QuantPolicy::Pq { m: 8, nbits: 4 },
            br#"{"pq":{"m":8,"nbits":4}}"#,
        );
        roundtrip(
            QuantPolicy::turboquant_default(),
            br#"{"turbo_quant":{"bits_per_channel_x2":7}}"#,
        );
        roundtrip(QuantPolicy::MxFp4, br#""mx_fp4""#);
        roundtrip(
            AnchorKind::Label("gold".to_string()),
            br#"{"label":"gold"}"#,
        );
        roundtrip(SlotState::Parked, br#""parked""#);
        roundtrip(
            AbsentReason::Error("CALYX_LENS_DIM_MISMATCH".to_string()),
            br#"{"error":"CALYX_LENS_DIM_MISMATCH"}"#,
        );
    }

    #[test]
    fn modality_variant_set_is_locked() {
        let names = [
            modality_name(Modality::Text),
            modality_name(Modality::Code),
            modality_name(Modality::Image),
            modality_name(Modality::Audio),
            modality_name(Modality::Video),
            modality_name(Modality::Protein),
            modality_name(Modality::Dna),
            modality_name(Modality::Molecule),
            modality_name(Modality::Structured),
            modality_name(Modality::Mixed),
        ];

        assert_eq!(
            names,
            [
                "text",
                "code",
                "image",
                "audio",
                "video",
                "protein",
                "dna",
                "molecule",
                "structured",
                "mixed"
            ]
        );
    }

    #[test]
    fn anchor_kind_variant_set_includes_identity_and_recurrence() {
        let names = [
            anchor_name(&AnchorKind::TestPass),
            anchor_name(&AnchorKind::TieFormed),
            anchor_name(&AnchorKind::Thumbs),
            anchor_name(&AnchorKind::Label("x".to_string())),
            anchor_name(&AnchorKind::Reward),
            anchor_name(&AnchorKind::SpeakerMatch),
            anchor_name(&AnchorKind::StyleHold),
            anchor_name(&AnchorKind::Recurrence),
        ];

        assert_eq!(
            names,
            [
                "test_pass",
                "tie_formed",
                "thumbs",
                "label",
                "reward",
                "speaker_match",
                "style_hold",
                "recurrence",
            ]
        );
    }

    #[test]
    fn all_enum_variant_sets_are_exhaustively_matched() {
        assert_eq!(slot_shape_name(&SlotShape::Sparse(42)), "sparse");
        assert_eq!(
            asymmetry_name(&Asymmetry::Dual {
                a: SlotId::new(1),
                b: SlotId::new(2),
            }),
            "dual"
        );
        assert_eq!(quant_name(&QuantPolicy::Float8), "float8");
        assert_eq!(slot_state_name(SlotState::Retired), "retired");
        assert_eq!(
            absent_reason_name(&AbsentReason::LensUnavailable),
            "lens_unavailable"
        );
    }

    fn modality_name(value: Modality) -> &'static str {
        match value {
            Modality::Text => "text",
            Modality::Code => "code",
            Modality::Image => "image",
            Modality::Audio => "audio",
            Modality::Video => "video",
            Modality::Protein => "protein",
            Modality::Dna => "dna",
            Modality::Molecule => "molecule",
            Modality::Structured => "structured",
            Modality::Mixed => "mixed",
        }
    }

    fn slot_shape_name(value: &SlotShape) -> &'static str {
        match value {
            SlotShape::Dense(_) => "dense",
            SlotShape::Sparse(_) => "sparse",
            SlotShape::Multi { .. } => "multi",
        }
    }

    fn asymmetry_name(value: &Asymmetry) -> &'static str {
        match value {
            Asymmetry::None => "none",
            Asymmetry::Dual { .. } => "dual",
        }
    }

    fn quant_name(value: &QuantPolicy) -> &'static str {
        match value {
            QuantPolicy::None => "none",
            QuantPolicy::TurboQuant { .. } => "turbo_quant",
            QuantPolicy::MxFp4 => "mx_fp4",
            QuantPolicy::Pq { .. } => "pq",
            QuantPolicy::Float8 => "float8",
            QuantPolicy::Binary => "binary",
        }
    }

    fn anchor_name(value: &AnchorKind) -> &'static str {
        match value {
            AnchorKind::TestPass => "test_pass",
            AnchorKind::TieFormed => "tie_formed",
            AnchorKind::Thumbs => "thumbs",
            AnchorKind::Label(_) => "label",
            AnchorKind::Reward => "reward",
            AnchorKind::SpeakerMatch => "speaker_match",
            AnchorKind::StyleHold => "style_hold",
            AnchorKind::Recurrence => "recurrence",
        }
    }

    fn slot_state_name(value: SlotState) -> &'static str {
        match value {
            SlotState::Active => "active",
            SlotState::Parked => "parked",
            SlotState::Retired => "retired",
        }
    }

    fn absent_reason_name(value: &AbsentReason) -> &'static str {
        match value {
            AbsentReason::NotApplicable => "not_applicable",
            AbsentReason::Redacted => "redacted",
            AbsentReason::LensUnavailable => "lens_unavailable",
            AbsentReason::Deferred => "deferred",
            AbsentReason::LensInactive => "lens_inactive",
            AbsentReason::Error(_) => "error",
        }
    }
}
