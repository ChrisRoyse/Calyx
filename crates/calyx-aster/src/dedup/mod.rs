//! Vault-level deduplication policy contracts.

mod audit;
mod compression_ratio;
mod engine;
mod ingest_at;
mod ingest_event;
mod ingest_input;
mod ingest_ledger;
mod policy;
mod signature;

use calyx_core::{CalyxError, CxId, Panel, Result, Slot, SlotId};
use serde::{Deserialize, Serialize};

pub use audit::{
    CALYX_DEDUP_UNDO_EMPTY_TOKEN, CALYX_DEDUP_UNDO_MISSING_RESTORE, CALYX_DEDUP_WRONG_VAULT,
    DedupAuditReport, DedupRestoreSnapshot, DedupUndoRecord, MergeRecord, ReversalToken,
    dedup_audit, dedup_undo,
};
pub use compression_ratio::{
    CALYX_DEDUP_INVALID_FREQUENCY, CALYX_DEDUP_MISSING_FREQUENCY, CompressionRatio, Domain,
    DomainCompressionStats, compression_ratio, domain_compression_stats,
};
pub use engine::{
    DEFAULT_DEDUP_DPI_CANDIDATE_LIMIT, DedupDecision, check_dedup, check_dedup_with_limit,
    cosine_passes_all_required, resolve_tau,
};
pub use ingest_at::{ingest, ingest_at, ingest_at_with_retention};
pub use ingest_event::{
    DedupOnlineEvent, DedupOnlineKind, decode_dedup_online_event, dedup_online_key,
};
pub use ingest_input::{CALYX_DEDUP_INVALID_EVENT_TIME, EpochSecs, IngestInput};
pub(crate) use policy::is_recurrence_series_policy;
pub use policy::{
    ANCHOR_VECTOR_TAU, AnchorConflictResult, ConflictReason, ContestedWith, check_anchor_conflict,
    contested_with_key, decode_contested_with, encode_contested_with,
};
pub use signature::{
    CALYX_RECURRENCE_SLOT_MISSING, SignatureResult, detect_recurrence_signature,
    temporal_slot_ids_for_panel,
};

pub const CALYX_DEDUP_NO_REQUIRED_SLOTS: &str = "CALYX_DEDUP_NO_REQUIRED_SLOTS";
pub const CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED: &str = "CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED";
pub const CALYX_DEDUP_INVALID_TAU: &str = "CALYX_DEDUP_INVALID_TAU";
pub const CALYX_DEDUP_SLOT_NOT_IN_TAU: &str = "CALYX_DEDUP_SLOT_NOT_IN_TAU";
pub const CALYX_DEDUP_SLOT_NOT_IN_PANEL: &str = "CALYX_DEDUP_SLOT_NOT_IN_PANEL";
pub const CALYX_DEDUP_MISSING_GUARD_PROFILE: &str = "CALYX_DEDUP_MISSING_GUARD_PROFILE";
pub const CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION: &str = "CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION";
pub const CALYX_DEDUP_DPI_EXCEEDED: &str = "CALYX_DEDUP_DPI_EXCEEDED";
pub const CALYX_DEDUP_ANCHOR_CONFLICT: &str = "CALYX_DEDUP_ANCHOR_CONFLICT";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TauStrategy {
    PerSlot(Vec<(SlotId, f32)>),
    Calibrated,
}

impl TauStrategy {
    fn validate(&self, required_slots: &[SlotId]) -> Result<()> {
        if let Self::PerSlot(entries) = self {
            if entries.is_empty() {
                return Err(dedup_error(
                    CALYX_DEDUP_INVALID_TAU,
                    "per-slot tau strategy must include at least one threshold",
                ));
            }
            for (slot, tau) in entries {
                if !tau.is_finite() || !(-1.0..=1.0).contains(tau) {
                    return Err(dedup_error(
                        CALYX_DEDUP_INVALID_TAU,
                        format!("tau for slot {slot} must be finite and in -1.0..=1.0"),
                    ));
                }
            }
            for required in required_slots {
                if !entries.iter().any(|(slot, _)| slot == required) {
                    return Err(dedup_error(
                        CALYX_DEDUP_INVALID_TAU,
                        format!("required slot {required} is missing a per-slot tau"),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DedupAction {
    Collapse,
    Link,
    RecurrenceSeries,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TctCosineConfig {
    pub required_slots: Vec<SlotId>,
    pub tau: TauStrategy,
    pub action: DedupAction,
}

impl TctCosineConfig {
    pub fn new(required_slots: Vec<SlotId>, tau: TauStrategy, action: DedupAction) -> Result<Self> {
        let config = Self {
            required_slots,
            tau,
            action,
        };
        config.validate_static()?;
        Ok(config)
    }

    pub fn validate(&self, panel: &Panel) -> Result<()> {
        self.validate_static()?;
        for required in &self.required_slots {
            let Some(slot) = panel.slots.iter().find(|slot| slot.slot_id == *required) else {
                return Err(dedup_error(
                    CALYX_DEDUP_SLOT_NOT_IN_PANEL,
                    format!("required slot {required} is missing from the active panel"),
                ));
            };
            if slot_excluded_from_dedup(slot) {
                return Err(dedup_error(
                    CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED,
                    format!("required slot {required} maps to a temporal or dedup-excluded lens"),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn validate_static(&self) -> Result<()> {
        if self.required_slots.is_empty() {
            return Err(dedup_error(
                CALYX_DEDUP_NO_REQUIRED_SLOTS,
                "TctCosine dedup requires at least one content slot",
            ));
        }
        self.tau.validate(&self.required_slots)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum DedupPolicy {
    #[default]
    Off,
    Exact,
    TctCosine(TctCosineConfig),
}

impl DedupPolicy {
    pub fn validate(&self, panel: &Panel) -> Result<()> {
        match self {
            Self::Off | Self::Exact => Ok(()),
            Self::TctCosine(config) => config.validate(panel),
        }
    }

    pub(crate) fn validate_manifest(&self) -> Result<()> {
        match self {
            Self::Off | Self::Exact => Ok(()),
            Self::TctCosine(config) => config.validate_static(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OccurrenceId(pub u64);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DedupResult {
    New(CxId),
    DedupMerge {
        into: CxId,
        occurrence: OccurrenceId,
    },
    ExactDuplicate(CxId),
}

fn slot_excluded_from_dedup(slot: &Slot) -> bool {
    slot.excluded_from_dedup || slot.retrieval_only || signature::is_temporal_slot(slot)
}

pub(crate) fn dedup_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    let remediation = match code {
        CALYX_DEDUP_NO_REQUIRED_SLOTS => "choose at least one required content slot",
        CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED => {
            "remove E2/E3/E4 temporal lenses from required dedup slots"
        }
        CALYX_DEDUP_INVALID_TAU => "set finite cosine thresholds in -1.0..=1.0",
        CALYX_DEDUP_SLOT_NOT_IN_TAU => "add a threshold for every required dedup slot",
        CALYX_DEDUP_SLOT_NOT_IN_PANEL => {
            "add the required slot to the active panel or remove it from required_slots"
        }
        CALYX_DEDUP_MISSING_GUARD_PROFILE => {
            "provide a calibrated guard profile with tau for each required slot"
        }
        CALYX_DEDUP_SLOT_NOT_IN_CONSTELLATION => {
            "ensure every required content slot has a dense vector on both constellations"
        }
        CALYX_DEDUP_DPI_EXCEEDED => "reduce the candidate set or use Exact dedup policy",
        CALYX_DEDUP_ANCHOR_CONFLICT => "keep conflicting anchors as separate contested regions",
        CALYX_DEDUP_INVALID_EVENT_TIME => "use a non-negative Unix epoch timestamp in seconds",
        CALYX_DEDUP_MISSING_FREQUENCY => {
            "write recurrence.frequency to the Base CF before reading recurrence consumers"
        }
        CALYX_DEDUP_INVALID_FREQUENCY => {
            "store recurrence.frequency as a finite non-negative integer scalar"
        }
        CALYX_DEDUP_WRONG_VAULT => "apply the reversal token to the vault that produced it",
        CALYX_DEDUP_UNDO_MISSING_RESTORE => {
            "use a merge ledger entry that contains a dedup restore snapshot"
        }
        CALYX_DEDUP_UNDO_EMPTY_TOKEN => "use a reversal token with at least one snapshot CxId",
        _ => "inspect dedup policy",
    };
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{Asymmetry, LensId, Modality, QuantPolicy, SlotKey, SlotShape, SlotState};
    use std::collections::BTreeMap;

    #[test]
    fn tct_cosine_accepts_present_required_content_slots() {
        sample_tct_config()
            .validate(&sample_panel())
            .expect("present content slots validate");
    }

    #[test]
    fn tct_cosine_rejects_temporal_required_slot() {
        let panel = sample_panel();
        let policy = DedupPolicy::TctCosine(TctCosineConfig {
            required_slots: vec![SlotId::new(5)],
            tau: TauStrategy::Calibrated,
            action: DedupAction::RecurrenceSeries,
        });

        let error = policy.validate(&panel).expect_err("temporal slot rejected");

        assert_eq!(error.code, CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED);
    }

    #[test]
    fn tct_cosine_rejects_temporal_prefix_required_slot() {
        let panel = Panel {
            version: 9,
            slots: vec![slot(6, "E2_custom", false, false)],
            created_at: 1_786_320_000,
            kernel_ref: None,
            guard_ref: None,
        };
        let policy = DedupPolicy::TctCosine(TctCosineConfig {
            required_slots: vec![SlotId::new(6)],
            tau: TauStrategy::Calibrated,
            action: DedupAction::RecurrenceSeries,
        });

        let error = policy.validate(&panel).expect_err("E2 prefix rejected");

        assert_eq!(error.code, CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED);
    }

    #[test]
    fn tct_cosine_rejects_dedup_excluded_required_slot() {
        let panel = sample_panel();
        let policy = DedupPolicy::TctCosine(TctCosineConfig {
            required_slots: vec![SlotId::new(2)],
            tau: TauStrategy::Calibrated,
            action: DedupAction::Link,
        });

        let error = policy
            .validate(&panel)
            .expect_err("dedup-excluded slot rejected");

        assert_eq!(error.code, CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED);
    }

    #[test]
    fn tct_cosine_rejects_missing_required_slot() {
        let panel = sample_panel();
        let policy = DedupPolicy::TctCosine(TctCosineConfig {
            required_slots: vec![SlotId::new(9)],
            tau: TauStrategy::Calibrated,
            action: DedupAction::Link,
        });

        let error = policy
            .validate(&panel)
            .expect_err("missing required slot rejected");

        assert_eq!(error.code, CALYX_DEDUP_SLOT_NOT_IN_PANEL);
    }

    #[test]
    fn tct_cosine_rejects_empty_required_slots() {
        let panel = sample_panel();
        let policy = DedupPolicy::TctCosine(TctCosineConfig {
            required_slots: Vec::new(),
            tau: TauStrategy::Calibrated,
            action: DedupAction::Link,
        });

        let error = policy
            .validate(&panel)
            .expect_err("empty required rejected");

        assert_eq!(error.code, CALYX_DEDUP_NO_REQUIRED_SLOTS);
    }

    #[test]
    fn off_policy_validates_for_any_panel() {
        DedupPolicy::Off.validate(&sample_panel()).expect("off ok");
    }

    #[test]
    fn dedup_policy_json_roundtrips_byte_exact() {
        for policy in [
            DedupPolicy::Off,
            DedupPolicy::Exact,
            DedupPolicy::TctCosine(sample_tct_config()),
        ] {
            let first = serde_json::to_vec(&policy).expect("serialize policy");
            let decoded: DedupPolicy = serde_json::from_slice(&first).expect("deserialize policy");
            let second = serde_json::to_vec(&decoded).expect("serialize decoded");

            assert_eq!(first, second);
            assert_eq!(policy, decoded);
        }
    }

    #[test]
    fn tau_strategy_roundtrips_byte_exact() {
        for strategy in [
            TauStrategy::PerSlot(vec![(SlotId::new(1), 0.75), (SlotId::new(2), 0.875)]),
            TauStrategy::Calibrated,
        ] {
            let first = serde_json::to_vec(&strategy).expect("serialize tau");
            let decoded: TauStrategy = serde_json::from_slice(&first).expect("deserialize tau");
            let second = serde_json::to_vec(&decoded).expect("serialize decoded");

            assert_eq!(first, second);
            assert_eq!(strategy, decoded);
        }
    }

    #[test]
    fn recurrence_series_action_serializes_as_string() {
        let bytes = serde_json::to_vec(&DedupAction::RecurrenceSeries).expect("serialize action");

        assert_eq!(bytes, br#""RecurrenceSeries""#);
    }

    #[test]
    fn dedup_result_roundtrips_byte_exact() {
        let result = DedupResult::DedupMerge {
            into: CxId::from_bytes([9; 16]),
            occurrence: OccurrenceId(42),
        };
        let first = serde_json::to_vec(&result).expect("serialize result");
        let decoded: DedupResult = serde_json::from_slice(&first).expect("deserialize result");

        assert_eq!(first, serde_json::to_vec(&decoded).unwrap());
        assert_eq!(result, decoded);
    }

    fn sample_tct_config() -> TctCosineConfig {
        TctCosineConfig::new(
            vec![SlotId::new(0), SlotId::new(1)],
            TauStrategy::PerSlot(vec![(SlotId::new(0), 0.91), (SlotId::new(1), 0.88)]),
            DedupAction::RecurrenceSeries,
        )
        .expect("valid tct config")
    }

    fn sample_panel() -> Panel {
        Panel {
            version: 8,
            slots: vec![
                slot(0, "E1_semantic", false, false),
                slot(1, "keyword_splade", false, false),
                slot(2, "E1_archive", false, true),
                slot(5, "E2_recency", true, true),
            ],
            created_at: 1_786_320_000,
            kernel_ref: None,
            guard_ref: None,
        }
    }

    fn slot(id: u16, key: &str, retrieval_only: bool, excluded_from_dedup: bool) -> Slot {
        let slot_id = SlotId::new(id);
        Slot {
            slot_id,
            slot_key: SlotKey::new(slot_id, key),
            lens_id: LensId::from_bytes([id as u8; 16]),
            shape: SlotShape::Dense(2),
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some(key.to_string()),
            retrieval_only,
            excluded_from_dedup,
            bits_about: BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: u32::from(id) + 1,
        }
    }
}
