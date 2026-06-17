use std::fs;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use serde::Serialize;

use super::real_types::{PanelRelevanceReport, SlotMetricReport};
use super::request::RecallRequest;

const METRIC_KEY: &[u8] = b"ph70/sextant/panel-relevance/scifact";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PanelMetricEvidence {
    pub(crate) relevance_report_path: String,
    pub(crate) query_evidence_path: String,
    pub(crate) metrics_tsv_path: String,
    pub(crate) metric_cf: String,
    pub(crate) metric_cf_key_hex: String,
    pub(crate) metric_cf_value_bytes: usize,
    pub(crate) metric_cf_seq: u64,
    pub(crate) report: PanelRelevanceReport,
}

pub(crate) fn write_outputs(
    vault: &AsterVault,
    request: &RecallRequest,
    report: PanelRelevanceReport,
) -> Result<PanelMetricEvidence, String> {
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;
    let summary = request.metrics_dir.join("sextant_panel_relevance.json");
    let queries = request
        .metrics_dir
        .join("sextant_panel_relevance_queries.jsonl");
    let tsv = request.metrics_dir.join("sextant_panel_metrics.tsv");
    let value = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    fs::write(&summary, &value).map_err(|error| error.to_string())?;
    fs::write(&queries, query_jsonl(&report)?).map_err(|error| error.to_string())?;
    fs::write(&tsv, metrics_tsv(&report)).map_err(|error| error.to_string())?;
    let seq = vault
        .write_cf(ColumnFamily::Online, METRIC_KEY.to_vec(), value.clone())
        .map_err(|error| error.to_string())?;
    vault.flush().map_err(|error| error.to_string())?;
    Ok(PanelMetricEvidence {
        relevance_report_path: summary.display().to_string(),
        query_evidence_path: queries.display().to_string(),
        metrics_tsv_path: tsv.display().to_string(),
        metric_cf: "online".to_string(),
        metric_cf_key_hex: hex(METRIC_KEY),
        metric_cf_value_bytes: value.len(),
        metric_cf_seq: seq,
        report,
    })
}

fn query_jsonl(report: &PanelRelevanceReport) -> Result<String, String> {
    let mut out = String::new();
    for row in &report.query_evidence {
        out.push_str(&serde_json::to_string(row).map_err(|error| error.to_string())?);
        out.push('\n');
    }
    Ok(out)
}

fn metrics_tsv(report: &PanelRelevanceReport) -> String {
    let mut out = "label\tslot\tlens\tndcg_at_k\trecall_at_k\tmrr\thits_examined\n".to_string();
    push_metric(&mut out, &report.best_single);
    push_metric(&mut out, &report.candidate_fusion);
    push_metric(&mut out, &report.fused);
    out
}

fn push_metric(out: &mut String, row: &SlotMetricReport) {
    out.push_str(&format!(
        "{}\t{}\t{}\t{:.6}\t{:.6}\t{:.6}\t{}\n",
        row.label,
        row.slot.map(|slot| slot.to_string()).unwrap_or_default(),
        row.lens.clone().unwrap_or_default(),
        row.metrics.ndcg_at_k,
        row.metrics.recall_at_k,
        row.metrics.mrr,
        row.hits_examined
    ));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
