use serde::{Deserialize, Serialize};

use crate::raw_large_corpus::LargeCorpusPage;
use crate::raw_onchain_backfill_runner_types::{
    OnchainBackfillCheckpoint, OnchainBackfillRunReport,
};

pub const ONCHAIN_BACKFILL_READBACK_FILE: &str = "onchain-backfill-readback-report.json";
pub const ONCHAIN_BACKFILL_READBACK_PROGRESS_FILE: &str =
    "onchain-backfill-readback-progress.jsonl";
pub const ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_FILE: &str =
    "onchain-backfill-current-run-readback-report.json";
pub const ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_PROGRESS_FILE: &str =
    "onchain-backfill-current-run-readback-progress.jsonl";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainBackfillReadbackScope {
    #[default]
    Full,
    CurrentRun,
}

impl OnchainBackfillReadbackScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::CurrentRun => "current_run",
        }
    }
}

pub(crate) fn scoped_readback_files(
    scope: OnchainBackfillReadbackScope,
) -> (&'static str, &'static str) {
    match scope {
        OnchainBackfillReadbackScope::Full => (
            ONCHAIN_BACKFILL_READBACK_FILE,
            ONCHAIN_BACKFILL_READBACK_PROGRESS_FILE,
        ),
        OnchainBackfillReadbackScope::CurrentRun => (
            ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_FILE,
            ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_PROGRESS_FILE,
        ),
    }
}

pub(crate) fn check_current_run_pages_in_checkpoint(
    report: &OnchainBackfillRunReport,
    checkpoint: &OnchainBackfillCheckpoint,
    parse_failures: &mut Vec<String>,
) {
    for page in &report.pages {
        check_page_in_checkpoint(page, checkpoint, parse_failures);
    }
}

fn check_page_in_checkpoint(
    page: &LargeCorpusPage,
    checkpoint: &OnchainBackfillCheckpoint,
    parse_failures: &mut Vec<String>,
) {
    let Some(range_state) = &page.range_state else {
        return;
    };
    let Some(request_path) = page.request_path.as_deref() else {
        return;
    };
    let Some(contract) = checkpoint
        .contracts
        .iter()
        .find(|contract| contract.dataset == page.dataset)
    else {
        parse_failures.push(format!(
            "{} dataset {} was missing from checkpoint",
            page.metadata_path, page.dataset
        ));
        return;
    };
    let found = contract.captured_ranges.iter().any(|range| {
        range.from_block == range_state.from_block
            && range.to_block == range_state.to_block
            && range.record_count == page.record_count
            && range.request_path == request_path
            && range.body_path == page.body_path
    });
    if !found {
        parse_failures.push(format!(
            "{} current-run page was not represented exactly in checkpoint",
            page.metadata_path
        ));
    }
}
