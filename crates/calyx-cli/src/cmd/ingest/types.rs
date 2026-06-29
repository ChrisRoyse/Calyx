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
    pub(super) verified_base_rows: usize,
    pub(super) first_cx_id: Option<String>,
    pub(super) last_cx_id: Option<String>,
    pub(super) first_ledger_seq: Option<u64>,
    pub(super) last_ledger_seq: Option<u64>,
}

impl BatchIngestSummary {
    pub(super) fn empty() -> Self {
        Self {
            status: "ingested",
            source_of_truth: "Aster Base CF readback after flush",
            row_count: 0,
            new_count: 0,
            already_count: 0,
            verified_base_rows: 0,
            first_cx_id: None,
            last_cx_id: None,
            first_ledger_seq: None,
            last_ledger_seq: None,
        }
    }

    pub(super) fn record(&mut self, report: &IngestReport) {
        self.row_count += 1;
        if report.new {
            self.new_count += 1;
        } else {
            self.already_count += 1;
        }
        self.verified_base_rows += 1;
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
