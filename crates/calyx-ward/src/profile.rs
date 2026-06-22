//! Guard profile configuration shared by Ward guard calls.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use calyx_core::{Clock, GuardTauProfile, SlotId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identifier for a guard profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GuardId(Uuid);

impl GuardId {
    /// Builds a guard id from a UUID.
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    /// Returns the wrapped UUID.
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl From<Uuid> for GuardId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl fmt::Display for GuardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for GuardId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Required-slot pass policy for a guard profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardPolicy {
    /// Every required slot must pass its per-slot tau.
    AllRequired,
    /// At least `k` required slots must pass.
    KofN { k: usize },
}

/// Action to take when an input lands outside the calibrated region.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoveltyAction {
    NewRegion,
    Quarantine,
    RejectClosed,
}

/// Calibration provenance attached to a guard profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationMeta {
    pub corpus_hash: [u8; 32],
    pub estimator: String,
    pub far: f32,
    pub frr: f32,
    pub confidence: f32,
    pub ts: i64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_slot: BTreeMap<SlotId, SlotCalibrationMeta>,
}

/// Per-slot calibration bounds preserved under a profile-level summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotCalibrationMeta {
    pub corpus_hash: [u8; 32],
    pub estimator: String,
    pub far: f32,
    pub frr: f32,
    pub confidence: f32,
    pub ts: i64,
}

impl CalibrationMeta {
    /// Builds calibration metadata from an injected Calyx clock.
    pub fn new(
        corpus_hash: [u8; 32],
        estimator: impl Into<String>,
        far: f32,
        frr: f32,
        confidence: f32,
        clock: &dyn Clock,
    ) -> Self {
        Self {
            corpus_hash,
            estimator: estimator.into(),
            far,
            frr,
            confidence,
            ts: clock_ts_i64(clock),
            per_slot: BTreeMap::new(),
        }
    }
}

impl SlotCalibrationMeta {
    pub fn from_calibration(meta: &CalibrationMeta) -> Self {
        Self {
            corpus_hash: meta.corpus_hash,
            estimator: meta.estimator.clone(),
            far: meta.far,
            frr: meta.frr,
            confidence: meta.confidence,
            ts: meta.ts,
        }
    }
}

fn clock_ts_i64(clock: &dyn Clock) -> i64 {
    i64::try_from(clock.now()).unwrap_or(i64::MAX)
}

/// Configuration object read by Ward guard calls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuardProfile {
    pub guard_id: GuardId,
    pub panel_version: u64,
    pub domain: String,
    pub tau: BTreeMap<SlotId, f32>,
    pub required_slots: Vec<SlotId>,
    pub policy: GuardPolicy,
    pub calibration: Option<CalibrationMeta>,
    pub novelty_action: NoveltyAction,
}

impl GuardProfile {
    /// Returns true when calibration provenance is attached.
    pub fn is_calibrated(&self) -> bool {
        self.calibration.is_some()
    }

    /// Returns the tau for `slot`; `None` means the slot is not guarded.
    pub fn tau_for(&self, slot: &SlotId) -> Option<f32> {
        self.tau.get(slot).copied()
    }
}

impl GuardTauProfile for GuardProfile {
    fn tau_for(&self, slot: &SlotId) -> Option<f32> {
        GuardProfile::tau_for(self, slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::FixedClock;
    use proptest::prelude::*;

    const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

    #[test]
    fn profile_tau_lookup_is_explicit() {
        let profile = sample_profile(GuardPolicy::AllRequired, None, vec![slot(1), slot(2)]);

        assert_eq!(profile.tau_for(&slot(1)), Some(0.80));
        assert_eq!(profile.tau_for(&slot(2)), Some(0.65));
        assert_eq!(profile.tau_for(&slot(3)), None);
        assert!(!profile.is_calibrated());
    }

    proptest! {
        #[test]
        fn profile_json_roundtrips(
            uuid in any::<u128>(),
            panel_version in any::<u64>(),
            domain in "[a-z]{1,12}",
            estimator in "[a-z]{1,12}",
            first_slot in any::<u16>(),
            second_slot in any::<u16>(),
            first_tau in 0.0f32..1.0f32,
            second_tau in 0.0f32..1.0f32,
            far in 0.0f32..1.0f32,
            frr in 0.0f32..1.0f32,
            confidence in 0.0f32..1.0f32,
            k in 0usize..8,
        ) {
            let mut tau = BTreeMap::new();
            tau.insert(SlotId::new(first_slot), first_tau);
            tau.insert(SlotId::new(second_slot), second_tau);
            let profile = GuardProfile {
                guard_id: GuardId::new(Uuid::from_u128(uuid)),
                panel_version,
                domain,
                tau,
                required_slots: vec![SlotId::new(first_slot), SlotId::new(second_slot)],
                policy: GuardPolicy::KofN { k },
                calibration: Some(CalibrationMeta {
                    corpus_hash: [7; 32],
                    estimator,
                    far,
                    frr,
                    confidence,
                    ts: 1_785_400_000,
                    per_slot: BTreeMap::new(),
                }),
                novelty_action: NoveltyAction::Quarantine,
            };

            let json = serde_json::to_string(&profile).expect("serialize profile");
            let decoded: GuardProfile = serde_json::from_str(&json).expect("deserialize profile");

            prop_assert_eq!(decoded, profile);
        }
    }

    #[test]
    fn kofn_zero_roundtrips() {
        let profile = sample_profile(GuardPolicy::KofN { k: 0 }, None, vec![slot(1)]);
        let decoded = roundtrip(&profile);

        assert_eq!(decoded.policy, GuardPolicy::KofN { k: 0 });
    }

    #[test]
    fn calibration_meta_uses_injected_clock_and_marks_profile_calibrated() {
        let clock = FixedClock::new(1_785_400_000);
        let calibration = CalibrationMeta::new([1; 32], "conformal", 0.0, 1.0, 0.99, &clock);
        let profile = sample_profile(
            GuardPolicy::AllRequired,
            Some(calibration.clone()),
            vec![slot(1), slot(2)],
        );
        let decoded = roundtrip(&profile);

        assert!(decoded.is_calibrated());
        assert_eq!(calibration.ts, 1_785_400_000);
        assert_eq!(decoded.calibration, Some(calibration));
    }

    #[test]
    fn empty_required_slots_serialize_without_panic() {
        let profile = sample_profile(GuardPolicy::AllRequired, None, Vec::new());
        let decoded = roundtrip(&profile);

        assert!(decoded.required_slots.is_empty());
        assert_eq!(decoded.novelty_action, NoveltyAction::NewRegion);
    }

    #[test]
    fn guard_id_display_and_parse_are_uuid_stable() {
        let parsed = GUARD_UUID.parse::<GuardId>().expect("parse guard id");

        assert_eq!(parsed.to_string(), GUARD_UUID);
        assert_eq!(GuardId::new(parsed.as_uuid()), parsed);
    }

    #[test]
    #[ignore = "manual FSV fixture; set CALYX_WARD_PROFILE_FSV_DIR"]
    fn profile_json_fsv_fixture_writes_readback_artifacts() {
        let root = std::env::var("CALYX_WARD_PROFILE_FSV_DIR")
            .expect("CALYX_WARD_PROFILE_FSV_DIR is required");
        std::fs::create_dir_all(&root).expect("create fsv root");

        let clock = FixedClock::new(1_785_400_000);
        let calibrated = CalibrationMeta::new([3; 32], "conformal", 0.0, 1.0, 0.99, &clock);
        let cases = [
            (
                "happy",
                sample_profile(GuardPolicy::AllRequired, None, vec![slot(1), slot(2)]),
            ),
            (
                "edge-kofn0",
                sample_profile(GuardPolicy::KofN { k: 0 }, None, vec![slot(1)]),
            ),
            (
                "edge-calibrated",
                sample_profile(
                    GuardPolicy::AllRequired,
                    Some(calibrated),
                    vec![slot(1), slot(2)],
                ),
            ),
            (
                "edge-empty-required",
                sample_profile(GuardPolicy::AllRequired, None, Vec::new()),
            ),
        ];

        for (name, profile) in cases {
            let path = std::path::Path::new(&root).join(format!("{name}.json"));
            let file = std::fs::File::create(&path).expect("create fsv json");
            serde_json::to_writer_pretty(file, &profile).expect("write fsv json");
            let file = std::fs::File::open(&path).expect("open fsv json");
            let decoded: GuardProfile = serde_json::from_reader(file).expect("read fsv json");

            assert_eq!(decoded, profile);
            println!(
                "FSV_CASE={name} ROUNDTRIP_EQUAL=true CALIBRATED={} TAU_SLOT1={:?} REQUIRED_SLOTS={}",
                decoded.is_calibrated(),
                decoded.tau_for(&slot(1)),
                decoded.required_slots.len()
            );
        }
    }

    fn sample_profile(
        policy: GuardPolicy,
        calibration: Option<CalibrationMeta>,
        required_slots: Vec<SlotId>,
    ) -> GuardProfile {
        let mut tau = BTreeMap::new();
        tau.insert(slot(1), 0.80);
        tau.insert(slot(2), 0.65);
        GuardProfile {
            guard_id: GUARD_UUID.parse().expect("sample guard id"),
            panel_version: 42,
            domain: "synthetic".to_string(),
            tau,
            required_slots,
            policy,
            calibration,
            novelty_action: NoveltyAction::NewRegion,
        }
    }

    fn roundtrip(profile: &GuardProfile) -> GuardProfile {
        let json = serde_json::to_string(profile).expect("serialize profile");
        serde_json::from_str(&json).expect("deserialize profile")
    }

    const fn slot(value: u16) -> SlotId {
        SlotId::new(value)
    }
}
