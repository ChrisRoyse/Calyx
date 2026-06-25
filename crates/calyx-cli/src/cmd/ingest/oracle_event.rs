use std::collections::BTreeMap;

use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{
    DEFAULT_MAX_OCCURRENCES, OccurrenceContext, RetentionPolicy, append_occurrence, read_series,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorValue, CxId};
use serde::Deserialize;
use serde_json::json;

use super::anchor::{parse_anchor_kind, parse_anchor_value};
use crate::error::{CliError, CliResult};

const DEFAULT_OUTCOME_KIND: &str = "label:answer";
const ORACLE_STRUCTURED_METADATA_KEY: &str = "oracle.structured";

#[derive(Clone, Debug)]
pub(super) struct OracleEvent {
    domain: String,
    action: String,
    outcome: AnchorValue,
    grounded: bool,
    t_secs: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct OracleEventSpec {
    domain: String,
    action: String,
    outcome: String,
    #[serde(default)]
    outcome_kind: Option<String>,
    #[serde(default = "default_grounded")]
    grounded: bool,
    #[serde(default)]
    t_secs: Option<i64>,
}

pub(super) fn parse_oracle_event(index: usize, spec: OracleEventSpec) -> CliResult<OracleEvent> {
    let line = index + 1;
    let domain = required_field(line, "oracle.domain", spec.domain)?;
    let action = required_field(line, "oracle.action", spec.action)?;
    let outcome_raw = required_field(line, "oracle.outcome", spec.outcome)?;
    let outcome_kind = spec
        .outcome_kind
        .unwrap_or_else(|| DEFAULT_OUTCOME_KIND.to_string());
    let kind = parse_anchor_kind(&outcome_kind).map_err(|err| {
        CliError::usage(format!(
            "batch JSONL line {line} oracle.outcome_kind: {}",
            err.message()
        ))
    })?;
    let outcome = parse_anchor_value(&kind, &outcome_kind, &outcome_raw).map_err(|err| {
        CliError::usage(format!(
            "batch JSONL line {line} oracle.outcome: {}",
            err.message()
        ))
    })?;
    if let Some(t_secs) = spec.t_secs
        && t_secs < 0
    {
        return Err(CliError::usage(format!(
            "batch JSONL line {line} oracle.t_secs must be non-negative"
        )));
    }
    Ok(OracleEvent {
        domain,
        action,
        outcome,
        grounded: spec.grounded,
        t_secs: spec.t_secs,
    })
}

impl OracleEvent {
    pub(super) fn apply_metadata(&self, metadata: &mut BTreeMap<String, String>) -> CliResult {
        metadata.insert(
            calyx_oracle::ORACLE_DOMAIN_METADATA_KEY.to_string(),
            self.domain.clone(),
        );
        metadata.insert(
            calyx_oracle::ORACLE_ACTION_METADATA_KEY.to_string(),
            self.action.clone(),
        );
        metadata.insert(
            calyx_oracle::ORACLE_EFFECT_METADATA_KEY.to_string(),
            serde_json::to_string(&self.outcome)?,
        );
        metadata.insert(
            ORACLE_STRUCTURED_METADATA_KEY.to_string(),
            "true".to_string(),
        );
        Ok(())
    }

    fn context(&self) -> CliResult<OccurrenceContext> {
        let value = json!({
            "outcome_anchor": { "value": self.outcome },
            "consequence": {
                "domain": self.domain,
                "outcome": { "value": self.outcome },
                "grounded": self.grounded,
                "provisional": !self.grounded
            }
        });
        let bytes = serde_json::to_vec(&value)?;
        OccurrenceContext::new(bytes).map_err(CliError::from)
    }
}

pub(super) fn append_recurrence_if_absent(
    vault: &AsterVault,
    cx_id: CxId,
    event: &OracleEvent,
    now_ms: u64,
) -> CliResult<bool> {
    let series = read_series(vault, cx_id)?;
    if !series.occurrences.is_empty() || series.frequency > 0 {
        return Ok(false);
    }
    let observed_secs = i64::try_from(now_ms / 1000)
        .map_err(|_| CliError::usage("oracle observed time exceeds i64 epoch seconds"))?;
    let t_secs = event.t_secs.unwrap_or(observed_secs);
    append_occurrence(
        vault,
        cx_id,
        EpochSecs(t_secs),
        event.context()?,
        EpochSecs(observed_secs),
        oracle_retention_policy()?,
    )?;
    Ok(true)
}

fn oracle_retention_policy() -> CliResult<RetentionPolicy> {
    RetentionPolicy::new(DEFAULT_MAX_OCCURRENCES, u64::MAX).map_err(CliError::from)
}

fn required_field(line: usize, field: &str, value: String) -> CliResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::usage(format!(
            "batch JSONL line {line} {field} must not be empty"
        )));
    }
    Ok(trimmed.to_string())
}

fn default_grounded() -> bool {
    true
}
