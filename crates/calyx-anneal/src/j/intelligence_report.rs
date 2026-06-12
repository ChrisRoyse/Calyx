use calyx_aster::{cf::ColumnFamily, vault::AsterVault};
use calyx_core::{CalyxError, Clock, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    CandidateAction, GoodhartReport, GoodhartState, GradientEntryReadback, IntelligenceGradient,
    JMetricSources, JObjectiveContext, JTerms, JWeights, LogicalTime, PriorityReadback, compute_j,
};

pub const ANNEAL_REPORT_TAG: &str = "calyx_anneal_report_v1";
pub const CALYX_ANNEAL_REPORT_INVALID_ROW: &str = "CALYX_ANNEAL_REPORT_INVALID_ROW";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IntelligenceReport {
    pub j: f64,
    pub terms: JTerms,
    pub weights: JWeights,
    pub dpi_ceiling: f64,
    pub dpi_headroom: f64,
    pub provisional_excluded: usize,
    pub gradient: Vec<GradientEntryReadback>,
    pub next_best_action: Option<CandidateAction>,
    pub goodhart_last: Option<GoodhartReport>,
    pub ts: LogicalTime,
    pub availability: ReportAvailability,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReportAvailability {
    Available,
    Unavailable {
        code: String,
        message: String,
        remediation: String,
    },
}

impl ReportAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportDiff {
    pub delta_j: f64,
    pub per_term_deltas: JTermDeltas,
    pub new_gradient_top: Option<GradientEntryReadback>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct JTermDeltas {
    pub w1_info: f64,
    pub w2_n_eff: f64,
    pub w3_sufficiency: f64,
    pub w4_kernel_recall: f64,
    pub w5_oracle_accuracy: f64,
    pub w6_mistake_rate: f64,
    pub w7_compression: f64,
    pub w8_coverage: f64,
    pub p_redundant: f64,
    pub p_ungrounded: f64,
    pub p_goodhart: f64,
}

#[derive(Serialize)]
struct TermContribution {
    label: &'static str,
    raw: Value,
    weight: Value,
    contribution: Value,
}

pub fn intelligence_report<S>(
    context: &JObjectiveContext,
    sources: &S,
    gradient: &IntelligenceGradient,
    goodhart_state: &GoodhartState,
    goodhart_last: Option<GoodhartReport>,
    clock: &dyn Clock,
) -> IntelligenceReport
where
    S: JMetricSources,
{
    let context = JObjectiveContext {
        domain: context.domain.clone(),
        panel_len: context.panel_len,
        weights: context.weights,
        goodhart_penalty: goodhart_state.p_goodhart,
    };
    let ts = clock.now();
    match compute_j(&context, sources) {
        Ok(value) => IntelligenceReport {
            j: value.j,
            terms: value.terms,
            weights: value.weights,
            dpi_ceiling: value.dpi_ceiling,
            dpi_headroom: value.dpi_headroom,
            provisional_excluded: value.provisional_excluded,
            gradient: gradient.top_readback(5),
            next_best_action: gradient.next_best_action().cloned(),
            goodhart_last,
            ts,
            availability: ReportAvailability::Available,
        },
        Err(error) => unavailable_report(context.weights, gradient, goodhart_last, ts, error),
    }
}

pub fn format_report(report: &IntelligenceReport) -> String {
    let mut out = String::new();
    match &report.availability {
        ReportAvailability::Available => out.push_str(&format!("J = {}\n", fmt(report.j))),
        ReportAvailability::Unavailable { code, message, .. } => {
            out.push_str(&format!("J = unavailable ({code}: {message})\n"));
        }
    }
    out.push_str(&format!("DPI headroom: {}\n", fmt(report.dpi_headroom)));
    out.push_str(&format!(
        "provisional_excluded: {}\n",
        report.provisional_excluded
    ));
    out.push_str("Terms:\n");
    for row in term_contributions(report) {
        out.push_str(&format!(
            "  {}: raw {} weight {} contribution {}\n",
            row.label,
            value_text(&row.raw),
            value_text(&row.weight),
            value_text(&row.contribution)
        ));
    }
    out.push_str("Gradient top 5:\n");
    if report.gradient.is_empty() {
        out.push_str("  <empty>\n");
    } else {
        for (index, entry) in report.gradient.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {:?} estimated_dj {} dJ/cost {} cost {}\n",
                index + 1,
                entry.action,
                fmt(entry.estimated_dj),
                priority_text(&entry.dj_per_cost),
                entry.cost_budget_units
            ));
        }
    }
    match &report.next_best_action {
        Some(action) => out.push_str(&format!("next_best_action: {action:?}\n")),
        None => out.push_str("next_best_action: None\n"),
    }
    match &report.goodhart_last {
        Some(goodhart) => out.push_str(&format!(
            "Goodhart: passed={} violations={} penalty={}\n",
            goodhart.passed,
            goodhart.violations.len(),
            fmt(goodhart.p_goodhart_increment)
        )),
        None => out.push_str("Goodhart: no check yet\n"),
    }
    out
}

pub fn to_json(report: &IntelligenceReport) -> Value {
    json!({
        "j": finite_json(report.j),
        "terms": terms_json(report.terms),
        "weights": report.weights,
        "dpi_ceiling": finite_json(report.dpi_ceiling),
        "dpi_headroom": finite_json(report.dpi_headroom),
        "provisional_excluded": report.provisional_excluded,
        "gradient": &report.gradient,
        "next_best_action": &report.next_best_action,
        "goodhart_last": &report.goodhart_last,
        "ts": report.ts,
        "availability": &report.availability,
        "term_contributions": term_contributions(report),
        "human": format_report(report),
    })
}

pub fn report_diff(before: &IntelligenceReport, after: &IntelligenceReport) -> ReportDiff {
    ReportDiff {
        delta_j: after.j - before.j,
        per_term_deltas: JTermDeltas {
            w1_info: after.terms.w1_info - before.terms.w1_info,
            w2_n_eff: after.terms.w2_n_eff - before.terms.w2_n_eff,
            w3_sufficiency: after.terms.w3_sufficiency - before.terms.w3_sufficiency,
            w4_kernel_recall: after.terms.w4_kernel_recall - before.terms.w4_kernel_recall,
            w5_oracle_accuracy: after.terms.w5_oracle_accuracy - before.terms.w5_oracle_accuracy,
            w6_mistake_rate: after.terms.w6_mistake_rate - before.terms.w6_mistake_rate,
            w7_compression: after.terms.w7_compression - before.terms.w7_compression,
            w8_coverage: after.terms.w8_coverage - before.terms.w8_coverage,
            p_redundant: after.terms.p_redundant - before.terms.p_redundant,
            p_ungrounded: after.terms.p_ungrounded - before.terms.p_ungrounded,
            p_goodhart: after.terms.p_goodhart - before.terms.p_goodhart,
        },
        new_gradient_top: after.gradient.first().cloned(),
    }
}

pub fn anneal_report_key(ts: LogicalTime) -> Vec<u8> {
    ts.to_be_bytes().to_vec()
}

pub fn write_intelligence_report_snapshot<C>(
    vault: &AsterVault<C>,
    report: &IntelligenceReport,
) -> Result<Vec<u8>>
where
    C: Clock,
{
    let key = anneal_report_key(report.ts);
    let row = json!({
        "tag": ANNEAL_REPORT_TAG,
        "report": to_json(report),
    });
    let value = serde_json::to_vec_pretty(&row)
        .map_err(|error| invalid_row(format!("encode intelligence report row: {error}")))?;
    vault.write_cf(ColumnFamily::AnnealReport, key.clone(), value)?;
    vault.flush()?;
    Ok(key)
}

pub fn read_intelligence_report_snapshot<C>(
    vault: &AsterVault<C>,
    ts: LogicalTime,
) -> Result<Option<IntelligenceReport>>
where
    C: Clock,
{
    let Some(bytes) = vault.read_cf_at(
        vault.latest_seq(),
        ColumnFamily::AnnealReport,
        &anneal_report_key(ts),
    )?
    else {
        return Ok(None);
    };
    decode_intelligence_report_row(&bytes).map(Some)
}

pub fn latest_intelligence_report_snapshot<C>(
    vault: &AsterVault<C>,
) -> Result<Option<IntelligenceReport>>
where
    C: Clock,
{
    let mut rows = vault.scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealReport)?;
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    for (_, bytes) in rows.into_iter().rev() {
        if report_row_is_available(&bytes)? {
            return decode_intelligence_report_row(&bytes).map(Some);
        }
    }
    Ok(None)
}

pub fn decode_intelligence_report_row(bytes: &[u8]) -> Result<IntelligenceReport> {
    let row: Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid_row(format!("parse intelligence report row JSON: {error}")))?;
    validate_tag(&row)?;
    let report = row
        .get("report")
        .ok_or_else(|| invalid_row("intelligence report row missing report"))?
        .clone();
    if report_availability_state(&report) == Some("unavailable") {
        return decode_unavailable_report(report);
    }
    serde_json::from_value(report)
        .map_err(|error| invalid_row(format!("decode intelligence report payload: {error}")))
}

fn unavailable_report(
    weights: JWeights,
    gradient: &IntelligenceGradient,
    goodhart_last: Option<GoodhartReport>,
    ts: LogicalTime,
    error: CalyxError,
) -> IntelligenceReport {
    IntelligenceReport {
        j: f64::NAN,
        terms: nan_terms(),
        weights,
        dpi_ceiling: f64::NAN,
        dpi_headroom: f64::NAN,
        provisional_excluded: 0,
        gradient: gradient.top_readback(5),
        next_best_action: gradient.next_best_action().cloned(),
        goodhart_last,
        ts,
        availability: ReportAvailability::Unavailable {
            code: error.code.to_string(),
            message: error.message,
            remediation: error.remediation.to_string(),
        },
    }
}

fn report_row_is_available(bytes: &[u8]) -> Result<bool> {
    let row: Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid_row(format!("parse intelligence report row JSON: {error}")))?;
    validate_tag(&row)?;
    Ok(row.get("report").and_then(report_availability_state) == Some("available"))
}

fn validate_tag(row: &Value) -> Result<()> {
    match row.get("tag").and_then(Value::as_str) {
        Some(ANNEAL_REPORT_TAG) => Ok(()),
        Some(other) => Err(invalid_row(format!(
            "unexpected intelligence report tag {other}"
        ))),
        None => Err(invalid_row("intelligence report row missing tag")),
    }
}

fn decode_unavailable_report(report: Value) -> Result<IntelligenceReport> {
    let availability =
        serde_json::from_value::<ReportAvailability>(required(&report, "availability")?.clone())
            .map_err(|error| invalid_row(format!("decode unavailable availability: {error}")))?;
    let weights = serde_json::from_value::<JWeights>(required(&report, "weights")?.clone())
        .map_err(|error| invalid_row(format!("decode unavailable weights: {error}")))?;
    let gradient = serde_json::from_value::<Vec<GradientEntryReadback>>(
        required(&report, "gradient")?.clone(),
    )
    .map_err(|error| invalid_row(format!("decode unavailable gradient: {error}")))?;
    let next_best_action = match report.get("next_best_action") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            serde_json::from_value::<CandidateAction>(value.clone())
                .map_err(|error| invalid_row(format!("decode unavailable next action: {error}")))?,
        ),
    };
    let goodhart_last = match report.get("goodhart_last") {
        Some(Value::Null) | None => None,
        Some(value) => Some(
            serde_json::from_value::<GoodhartReport>(value.clone())
                .map_err(|error| invalid_row(format!("decode unavailable Goodhart: {error}")))?,
        ),
    };
    Ok(IntelligenceReport {
        j: f64::NAN,
        terms: nan_terms(),
        weights,
        dpi_ceiling: f64::NAN,
        dpi_headroom: f64::NAN,
        provisional_excluded: report
            .get("provisional_excluded")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        gradient,
        next_best_action,
        goodhart_last,
        ts: required(&report, "ts")?
            .as_u64()
            .ok_or_else(|| invalid_row("unavailable report ts must be u64"))?,
        availability,
    })
}

fn required<'a>(value: &'a Value, field: &str) -> Result<&'a Value> {
    value
        .get(field)
        .ok_or_else(|| invalid_row(format!("intelligence report missing {field}")))
}

fn report_availability_state(report: &Value) -> Option<&str> {
    report
        .get("availability")
        .and_then(|availability| availability.get("state"))
        .and_then(Value::as_str)
}

fn term_contributions(report: &IntelligenceReport) -> Vec<TermContribution> {
    let t = report.terms;
    let w = report.weights;
    vec![
        positive("w1_info", t.w1_info, w.w1),
        positive("w2_n_eff", t.w2_n_eff, w.w2),
        positive("w3_sufficiency", t.w3_sufficiency, w.w3),
        positive("w4_kernel_recall", t.w4_kernel_recall, w.w4),
        positive("w5_oracle_accuracy", t.w5_oracle_accuracy, w.w5),
        negative("w6_mistake_rate", t.w6_mistake_rate, w.w6),
        positive("w7_compression", t.w7_compression, w.w7),
        positive("w8_coverage", t.w8_coverage, w.w8),
        penalty("p_redundant", t.p_redundant),
        penalty("p_ungrounded", t.p_ungrounded),
        penalty("p_goodhart", t.p_goodhart),
    ]
}

fn positive(label: &'static str, raw: f64, weight: f64) -> TermContribution {
    TermContribution {
        label,
        raw: finite_json(raw),
        weight: finite_json(weight),
        contribution: finite_json(raw * weight),
    }
}

fn negative(label: &'static str, raw: f64, weight: f64) -> TermContribution {
    TermContribution {
        label,
        raw: finite_json(raw),
        weight: finite_json(weight),
        contribution: finite_json(-(raw * weight)),
    }
}

fn penalty(label: &'static str, raw: f64) -> TermContribution {
    TermContribution {
        label,
        raw: finite_json(raw),
        weight: finite_json(1.0),
        contribution: finite_json(-raw),
    }
}

fn terms_json(terms: JTerms) -> Value {
    json!({
        "w1_info": finite_json(terms.w1_info),
        "w2_n_eff": finite_json(terms.w2_n_eff),
        "w3_sufficiency": finite_json(terms.w3_sufficiency),
        "w4_kernel_recall": finite_json(terms.w4_kernel_recall),
        "w5_oracle_accuracy": finite_json(terms.w5_oracle_accuracy),
        "w6_mistake_rate": finite_json(terms.w6_mistake_rate),
        "w7_compression": finite_json(terms.w7_compression),
        "w8_coverage": finite_json(terms.w8_coverage),
        "p_redundant": finite_json(terms.p_redundant),
        "p_ungrounded": finite_json(terms.p_ungrounded),
        "p_goodhart": finite_json(terms.p_goodhart),
    })
}

fn finite_json(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        Value::Null
    }
}

fn value_text(value: &Value) -> String {
    value
        .as_f64()
        .map(fmt)
        .unwrap_or_else(|| "unavailable".to_string())
}

fn priority_text(priority: &PriorityReadback) -> String {
    match priority {
        PriorityReadback::Finite { value } => fmt(*value),
        PriorityReadback::Infinite => "infinite".to_string(),
    }
}

fn fmt(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        "unavailable".to_string()
    }
}

fn nan_terms() -> JTerms {
    JTerms {
        w1_info: f64::NAN,
        w2_n_eff: f64::NAN,
        w3_sufficiency: f64::NAN,
        w4_kernel_recall: f64::NAN,
        w5_oracle_accuracy: f64::NAN,
        w6_mistake_rate: f64::NAN,
        w7_compression: f64::NAN,
        w8_coverage: f64::NAN,
        p_redundant: f64::NAN,
        p_ungrounded: f64::NAN,
        p_goodhart: f64::NAN,
    }
}

fn invalid_row(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_REPORT_INVALID_ROW,
        message: message.into(),
        remediation: "discard corrupt anneal_report row and regenerate the report from grounded inputs",
    }
}
