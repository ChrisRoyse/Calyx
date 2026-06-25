//! Panel and slot declarations.

use std::collections::BTreeMap;

use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    AnchorKind, Asymmetry, LensId, Modality, QuantPolicy, SlotId, SlotKey, SlotShape, SlotState,
};

use super::{LedgerRef, Signal, Ts};

/// Measured cost for a frozen lens in a slot.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensCost {
    /// Wall-clock profile cost over the probe batch.
    #[serde(default)]
    pub total_ms: f32,
    /// Wall-clock profile cost per probe input.
    #[serde(default)]
    pub ms_per_input: f32,
    /// Resident GPU memory required by the lens.
    #[serde(default)]
    pub vram_bytes: u64,
    /// Resident CPU memory required by the lens.
    #[serde(default)]
    pub ram_bytes: u64,
    /// Largest admitted batch size under the measured cost envelope.
    #[serde(default)]
    pub batch_ceiling: u32,
}

impl LensCost {
    pub fn zero() -> Self {
        Self {
            total_ms: 0.0,
            ms_per_input: 0.0,
            vram_bytes: 0,
            ram_bytes: 0,
            batch_ceiling: u32::MAX,
        }
    }

    pub fn is_zero_cost(&self) -> bool {
        self.vram_bytes == 0
            && self.ram_bytes == 0
            && self.total_ms == 0.0
            && self.ms_per_input == 0.0
    }
}

impl Default for LensCost {
    fn default() -> Self {
        Self::zero()
    }
}

/// Runtime placement for a frozen lens slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Cpu,
    Gpu,
}

/// Persisted resource policy for a slot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlotResource {
    /// Real measured lens cost.
    #[serde(default)]
    pub cost: LensCost,
    /// CPU/GPU placement selected from the runtime and admission budget.
    #[serde(default)]
    pub placement: Placement,
}

impl SlotResource {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

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
    /// Persisted cost and placement selected at admission time.
    #[serde(default, skip_serializing_if = "SlotResource::is_default")]
    pub resource: SlotResource,
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

impl Slot {
    /// Returns true when an absent value in this slot means the primary content
    /// measurement degraded for the current input.
    pub fn counts_toward_degraded(&self, input_modality: Modality) -> bool {
        self.state == SlotState::Active && self.modality == input_modality && !self.retrieval_only
    }
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
