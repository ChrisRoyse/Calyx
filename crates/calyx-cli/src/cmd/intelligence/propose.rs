use calyx_anneal::{AnchorGap, DeficitMap, describe, synthesize};
use calyx_aster::cf::ColumnFamily;
use calyx_core::Clock;

use super::bits;
use super::core::{
    active_slots, anchor_label, load_context, load_docs, parse_anchor, write_json_row,
};
use super::model::{ProposeLensOut, assay_key};
use super::parse::ProposeLensArgs;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(super) fn command(args: ProposeLensArgs) -> CliResult {
    let ctx = load_context(&args.vault)?;
    let docs = load_docs(&ctx.vault)?;
    let anchor = parse_anchor(&args.anchor)?;
    let label = anchor_label(&anchor);
    let corpus = docs.values().cloned().collect::<Vec<_>>();
    let assay_key = assay_key(&label);
    let measured =
        bits::calculate(&ctx.state.panel, &docs, &anchor, &label, false, &assay_key).ok();
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
        .map_err(|error| CliError::usage(format!("serialize CandidateLens: {error}")))?;
    let out = ProposeLensOut {
        name: candidate_name(&candidate),
        rationale: describe(&candidate),
        predicted_bits_gain: deficit.total_bits_deficit,
        runtime_hint: runtime_hint(&candidate).to_string(),
        candidate: candidate_json,
    };
    write_json_row(
        &ctx.vault,
        ColumnFamily::AnnealOperators,
        proposal_key(&label),
        &out,
    )?;
    print_json(&out)
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

fn proposal_key(anchor: &str) -> Vec<u8> {
    let mut key = b"propose-lens\0".to_vec();
    key.extend_from_slice(anchor.as_bytes());
    key
}
