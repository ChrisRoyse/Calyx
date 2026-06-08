//! Panel and slot declarations.

use std::collections::BTreeMap;

use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    AnchorKind, Asymmetry, LensId, Modality, QuantPolicy, SlotId, SlotKey, SlotShape, SlotState,
};

use super::{LedgerRef, Signal, Ts};

/// A frozen lens slot in a panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// Compact panel slot id.
    pub slot_id: SlotId,
    /// Stable human-readable slot key paired with the id.
    pub slot_key: SlotKey,
    /// Frozen lens content id.
    pub lens_id: LensId,
    /// Physical vector shape produced by this slot.
    pub shape: SlotShape,
    /// Modality measured by this slot.
    pub modality: Modality,
    /// Directional relationship for asymmetric slots.
    pub asymmetry: Asymmetry,
    /// Quantization policy.
    pub quant: QuantPolicy,
    /// Optional semantic axis/grouping tag.
    pub axis: Option<String>,
    /// Slot participates only as a post-retrieval signal, not primary recall.
    #[serde(default)]
    pub retrieval_only: bool,
    /// Slot must not drive deduplication decisions.
    #[serde(default)]
    pub excluded_from_dedup: bool,
    /// Assay signal by grounded outcome axis.
    #[serde(with = "anchor_signal_map")]
    pub bits_about: BTreeMap<AnchorKind, Signal>,
    /// Slot lifecycle state.
    pub state: SlotState,
    /// Panel version that introduced this slot.
    pub added_at_panel_version: u32,
}

/// Versioned panel of slots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Panel {
    /// Panel version.
    pub version: u32,
    /// Slots active or historically interpretable in this panel.
    pub slots: Vec<Slot>,
    /// Server-stamped creation timestamp.
    pub created_at: Ts,
    /// Ledger ref for the grounding kernel used with this panel.
    pub kernel_ref: Option<LedgerRef>,
    /// Ledger ref for the guard calibration used with this panel.
    pub guard_ref: Option<LedgerRef>,
}

mod anchor_signal_map {
    use super::*;

    pub fn serialize<S>(
        map: &BTreeMap<AnchorKind, Signal>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let keyed: BTreeMap<String, &Signal> = map
            .iter()
            .map(|(kind, signal)| (encode_key(kind), signal))
            .collect();
        keyed.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<BTreeMap<AnchorKind, Signal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let keyed = BTreeMap::<String, Signal>::deserialize(deserializer)?;
        let mut map = BTreeMap::new();
        for (key, signal) in keyed {
            let kind = decode_key(&key).map_err(de::Error::custom)?;
            if map.insert(kind, signal).is_some() {
                return Err(de::Error::custom("duplicate anchor kind in bits_about"));
            }
        }
        Ok(map)
    }

    fn encode_key(kind: &AnchorKind) -> String {
        match kind {
            AnchorKind::TestPass => "test_pass".to_string(),
            AnchorKind::TieFormed => "tie_formed".to_string(),
            AnchorKind::Thumbs => "thumbs".to_string(),
            AnchorKind::Label(value) => format!("label:{value}"),
            AnchorKind::Reward => "reward".to_string(),
            AnchorKind::SpeakerMatch => "speaker_match".to_string(),
            AnchorKind::StyleHold => "style_hold".to_string(),
            AnchorKind::Recurrence => "recurrence".to_string(),
        }
    }

    fn decode_key(value: &str) -> Result<AnchorKind, String> {
        Ok(match value {
            "test_pass" => AnchorKind::TestPass,
            "tie_formed" => AnchorKind::TieFormed,
            "thumbs" => AnchorKind::Thumbs,
            "reward" => AnchorKind::Reward,
            "speaker_match" => AnchorKind::SpeakerMatch,
            "style_hold" => AnchorKind::StyleHold,
            "recurrence" => AnchorKind::Recurrence,
            label if label.starts_with("label:") => {
                AnchorKind::Label(label["label:".len()..].to_string())
            }
            other => return Err(format!("unknown anchor kind key `{other}`")),
        })
    }
}
