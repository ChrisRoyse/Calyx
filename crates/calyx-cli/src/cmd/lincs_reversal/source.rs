use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::model::LincsGraphDraft;
use crate::error::CliResult;

mod ingest;
mod root;
mod rows;

use ingest::{
    ingest_disease_signatures, ingest_request_records, ingest_scores, ingest_unsupported,
};
pub(crate) use root::VerifiedRootReport;
use root::{add_source_nodes, read_json_file, verify_root};

pub(super) const FAMILY: &str = "lincs_cmap_reversal";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LincsSourceReport {
    pub root: VerifiedRootReport,
    pub parsed_row_counts: BTreeMap<String, usize>,
    pub run_summary: Value,
}

pub(crate) fn load_root(root: &Path) -> CliResult<(LincsGraphDraft, LincsSourceReport)> {
    let mut draft = LincsGraphDraft::default();
    let (index, root_report) = verify_root(root, &mut draft)?;
    add_source_nodes(&index, &mut draft);
    let run_summary = read_json_file(&root.join("run_summary.json"))?;
    let mut report = LincsSourceReport {
        root: root_report,
        parsed_row_counts: BTreeMap::new(),
        run_summary,
    };
    ingest_disease_signatures(&index, &mut draft, &mut report)?;
    ingest_request_records(&index, &mut draft, &mut report)?;
    ingest_scores(&index, &mut draft, &mut report)?;
    ingest_unsupported(&index, &mut draft, &mut report)?;
    draft.require_path("disease_signature_to_reversal_score")?;
    draft.require_path("reversal_score_to_perturbation")?;
    draft.require_path("unsupported_current_candidate_absence")?;
    Ok((draft, report))
}
