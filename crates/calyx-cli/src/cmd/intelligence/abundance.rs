use calyx_core::{SlotId, SlotVector};

use super::core::{active_slots, load_context, load_docs};
use super::model::AbundanceOut;
use super::parse::AbundanceArgs;
use crate::error::CliResult;
use crate::output::print_json;

pub(super) fn command(args: AbundanceArgs) -> CliResult {
    let ctx = load_context(&args.vault)?;
    let docs = load_docs(&ctx.vault)?;
    let slots = active_slots(&ctx.state.panel)
        .into_iter()
        .map(|slot| slot.slot_id)
        .collect::<Vec<_>>();
    let report = calculate(&docs, &slots);
    print_json(&report)
}

pub(super) fn calculate(
    docs: &std::collections::BTreeMap<calyx_core::CxId, calyx_core::Constellation>,
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
        n_eff: effective_slots(docs.len(), materialized, active_slots.len()),
        dpi_ceiling: (n.max(1) as f64 + 1.0).log2(),
        panel_size: active_slots.len(),
    }
}

fn concrete_slot_count(cx: &calyx_core::Constellation, active_slots: &[SlotId]) -> usize {
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
