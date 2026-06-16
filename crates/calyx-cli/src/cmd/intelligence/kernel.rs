use std::collections::BTreeMap;

use calyx_aster::cf::ColumnFamily;
use calyx_core::{AnchorKind, CalyxError, Constellation, CxId, VaultStore};

use super::core::{
    anchor_label, has_any_anchor, load_context, load_docs, parse_anchor, write_json_row,
};
use super::model::{KernelOut, kernel_key};
use super::parse::KernelArgs;
use crate::error::CliResult;
use crate::output::print_json;

pub(super) fn command(args: KernelArgs) -> CliResult {
    let ctx = load_context(&args.vault)?;
    let docs = load_docs(&ctx.vault)?;
    let anchor = args.anchor.as_deref().map(parse_anchor).transpose()?;
    let label = anchor.as_ref().map(anchor_label);
    let report = calculate(&docs, anchor.as_ref())?;
    let key = kernel_key(label.as_deref());
    if args.rebuild || !row_exists(&ctx.vault, &key)? {
        write_json_row(&ctx.vault, ColumnFamily::Kernel, key, &report)?;
    }
    print_json(&report)
}

pub(super) fn calculate(
    docs: &BTreeMap<CxId, Constellation>,
    anchor: Option<&AnchorKind>,
) -> CliResult<KernelOut> {
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
    let kernel_ids = grounded
        .iter()
        .copied()
        .take(budget)
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    let missing = total.saturating_sub(grounded.len());
    Ok(KernelOut {
        kernel_size: kernel_ids.len(),
        recall: grounded.len() as f32 / total.max(1) as f32,
        total_cx: total,
        kernel_cx_ids: kernel_ids,
        grounding_gaps: gaps(missing, anchor),
    })
}

fn row_exists(vault: &calyx_aster::vault::AsterVault, key: &[u8]) -> CliResult<bool> {
    Ok(vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Kernel, key)?
        .is_some())
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
