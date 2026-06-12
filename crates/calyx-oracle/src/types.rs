//! Public Oracle contract types for consequence prediction.

use std::fmt;

use calyx_core::{AnchorValue, LedgerRef, LensId};
use calyx_ward::GuardVerdict;
use serde::{Deserialize, Serialize};

pub const DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DomainId(String);

impl DomainId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DomainId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<&str> for DomainId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for DomainId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    pub outcome: AnchorValue,
    pub confidence: f32,
    pub consequences: Vec<Consequence>,
    pub bound: SufficiencyBound,
    pub provenance: LedgerRef,
    pub guard: GuardVerdict,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SufficiencyBound {
    #[serde(rename = "I_panel_oracle")]
    pub i_panel_oracle: f32,
    pub dpi_ceiling: f32,
    pub sufficient: bool,
    pub per_sensor_deficit: Vec<(LensId, f32)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OracleSelfConsistency {
    pub flakiness: f32,
    pub validity: f32,
    pub ceiling: f32,
}

impl OracleSelfConsistency {
    pub fn measured(flakiness: f32, validity: f32) -> Self {
        Self {
            flakiness,
            validity,
            ceiling: validity * (1.0 - flakiness),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Consequence {
    pub action_or_event: String,
    pub domain: DomainId,
    pub outcome: AnchorValue,
    pub confidence: f32,
    pub hop: u8,
    pub provenance: LedgerRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsequenceTree {
    pub root: Consequence,
    pub children: Vec<ConsequenceTree>,
    pub max_depth: u8,
}

impl ConsequenceTree {
    pub fn leaf(root: Consequence) -> Self {
        Self {
            root,
            children: Vec::new(),
            max_depth: DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::str::FromStr;

    use calyx_core::SlotId;
    use calyx_ward::{GuardId, NoveltyAction, SlotVerdict};
    use proptest::prelude::*;
    use serde::Serialize;

    use super::*;
    use crate::{
        CALYX_ORACLE_FLAKY_ANCHOR, CALYX_ORACLE_INSUFFICIENT, CALYX_ORACLE_NO_RECURRENCE,
        OracleError,
    };

    #[test]
    fn prediction_json_roundtrips_with_known_fields() {
        let prediction = prediction_fixture();

        let json = serde_json::to_string(&prediction).expect("serialize prediction");
        let decoded: Prediction = serde_json::from_str(&json).expect("deserialize prediction");

        assert_eq!(decoded, prediction);
        assert!(json.contains("\"I_panel_oracle\":1.05"));
    }

    #[test]
    fn self_consistency_ceiling_matches_known_values() {
        assert_close(OracleSelfConsistency::measured(0.0, 1.0).ceiling, 1.0);
        assert_close(OracleSelfConsistency::measured(0.1, 0.8).ceiling, 0.72);
        assert_close(OracleSelfConsistency::measured(0.5, 0.5).ceiling, 0.25);
    }

    proptest! {
        #[test]
        fn self_consistency_ceiling_stays_in_unit_interval(
            flakiness in 0.0f32..=1.0,
            validity in 0.0f32..=1.0,
        ) {
            let consistency = OracleSelfConsistency::measured(flakiness, validity);

            prop_assert!(consistency.ceiling >= 0.0);
            prop_assert!(consistency.ceiling <= 1.0);
        }
    }

    #[test]
    fn empty_per_sensor_deficit_still_serializes() {
        let bound = SufficiencyBound {
            i_panel_oracle: 0.0,
            dpi_ceiling: 0.0,
            sufficient: false,
            per_sensor_deficit: Vec::new(),
        };

        let json = serde_json::to_string(&bound).expect("serialize bound");
        let decoded: SufficiencyBound = serde_json::from_str(&json).expect("deserialize bound");

        assert_eq!(decoded, bound);
        assert!(json.contains("\"per_sensor_deficit\":[]"));
    }

    #[test]
    fn root_consequence_keeps_hop_zero() {
        let root = consequence(2, "root-action", 0, 0.9);

        assert_eq!(root.hop, 0);
    }

    #[test]
    fn max_depth_zero_allows_empty_tree() {
        let tree = ConsequenceTree {
            root: consequence(3, "terminal", 0, 0.6),
            children: Vec::new(),
            max_depth: 0,
        };

        assert_eq!(tree.max_depth, 0);
        assert!(tree.children.is_empty());
    }

    #[test]
    fn oracle_error_display_contains_codes_and_remediation() {
        let insufficient = OracleError::Insufficient {
            bound: SufficiencyBound {
                i_panel_oracle: 0.46,
                dpi_ceiling: 0.46,
                sufficient: false,
                per_sensor_deficit: Vec::new(),
            },
        };
        let flaky = OracleError::FlakyAnchor {
            self_consistency: 0.25,
        };
        let recurrence = OracleError::NoRecurrence {
            domain: DomainId::from("fixture"),
        };

        assert_display_has_code_and_remediation(&insufficient, CALYX_ORACLE_INSUFFICIENT);
        assert_display_has_code_and_remediation(&flaky, CALYX_ORACLE_FLAKY_ANCHOR);
        assert_display_has_code_and_remediation(&recurrence, CALYX_ORACLE_NO_RECURRENCE);
    }

    #[test]
    #[ignore = "manual aiwonder FSV for issue #429 Oracle contract readbacks"]
    fn issue429_oracle_types_fsv_writes_readbacks() {
        let root = std::env::var_os("CALYX_ORACLE_TYPES_FSV_DIR")
            .map(std::path::PathBuf::from)
            .expect("set CALYX_ORACLE_TYPES_FSV_DIR");
        fs::create_dir_all(&root).expect("create oracle types fsv root");

        let prediction = prediction_fixture();
        write_json(&root.join("prediction.json"), &prediction);
        let decoded: Prediction = serde_json::from_slice(
            &fs::read(root.join("prediction.json")).expect("read prediction"),
        )
        .expect("decode prediction");
        write_json(&root.join("prediction-roundtrip.json"), &decoded);

        write_json(
            &root.join("edge-empty-deficit.json"),
            &SufficiencyBound {
                i_panel_oracle: 0.0,
                dpi_ceiling: 0.0,
                sufficient: false,
                per_sensor_deficit: Vec::new(),
            },
        );
        write_json(
            &root.join("edge-hop-zero.json"),
            &consequence(2, "root-action", 0, 0.9),
        );
        write_json(
            &root.join("edge-max-depth-zero.json"),
            &ConsequenceTree {
                root: consequence(3, "terminal", 0, 0.6),
                children: Vec::new(),
                max_depth: 0,
            },
        );
        fs::write(
            root.join("oracle-error-catalog.txt"),
            [
                CALYX_ORACLE_INSUFFICIENT,
                CALYX_ORACLE_FLAKY_ANCHOR,
                CALYX_ORACLE_NO_RECURRENCE,
            ]
            .join("\n"),
        )
        .expect("write oracle catalog");
    }

    fn assert_display_has_code_and_remediation(error: &OracleError, code: &'static str) {
        let display = error.to_string();
        assert!(display.contains(code));
        assert!(display.contains("remediation:"));
        assert!(!error.remediation().is_empty());
    }

    fn prediction_fixture() -> Prediction {
        Prediction {
            outcome: AnchorValue::Bool(true),
            confidence: 0.72,
            consequences: vec![consequence(1, "compile-pass", 1, 0.5)],
            bound: SufficiencyBound {
                i_panel_oracle: 1.05,
                dpi_ceiling: 1.05,
                sufficient: true,
                per_sensor_deficit: vec![(LensId::from_bytes([7; 16]), 0.0)],
            },
            provenance: ledger(9),
            guard: guard(true),
        }
    }

    fn write_json<T: Serialize>(path: &Path, value: &T) {
        let json = serde_json::to_vec_pretty(value).expect("serialize fsv json");
        fs::write(path, json).expect("write fsv json");
    }

    fn consequence(seed: u8, action_or_event: &str, hop: u8, confidence: f32) -> Consequence {
        Consequence {
            action_or_event: action_or_event.to_string(),
            domain: DomainId::from("fixture"),
            outcome: AnchorValue::Text(format!("outcome-{seed}")),
            confidence,
            hop,
            provenance: ledger(u64::from(seed)),
        }
    }

    fn guard(pass: bool) -> GuardVerdict {
        GuardVerdict {
            guard_id: GuardId::from_str("018f48a4-9a79-74d2-8a5c-9ad7f6b8c101").expect("guard id"),
            overall_pass: pass,
            provisional: false,
            per_slot: vec![SlotVerdict {
                slot: SlotId::new(1),
                cos: 0.9,
                tau: 0.7,
                pass,
            }],
            action: Some(NoveltyAction::RejectClosed),
        }
    }

    fn ledger(seed: u64) -> LedgerRef {
        LedgerRef {
            seq: seed,
            hash: [seed as u8; 32],
        }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1.0e-6);
    }
}
