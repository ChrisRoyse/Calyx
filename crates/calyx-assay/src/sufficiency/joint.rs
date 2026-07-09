use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::calibration::underpowered;
use crate::estimate::MiEstimate;

/// Union-bounded panel joint estimate (#1140 Finding C). A concatenated-feature
/// probe can fall below the strongest single member; this floors the panel
/// estimate and lower bound at the best admitted member.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelJointBasis {
    pub bits: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub raw_joint_bits: f32,
    pub raw_joint_ci_low: f32,
    pub best_member_bits: f32,
    pub best_member_ci_low: f32,
    pub floored: bool,
}

pub fn panel_joint_with_union_floor(
    joint: &MiEstimate,
    members: &[MiEstimate],
) -> Result<PanelJointBasis> {
    if members.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "panel joint union floor requires at least one admitted member estimate",
        ));
    }
    if !joint.bits.is_finite() || !joint.ci_low.is_finite() || !joint.ci_high.is_finite() {
        return Err(underpowered(
            "panel joint estimate must be finite to apply the union-bound floor",
        ));
    }
    let mut best_member_bits = 0.0_f32;
    let mut best_member_ci_low = 0.0_f32;
    for member in members {
        if !member.bits.is_finite() || !member.ci_low.is_finite() {
            return Err(underpowered(
                "admitted member estimate must be finite to apply the union-bound floor",
            ));
        }
        best_member_bits = best_member_bits.max(member.bits);
        best_member_ci_low = best_member_ci_low.max(member.ci_low);
    }
    let bits = joint.bits.max(best_member_bits);
    let ci_low = joint.ci_low.max(best_member_ci_low);
    let ci_high = joint.ci_high.max(bits);
    Ok(PanelJointBasis {
        bits,
        ci_low,
        ci_high,
        raw_joint_bits: joint.bits,
        raw_joint_ci_low: joint.ci_low,
        best_member_bits,
        best_member_ci_low,
        floored: bits > joint.bits || ci_low > joint.ci_low,
    })
}
