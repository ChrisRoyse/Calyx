use serde::Serialize;

#[derive(Serialize)]
pub(super) struct IngestReport {
    pub(super) cx_id: String,
    pub(super) new: bool,
    pub(super) ledger_seq: u64,
}

#[derive(Serialize)]
pub(super) struct AnchorReport {
    pub(super) status: &'static str,
    pub(super) cx_id: String,
    pub(super) ledger_seq: u64,
}
