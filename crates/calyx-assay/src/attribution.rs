//! Per-sensor signal attribution and bits reports.

use std::collections::BTreeSet;

use calyx_core::{Anchor, CalyxError, CxId, Result, SlotId};
use serde::{Deserialize, Serialize};

use crate::estimate::{TrustTag, provisional_without_anchor, trust_for_anchor};

pub const CALYX_ASSAY_INVALID_COVERAGE: &str = "CALYX_ASSAY_INVALID_COVERAGE";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CoverageMask {
    #[default]
    Full,
    Partial {
        total: usize,
        observed: BTreeSet<CxId>,
    },
}

impl CoverageMask {
    pub fn partial(total: usize, observed: impl IntoIterator<Item = CxId>) -> Result<Self> {
        let observed = observed.into_iter().collect::<BTreeSet<_>>();
        if observed.len() > total {
            return Err(invalid_coverage(format!(
                "coverage observed {} rows but total is {total}",
                observed.len()
            )));
        }
        Ok(Self::Partial { total, observed })
    }

    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }

    pub fn observed_count(&self) -> Option<usize> {
        match self {
            Self::Full => None,
            Self::Partial { observed, .. } => Some(observed.len()),
        }
    }

    pub fn total_count(&self) -> Option<usize> {
        match self {
            Self::Full => None,
            Self::Partial { total, .. } => Some(*total),
        }
    }

    pub fn coverage_rate(&self) -> f32 {
        match self {
            Self::Full => 1.0,
            Self::Partial { total: 0, .. } => 0.0,
            Self::Partial { total, observed } => observed.len() as f32 / *total as f32,
        }
    }

    pub fn contains(&self, cx: CxId) -> bool {
        match self {
            Self::Full => true,
            Self::Partial { observed, .. } => observed.contains(&cx),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotAttribution {
    pub slot: SlotId,
    pub marginal_bits: f32,
    pub sole_carrier: bool,
    #[serde(default, skip_serializing_if = "CoverageMask::is_full")]
    pub coverage: CoverageMask,
}

impl SlotAttribution {
    pub fn bits_for(&self, cx: CxId) -> Option<f32> {
        self.coverage.contains(cx).then_some(self.marginal_bits)
    }

    pub fn is_observed_for(&self, cx: CxId) -> bool {
        self.coverage.contains(cx)
    }

    pub fn coverage_rate(&self) -> f32 {
        self.coverage.coverage_rate()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BitsReport {
    pub slots: Vec<SlotAttribution>,
    pub total_bits: f32,
    pub trust: TrustTag,
}

pub fn per_sensor_attribution(
    slot_bits: &[(SlotId, f32)],
    sole_threshold_bits: f32,
) -> Vec<SlotAttribution> {
    let slot_bits = slot_bits
        .iter()
        .map(|(slot, bits)| (*slot, *bits, CoverageMask::Full))
        .collect::<Vec<_>>();
    per_sensor_attribution_with_coverage(&slot_bits, sole_threshold_bits)
}

pub fn per_sensor_attribution_with_coverage(
    slot_bits: &[(SlotId, f32, CoverageMask)],
    sole_threshold_bits: f32,
) -> Vec<SlotAttribution> {
    let strong_slots = slot_bits
        .iter()
        .filter(|(_, bits, coverage)| {
            *bits >= sole_threshold_bits && coverage.coverage_rate() > 0.0
        })
        .count();
    slot_bits
        .iter()
        .map(|(slot, bits, coverage)| SlotAttribution {
            slot: *slot,
            marginal_bits: *bits,
            sole_carrier: *bits >= sole_threshold_bits
                && coverage.coverage_rate() > 0.0
                && strong_slots == 1,
            coverage: coverage.clone(),
        })
        .collect()
}

pub fn bits_report(slots: Vec<SlotAttribution>, trust: TrustTag) -> BitsReport {
    bits_report_with_trust(slots, provisional_without_anchor(trust))
}

pub fn bits_report_with_anchor(slots: Vec<SlotAttribution>, anchor: &Anchor) -> BitsReport {
    bits_report_with_trust(slots, trust_for_anchor(Some(anchor)))
}

fn bits_report_with_trust(slots: Vec<SlotAttribution>, trust: TrustTag) -> BitsReport {
    BitsReport {
        total_bits: slots.iter().map(|slot| slot.marginal_bits).sum(),
        slots,
        trust,
    }
}

impl BitsReport {
    pub fn total_bits_for(&self, cx: CxId) -> f32 {
        self.slots.iter().filter_map(|slot| slot.bits_for(cx)).sum()
    }

    pub fn observed_slots_for(&self, cx: CxId) -> Vec<SlotId> {
        self.slots
            .iter()
            .filter(|slot| slot.is_observed_for(cx))
            .map(|slot| slot.slot)
            .collect()
    }
}

fn invalid_coverage(message: String) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_INVALID_COVERAGE,
        message,
        remediation: "build coverage masks from the exact observed constellation ids",
    }
}
