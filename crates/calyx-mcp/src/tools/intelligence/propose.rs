use calyx_anneal::{AnchorGap, DeficitMap, describe, synthesize};
use calyx_aster::cf::ColumnFamily;
use calyx_core::Clock;
use serde_json::{Value, json};

use super::core::{active_slots, anchor_label, load_context, load_docs, parse_anchor};
use super::metrics;
use super::model::{ProposeLensOut, assay_key, proposal_key};
use crate::server::{ToolError, ToolResult};

pub(super) fn run(vault_name: &str, anchor: &str) -> ToolResult<Value> {
    let ctx = load_context(vault_name)?;
    let docs = load_docs(&ctx.vault)?;
    let anchor = parse_anchor(anchor)?;
    let label = anchor_label(&anchor);
    let corpus = docs.values().cloned().collect::<Vec<_>>();
    let assay_key = assay_key(&label);
    let measured = metrics::bits(&ctx.state.panel, &docs, &anchor, &label, false, &assay_key).ok();
    let mutual_info = measured
        .as_ref()
        .map(|report| report.per_slot.iter().map(|slot| slot.bits).sum::<f64>())
        .unwrap_or(0.0);
    let entropy = entropy_bits(docs.len());
    let deficit = DeficitMap {
        computed_at: calyx_core::SystemClock.now(),
        top_gaps: vec![AnchorGap {
            anchor_class: label.clone(),
            entropy_h: entropy,
            mutual_info_i: mutual_info.min(entropy),
            gap: (entropy - mutual_info).max(0.0),
        }],
        underrepresented_modalities: underrepresented_modalities(&ctx.state.panel, &corpus),
        total_bits_deficit: (entropy - mutual_info).max(0.0),
    };
    let candidate = synthesize(&deficit, &corpus)?;
    let candidate_json = serde_json::to_value(&candidate)
        .map_err(|err| ToolError::invalid_params(format!("serialize CandidateLens: {err}")))?;
    let out = ProposeLensOut {
        name: candidate_name(&candidate),
        rationale: describe(&candidate),
        predicted_bits_gain: deficit.total_bits_deficit,
        runtime_hint: runtime_hint(&candidate).to_string(),
        estimated_cost: "zero external cost".to_string(),
        candidate: candidate_json,
    };
    super::core::write_json_row(
        &ctx.vault,
        ColumnFamily::AnnealOperators,
        proposal_key(&label),
        &out,
    )?;
    Ok(json!(out))
}

fn entropy_bits(n: usize) -> f64 {
    (n.max(2) as f64).log2()
}

fn underrepresented_modalities(
    panel: &calyx_core::Panel,
    corpus: &[calyx_core::Constellation],
) -> Vec<calyx_core::Modality> {
    if active_slots(panel).is_empty() {
        corpus
            .first()
            .map(|cx| vec![cx.modality])
            .unwrap_or_else(|| vec![calyx_core::Modality::Mixed])
    } else {
        Vec::new()
    }
}

fn candidate_name(candidate: &calyx_anneal::CandidateLens) -> String {
    match candidate {
        calyx_anneal::CandidateLens::Algorithmic { kind, .. } => {
            format!("algorithmic::{kind:?}")
        }
        calyx_anneal::CandidateLens::Commission { spec } => {
            format!("commission::{}", spec.axis)
        }
    }
}

fn runtime_hint(candidate: &calyx_anneal::CandidateLens) -> &'static str {
    match candidate {
        calyx_anneal::CandidateLens::Algorithmic { .. } => "algorithmic",
        calyx_anneal::CandidateLens::Commission { .. } => "commission",
    }
}
