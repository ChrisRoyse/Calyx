//! Reusable edge-case audit harness for Full State Verification tests.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Standard edge input classes expected by Poly FSV.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeInputClass {
    HappyPath,
    EmptyInput,
    MaxLimit,
    InvalidInput,
}

/// Persisted before/action/after readback for one edge-case audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeCaseOutcome {
    pub name: String,
    pub input_class: EdgeInputClass,
    pub expected_code: String,
    pub observed_code: String,
    pub state_change_expected: bool,
    pub state_changed: bool,
    pub ok: bool,
    pub before: Value,
    pub decision: Value,
    pub after: Value,
}

/// Static metadata for one edge-case run.
pub struct EdgeCaseSpec<'a> {
    pub case_dir: &'a Path,
    pub name: &'a str,
    pub input_class: EdgeInputClass,
    pub expected_code: &'a str,
    pub expect_state_change: bool,
}

/// Closures that read state, execute the action, and serialize the decision.
pub struct EdgeCaseDriver<ReadBefore, Execute, ReadAfter, DecisionRecord> {
    pub read_before: ReadBefore,
    pub execute: Execute,
    pub read_after: ReadAfter,
    pub decision_record: DecisionRecord,
}

/// Drive one case by reading state, executing the action, then reading state again.
pub fn drive_edge_case<D, ReadBefore, Execute, ReadAfter, DecisionRecord>(
    spec: EdgeCaseSpec<'_>,
    driver: EdgeCaseDriver<ReadBefore, Execute, ReadAfter, DecisionRecord>,
) -> Result<EdgeCaseOutcome, String>
where
    ReadBefore: FnOnce() -> Value,
    Execute: FnOnce() -> D,
    ReadAfter: FnOnce() -> Value,
    DecisionRecord: FnOnce(D) -> (String, Value),
{
    let case_dir = spec.case_dir;
    fs::create_dir_all(case_dir).map_err(write_err)?;

    let before = (driver.read_before)();
    let decision = (driver.execute)();
    let after = (driver.read_after)();
    let (observed_code, decision) = (driver.decision_record)(decision);
    let state_changed = before != after;
    let ok = observed_code == spec.expected_code && state_changed == spec.expect_state_change;

    write_json(&case_dir.join("before.json"), &before)?;
    write_json(&case_dir.join("decision.json"), &decision)?;
    write_json(&case_dir.join("after.json"), &after)?;

    let outcome = EdgeCaseOutcome {
        name: spec.name.to_string(),
        input_class: spec.input_class,
        expected_code: spec.expected_code.to_string(),
        observed_code,
        state_change_expected: spec.expect_state_change,
        state_changed,
        ok,
        before,
        decision,
        after,
    };
    write_json(
        &case_dir.join("edge-case-outcome.json"),
        &serde_json::to_value(&outcome).map_err(|err| err.to_string())?,
    )?;
    Ok(outcome)
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| err.to_string())?;
    fs::write(path, bytes).map_err(write_err)
}

fn write_err(err: std::io::Error) -> String {
    format!("POLY_EDGE_AUDIT_WRITE_FAILED:{err}")
}
