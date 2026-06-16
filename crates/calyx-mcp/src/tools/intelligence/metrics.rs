use std::collections::BTreeMap;

use calyx_core::{AnchorKind, CalyxError, Constellation, CxId, Panel, Slot, SlotId, SlotVector};

use super::core::{
    active_slots, anchor_label, cosine, dense, has_anchor, has_anchor_kind, has_any_anchor,
};
use super::model::{AbundanceOut, BitsExplainOut, BitsOut, KernelOut, SlotBitsOut, hex};
use crate::server::ToolResult;

const MIN_ANCHORS: usize = 50;
const LOW_SIGNAL_BITS: f64 = 0.05;
const REDUNDANT_CORR: f64 = 0.6;
const ESTIMATOR: &str = "centroid_cosine_v1";

pub(super) fn abundance(
    docs: &BTreeMap<CxId, Constellation>,
    active_slots: &[SlotId],
) -> AbundanceOut {
    let n = docs.len();
    let materialized = docs
        .values()
        .map(|cx| concrete_slot_count(cx, active_slots))
        .sum::<usize>();
    AbundanceOut {
        n,
        pairs: pairs(n),
        materialized,
        n_eff: effective_slots(n, materialized, active_slots.len()),
        dpi_ceiling: (n.max(1) as f64 + 1.0).log2(),
        panel_size: active_slots.len(),
    }
}

pub(super) fn bits(
    panel: &Panel,
    docs: &BTreeMap<CxId, Constellation>,
    anchor: &AnchorKind,
    label: &str,
    explain: bool,
    key: &[u8],
) -> ToolResult<BitsOut> {
    let observed = docs
        .values()
        .filter(|cx| has_anchor_kind(cx, anchor))
        .collect::<Vec<_>>();
    if observed.len() < MIN_ANCHORS {
        return Err(insufficient_samples(label, observed.len()).into());
    }
    let slots = active_slots(panel);
    if slots.is_empty() {
        return Err(
            CalyxError::assay_low_signal(format!("bits for {label} has no active slots")).into(),
        );
    }
    reject_redundant_pair(docs, &slots)?;
    let positive = observed
        .iter()
        .copied()
        .filter(|cx| has_anchor(cx, anchor))
        .collect::<Vec<_>>();
    let comparison = comparison_docs(docs, anchor, &observed);
    let per_slot = slots
        .iter()
        .map(|slot| slot_bits(slot, &positive, &comparison, observed.len()))
        .collect::<Vec<_>>();
    if per_slot.iter().all(|slot| slot.bits < LOW_SIGNAL_BITS) {
        return Err(CalyxError::assay_low_signal(format!(
            "all active slots are below {LOW_SIGNAL_BITS:.2} bits for {label}"
        ))
        .into());
    }
    let total_bits = per_slot.iter().map(|slot| slot.bits).sum::<f64>();
    let dpi_ceiling = dpi_ceiling(observed.len());
    Ok(BitsOut {
        anchor: label.to_string(),
        panel_sufficiency: clamp01(total_bits / dpi_ceiling.max(1e-9)),
        n: observed.len(),
        dpi_ceiling,
        per_slot,
        explain: explain.then(|| BitsExplainOut {
            positive_anchor_count: positive.len(),
            comparison_count: comparison.len(),
            persisted_cf: "assay".to_string(),
            persisted_key_hex: hex(key),
        }),
    })
}

pub(super) fn kernel(
    docs: &BTreeMap<CxId, Constellation>,
    anchor: Option<&AnchorKind>,
) -> ToolResult<KernelOut> {
    let grounded = docs
        .values()
        .filter(|cx| has_any_anchor(cx, anchor))
        .map(|cx| cx.cx_id)
        .collect::<Vec<_>>();
    if grounded.is_empty() {
        return Err(CalyxError::kernel_ungrounded("kernel has no grounded anchors").into());
    }
    let total = docs.len();
    let budget = total.div_ceil(100).max(1);
    let kernel_cx_ids = grounded
        .iter()
        .copied()
        .take(budget)
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    let missing = total.saturating_sub(grounded.len());
    Ok(KernelOut {
        kernel_size: kernel_cx_ids.len(),
        recall: grounded.len() as f32 / total.max(1) as f32,
        total_cx: total,
        kernel_cx_ids,
        grounding_gaps: gaps(missing, anchor),
    })
}

fn concrete_slot_count(cx: &Constellation, active_slots: &[SlotId]) -> usize {
    active_slots
        .iter()
        .filter(|slot| {
            cx.slots
                .get(slot)
                .is_some_and(|vector| !matches!(vector, SlotVector::Absent { .. }))
        })
        .count()
}

fn pairs(n: usize) -> u64 {
    let n = n as u64;
    n.saturating_mul(n.saturating_sub(1)) / 2
}

fn effective_slots(n: usize, materialized: usize, panel_size: usize) -> f64 {
    if n == 0 || panel_size == 0 {
        0.0
    } else {
        materialized as f64 / n as f64
    }
}

fn comparison_docs<'a>(
    docs: &'a BTreeMap<CxId, Constellation>,
    anchor: &AnchorKind,
    observed: &[&'a Constellation],
) -> Vec<&'a Constellation> {
    let negative = observed
        .iter()
        .copied()
        .filter(|cx| !has_anchor(cx, anchor))
        .collect::<Vec<_>>();
    if !negative.is_empty() {
        return negative;
    }
    docs.values()
        .filter(|cx| !has_anchor_kind(cx, anchor))
        .collect()
}

fn slot_bits(
    slot: &Slot,
    positives: &[&Constellation],
    comparisons: &[&Constellation],
    n: usize,
) -> SlotBitsOut {
    let bits = centroid_gap_bits(slot.slot_id, positives, comparisons);
    let margin = confidence_margin(n);
    SlotBitsOut {
        slot: slot.slot_id.get(),
        name: slot.slot_key.key().to_string(),
        bits,
        ci: [(bits - margin).max(0.0), (bits + margin).min(1.0)],
        estimator: ESTIMATOR.to_string(),
        state: "active".to_string(),
        low_signal: bits < LOW_SIGNAL_BITS,
    }
}

fn centroid_gap_bits(
    slot: SlotId,
    positives: &[&Constellation],
    comparisons: &[&Constellation],
) -> f64 {
    let Some(pos) = centroid(slot, positives) else {
        return 0.0;
    };
    let Some(neg) = centroid(slot, comparisons) else {
        return 0.0;
    };
    cosine(&pos, &neg)
        .map(|cos| ((1.0 - f64::from(cos)) / 2.0).clamp(0.0, 1.0))
        .unwrap_or(0.0)
}

fn centroid(slot: SlotId, docs: &[&Constellation]) -> Option<Vec<f32>> {
    let mut count = 0usize;
    let mut out = Vec::<f32>::new();
    for cx in docs {
        let Some(values) = dense(cx, slot) else {
            continue;
        };
        if out.is_empty() {
            out.resize(values.len(), 0.0);
        }
        if out.len() != values.len() {
            return None;
        }
        for (sum, value) in out.iter_mut().zip(values) {
            *sum += *value;
        }
        count += 1;
    }
    if count == 0 {
        return None;
    }
    for value in &mut out {
        *value /= count as f32;
    }
    Some(out)
}

fn reject_redundant_pair(docs: &BTreeMap<CxId, Constellation>, slots: &[&Slot]) -> ToolResult<()> {
    for left_idx in 0..slots.len() {
        for right_idx in (left_idx + 1)..slots.len() {
            let corr = slot_pair_corr(docs, slots[left_idx].slot_id, slots[right_idx].slot_id);
            if corr > REDUNDANT_CORR {
                return Err(CalyxError::assay_redundant(format!(
                    "slots {} and {} corr {:.3} > {REDUNDANT_CORR}",
                    slots[left_idx].slot_id, slots[right_idx].slot_id, corr
                ))
                .into());
            }
        }
    }
    Ok(())
}

fn slot_pair_corr(docs: &BTreeMap<CxId, Constellation>, left: SlotId, right: SlotId) -> f64 {
    let mut total = 0.0f64;
    let mut count = 0usize;
    for cx in docs.values() {
        let (Some(left), Some(right)) = (dense(cx, left), dense(cx, right)) else {
            continue;
        };
        let Some(cos) = cosine(left, right) else {
            continue;
        };
        total += f64::from(cos.abs());
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

fn confidence_margin(n: usize) -> f64 {
    0.10 / (n.max(1) as f64).sqrt()
}

fn dpi_ceiling(n: usize) -> f64 {
    (n.max(1) as f64 + 1.0).log2()
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn insufficient_samples(anchor: &str, n: usize) -> CalyxError {
    CalyxError {
        code: "CALYX_ASSAY_INSUFFICIENT_SAMPLES",
        message: format!("bits for {anchor} requires >=50 anchored outcomes; got {n}"),
        remediation: "anchor ≥50 outcomes first",
    }
}

fn gaps(missing: usize, anchor: Option<&AnchorKind>) -> Vec<String> {
    if missing == 0 {
        return Vec::new();
    }
    let axis = anchor
        .map(anchor_label)
        .unwrap_or_else(|| "any_anchor".to_string());
    vec![format!("{axis}:missing_grounding:{missing}")]
}
