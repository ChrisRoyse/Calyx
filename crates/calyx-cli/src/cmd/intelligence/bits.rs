use std::collections::BTreeMap;

use calyx_aster::cf::ColumnFamily;
use calyx_core::{AnchorKind, CalyxError, Constellation, CxId, Slot, SlotId};

use super::core::{
    active_slots, anchor_label, cosine, dense, has_anchor, has_anchor_kind, load_context,
    load_docs, parse_anchor, write_json_row,
};
use super::model::{BitsExplainOut, BitsOut, SlotBitsOut, assay_key, hex};
use super::parse::BitsArgs;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const MIN_ANCHORS: usize = 50;
const LOW_SIGNAL_BITS: f64 = 0.05;
const ESTIMATOR: &str = "centroid_cosine_v1";

pub(super) fn command(args: BitsArgs) -> CliResult {
    let ctx = load_context(&args.vault)?;
    let docs = load_docs(&ctx.vault)?;
    let anchor = parse_anchor(&args.anchor_kind)?;
    let label = anchor_label(&anchor);
    let key = assay_key(&label);
    let report = calculate(&ctx.state.panel, &docs, &anchor, &label, args.explain, &key)?;
    write_json_row(&ctx.vault, ColumnFamily::Assay, key, &report)?;
    print_json(&report)
}

pub(super) fn calculate(
    panel: &calyx_core::Panel,
    docs: &BTreeMap<CxId, Constellation>,
    anchor: &AnchorKind,
    label: &str,
    explain: bool,
    key: &[u8],
) -> CliResult<BitsOut> {
    let observed = docs
        .values()
        .filter(|cx| has_anchor_kind(cx, anchor))
        .collect::<Vec<_>>();
    if observed.len() < MIN_ANCHORS {
        return Err(insufficient_samples(label, observed.len()));
    }
    let slots = active_slots(panel);
    if slots.is_empty() {
        return Err(low_signal(format!("bits for {label} has no active slots")));
    }
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
        return Err(low_signal(format!(
            "all active slots are below {LOW_SIGNAL_BITS:.2} bits for {label}"
        )));
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
        .collect::<Vec<_>>()
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

fn confidence_margin(n: usize) -> f64 {
    0.10 / (n.max(1) as f64).sqrt()
}

fn dpi_ceiling(n: usize) -> f64 {
    (n.max(1) as f64 + 1.0).log2()
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn insufficient_samples(anchor: &str, n: usize) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_ASSAY_INSUFFICIENT_SAMPLES",
        message: format!("bits for {anchor} requires >=50 anchored outcomes; got {n}"),
        remediation: "anchor ≥50 outcomes first",
    })
}

fn low_signal(message: String) -> CliError {
    CliError::from(CalyxError::assay_low_signal(message))
}
