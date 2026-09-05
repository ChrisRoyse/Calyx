use calyx_core::{CalyxError, Clock};

use crate::recall_test::kernel_recall_test_with_clock;
use crate::{
    Kernel, build_kernel_index, build_panel_kernel_index, build_typed_panel_kernel_index,
    panel_kernel_recall_test, typed_panel_kernel_recall_test,
};

use super::{SummarizeRecall, to_calyx};

pub(super) fn apply_measured_recall(
    kernel: &mut Kernel,
    recall: Option<&SummarizeRecall<'_>>,
    clock: &dyn Clock,
) -> Result<(), CalyxError> {
    let Some(recall) = recall else {
        return Ok(());
    };
    if kernel.members.is_empty() {
        return Ok(());
    }
    let approx_factor = kernel.recall.approx_factor;
    let tau_star_estimate = kernel.recall.tau_star_estimate;
    let tau_star_exact = kernel.recall.tau_star_exact;
    let mut measured = match recall {
        SummarizeRecall::Legacy {
            embeddings,
            full_index,
            corpus,
            params,
        } => {
            let index = build_kernel_index(kernel, *embeddings).map_err(to_calyx)?;
            kernel_recall_test_with_clock(&index, *full_index, *corpus, params, clock)
                .map_err(to_calyx)?
        }
        SummarizeRecall::Panel {
            slots,
            full,
            corpus_name,
            params,
        } => {
            let index = build_panel_kernel_index(kernel, slots, full).map_err(to_calyx)?;
            panel_kernel_recall_test(&index, full, params, corpus_name).map_err(to_calyx)?
        }
        SummarizeRecall::TypedPanel {
            slots,
            full,
            corpus_name,
            params,
        } => {
            let index = build_typed_panel_kernel_index(kernel, slots, full).map_err(to_calyx)?;
            typed_panel_kernel_recall_test(&index, full, params, corpus_name).map_err(to_calyx)?
        }
    };
    measured.approx_factor = approx_factor;
    measured.tau_star_estimate = tau_star_estimate;
    measured.tau_star_exact = tau_star_exact;
    kernel.recall = measured;
    Ok(())
}
