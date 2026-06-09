//! Structured verdicts emitted by Ward guard calls.

use calyx_core::SlotId;
use serde::{Deserialize, Serialize};

use crate::profile::{GuardId, NoveltyAction};

/// Per-slot cosine decision for a guarded output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotVerdict {
    pub slot: SlotId,
    pub cos: f32,
    pub tau: f32,
    pub pass: bool,
}

/// Aggregate guard decision with the full per-slot decomposition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GuardVerdict {
    pub guard_id: GuardId,
    pub overall_pass: bool,
    #[serde(default)]
    pub provisional: bool,
    pub per_slot: Vec<SlotVerdict>,
    pub action: Option<NoveltyAction>,
}

impl GuardVerdict {
    /// Returns every slot verdict that failed its per-slot tau.
    pub fn failing_slots(&self) -> Vec<&SlotVerdict> {
        self.per_slot.iter().filter(|slot| !slot.pass).collect()
    }

    /// Returns the full per-slot breakdown for pass and fail outcomes.
    pub fn all_slot_details(&self) -> &[SlotVerdict] {
        &self.per_slot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

    #[test]
    fn guard_verdict_keeps_all_slot_details_and_reports_failures() {
        let pass = SlotVerdict {
            slot: slot(1),
            cos: 0.91,
            tau: 0.70,
            pass: true,
        };
        let fail = SlotVerdict {
            slot: slot(2),
            cos: 0.40,
            tau: 0.70,
            pass: false,
        };
        let verdict = GuardVerdict {
            guard_id: guard_id(),
            overall_pass: false,
            provisional: false,
            per_slot: vec![pass.clone(), fail.clone()],
            action: Some(NoveltyAction::Quarantine),
        };

        assert_eq!(verdict.failing_slots(), vec![&fail]);
        assert_eq!(verdict.all_slot_details(), &[pass, fail]);
    }

    proptest! {
        #[test]
        fn slot_verdict_json_roundtrips(
            slot_id in any::<u16>(),
            cos in -1.0f32..1.0f32,
            tau in -1.0f32..1.0f32,
            pass in any::<bool>(),
        ) {
            let verdict = SlotVerdict {
                slot: SlotId::new(slot_id),
                cos,
                tau,
                pass,
            };

            let json = serde_json::to_string(&verdict).expect("serialize slot verdict");
            let decoded: SlotVerdict = serde_json::from_str(&json).expect("deserialize slot verdict");

            prop_assert_eq!(decoded, verdict);
        }
    }

    #[test]
    fn empty_per_slot_verdict_serializes_cleanly() {
        let verdict = GuardVerdict {
            guard_id: guard_id(),
            overall_pass: true,
            provisional: true,
            per_slot: Vec::new(),
            action: None,
        };
        let json = serde_json::to_string(&verdict).expect("serialize empty verdict");
        let decoded: GuardVerdict = serde_json::from_str(&json).expect("deserialize empty verdict");

        assert_eq!(decoded, verdict);
        assert!(decoded.provisional);
        assert!(decoded.failing_slots().is_empty());
        assert!(decoded.all_slot_details().is_empty());
    }

    fn guard_id() -> GuardId {
        GUARD_UUID.parse().expect("guard id")
    }

    const fn slot(value: u16) -> SlotId {
        SlotId::new(value)
    }
}
