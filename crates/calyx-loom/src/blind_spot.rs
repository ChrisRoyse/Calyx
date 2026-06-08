//! Cross-lens anomaly detector.

use calyx_core::{CxId, SlotId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindSpotAlert {
    pub cx_id: CxId,
    pub a: SlotId,
    pub b: SlotId,
    pub delta: f32,
    pub severity: Severity,
}

pub fn detect_blind_spot(
    cx_id: CxId,
    a: SlotId,
    b: SlotId,
    lens_a_similarity: f32,
    lens_b_neighbor_mean: f32,
) -> Option<BlindSpotAlert> {
    let delta = lens_a_similarity - lens_b_neighbor_mean;
    if delta < 0.5 {
        return None;
    }
    let severity = if delta >= 0.8 {
        Severity::High
    } else if delta >= 0.65 {
        Severity::Medium
    } else {
        Severity::Low
    };
    Some(BlindSpotAlert {
        cx_id,
        a,
        b,
        delta,
        severity,
    })
}
