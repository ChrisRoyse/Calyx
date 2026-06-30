use std::collections::BTreeSet;

use calyx_core::CxId;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IngestOutput {
    Summary,
    Rows,
}

#[derive(Serialize)]
pub(super) struct IngestReport {
    pub(super) cx_id: String,
    pub(super) new: bool,
    pub(super) ledger_seq: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(super) struct BatchIngestSummary {
    pub(super) status: &'static str,
    pub(super) source_of_truth: &'static str,
    pub(super) row_count: usize,
    pub(super) new_count: usize,
    pub(super) already_count: usize,
    pub(super) runtime_new_count: usize,
    pub(super) runtime_already_count: usize,
    pub(super) verified_base_rows: usize,
    pub(super) distinct_cx_count: usize,
    pub(super) batch_base_visible_before: usize,
    pub(super) batch_base_visible_after: usize,
    pub(super) batch_base_materialized_count: usize,
    pub(super) batch_base_tombstoned_before: usize,
    pub(super) batch_base_tombstoned_after: usize,
    pub(super) first_cx_id: Option<String>,
    pub(super) last_cx_id: Option<String>,
    pub(super) first_ledger_seq: Option<u64>,
    pub(super) last_ledger_seq: Option<u64>,
    #[serde(skip)]
    pub(super) batch_cx_ids: BTreeSet<CxId>,
}

impl BatchIngestSummary {
    pub(super) fn empty() -> Self {
        Self {
            status: "ingested",
            source_of_truth: "physical Aster Base CF readback for distinct batch Cx IDs after flush",
            row_count: 0,
            new_count: 0,
            already_count: 0,
            runtime_new_count: 0,
            runtime_already_count: 0,
            verified_base_rows: 0,
            distinct_cx_count: 0,
            batch_base_visible_before: 0,
            batch_base_visible_after: 0,
            batch_base_materialized_count: 0,
            batch_base_tombstoned_before: 0,
            batch_base_tombstoned_after: 0,
            first_cx_id: None,
            last_cx_id: None,
            first_ledger_seq: None,
            last_ledger_seq: None,
            batch_cx_ids: BTreeSet::new(),
        }
    }

    pub(super) fn record(&mut self, cx_id: CxId, report: &IngestReport) {
        self.row_count += 1;
        if report.new {
            self.runtime_new_count += 1;
        } else {
            self.runtime_already_count += 1;
        }
        self.verified_base_rows += 1;
        self.batch_cx_ids.insert(cx_id);
        if self.first_cx_id.is_none() {
            self.first_cx_id = Some(report.cx_id.clone());
        }
        self.last_cx_id = Some(report.cx_id.clone());
        if self.first_ledger_seq.is_none() {
            self.first_ledger_seq = Some(report.ledger_seq);
        }
        self.last_ledger_seq = Some(report.ledger_seq);
    }
}

#[derive(Serialize)]
pub(super) struct AnchorReport {
    pub(super) status: &'static str,
    pub(super) cx_id: String,
    pub(super) ledger_seq: u64,
}
