mod basic;
mod mastery;
mod mastery_plan;
mod mastery_support;
mod oracle;
mod oracle_graph;
mod oracle_plan;
mod reactive;
mod reactive_plan;
mod reactive_support;
mod shared;
mod track_spines;

const ORACLE_FORECAST_PANEL_VERSION: u32 = 1240;
const ORACLE_FORECAST_EVIDENCE_KIND: &str = "oracle_forecast_evidence";
const ORACLE_FORECAST_GRAPH_KIND: &str = "oracle_forecast_recurrence";
const TRACK_SPINES_PANEL_VERSION: u64 = 1242;
const TRACK_SPINES_EVIDENCE_KIND: &str = "track_spines_evidence";

const REACTIVE_AFFECT_PANEL_VERSION: u32 = 1244;
const REACTIVE_AFFECT_DEFAULT_SLOT_ID: u16 = 13;
const REACTIVE_AFFECT_MAX_SLOT_ID: u16 = 47;
const REACTIVE_AFFECT_EVIDENCE_KIND: &str = "reactive_affect_evidence";
const REACTIVE_AFFECT_BASELINE_KIND: &str = "reactive_affect_baseline";
const REACTIVE_AFFECT_MATCHED_KIND: &str = "reactive_affect_matched_region";
const REACTIVE_AFFECT_RECURRENCE_KIND: &str = "reactive_affect_recurrence";
const REACTIVE_AFFECT_MIN_MMD_WINDOW: usize = 4;
