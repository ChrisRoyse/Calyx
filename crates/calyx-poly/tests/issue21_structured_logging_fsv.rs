use std::fs;
use std::path::{Path, PathBuf};

use calyx_poly::edge_audit::{EdgeCaseDriver, EdgeCaseSpec, EdgeInputClass, drive_edge_case};
use calyx_poly::{
    POLY_LOG_EVENT_RECORDED, POLY_LOG_MAX_CONTEXT_FIELDS, PolyError, PolyLogEvent, PolyLogLevel,
    StructuredLogSink, log_context, read_structured_log_events,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const FIELD_EMPTY: &str = "POLY_LOG_FIELD_EMPTY";
const CONTEXT_TOO_LARGE: &str = "POLY_LOG_CONTEXT_TOO_LARGE";
const JSON_PARSE_FAILED: &str = "POLY_LOG_JSON_PARSE_FAILED";

#[test]
fn issue21_structured_logging_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE21_FSV_ROOT", "poly-issue21-logging");
    reset_dir(&root);

    let happy = run_case(
        &root,
        "happy-jsonl-write-readback",
        EdgeInputClass::HappyPath,
        POLY_LOG_EVENT_RECORDED,
        true,
        Scenario::Happy,
    );
    let empty_component = run_case(
        &root,
        "edge-empty-component",
        EdgeInputClass::EmptyInput,
        FIELD_EMPTY,
        false,
        Scenario::EmptyComponent,
    );
    let oversized_context = run_case(
        &root,
        "edge-oversized-context",
        EdgeInputClass::MaxLimit,
        CONTEXT_TOO_LARGE,
        false,
        Scenario::OversizedContext,
    );
    let malformed_readback = run_case(
        &root,
        "edge-malformed-jsonl-readback",
        EdgeInputClass::InvalidInput,
        JSON_PARSE_FAILED,
        true,
        Scenario::MalformedReadback,
    );

    for outcome in [
        &happy,
        &empty_component,
        &oversized_context,
        &malformed_readback,
    ] {
        assert!(
            outcome.ok,
            "{} expected {} got {}",
            outcome.name, outcome.expected_code, outcome.observed_code
        );
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 21,
        "source_of_truth": [
            "physical structured JSONL log files",
            "separate read_structured_log_events readback",
            "per-case before.json, decision.json, after.json, and edge-case-outcome.json"
        ],
        "happy_path": happy,
        "edge_cases": {
            "empty_component": empty_component,
            "oversized_context": oversized_context,
            "malformed_jsonl_readback": malformed_readback
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue21_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue21_fsv_root={}", root.display());
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    Happy,
    EmptyComponent,
    OversizedContext,
    MalformedReadback,
}

struct Fixture {
    log_path: PathBuf,
    readback_path: PathBuf,
    scenario: Scenario,
}

fn run_case(
    root: &Path,
    name: &str,
    input_class: EdgeInputClass,
    expected_code: &str,
    expect_state_change: bool,
    scenario: Scenario,
) -> calyx_poly::edge_audit::EdgeCaseOutcome {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let fixture = Fixture {
        log_path: case_dir.join("structured-log.jsonl"),
        readback_path: case_dir.join("readback.json"),
        scenario,
    };

    drive_edge_case(
        EdgeCaseSpec {
            case_dir: &case_dir,
            name,
            input_class,
            expected_code,
            expect_state_change,
        },
        EdgeCaseDriver {
            read_before: || state(&fixture),
            execute: || execute_case(&fixture),
            read_after: || state(&fixture),
            decision_record,
        },
    )
    .expect("drive edge case")
}

fn execute_case(fixture: &Fixture) -> calyx_poly::Result<Value> {
    match fixture.scenario {
        Scenario::Happy => {
            let sink = StructuredLogSink::new(&fixture.log_path)?;
            let started = PolyLogEvent::info(
                "forecast_agent",
                "write_known_truth_prediction",
                "POLY_SYNTHETIC_FORECAST_STARTED",
                "synthetic known-truth forecast logging started",
                log_context(&[
                    ("market_id", "poly-issue21-market".to_string()),
                    ("expected_probability", "0.62".to_string()),
                ]),
            )?;
            sink.append_event(&started)?;
            let error = PolyError::config(
                "POLY_CONFIG_SYNTHETIC_INVALID",
                "synthetic config edge proves what_failed and how_to_fix are logged",
            );
            sink.append_error(
                "config",
                "load_known_truth_fixture",
                &error,
                log_context(&[("fixture", "poly-issue21-config.toml".to_string())]),
            )?;
            let events = read_structured_log_events(&fixture.log_path)?;
            assert_eq!(events.len(), 2);
            assert_eq!(events[1].level, PolyLogLevel::Error);
            assert!(
                events[1]
                    .what_failed
                    .as_deref()
                    .unwrap_or("")
                    .contains("configuration")
            );
            assert!(
                events[1]
                    .how_to_fix
                    .as_deref()
                    .unwrap_or("")
                    .contains("config")
            );
            let readback = json!({
                "code": POLY_LOG_EVENT_RECORDED,
                "event_count": events.len(),
                "events": events
            });
            write_json(&fixture.readback_path, &readback);
            Ok(readback)
        }
        Scenario::EmptyComponent => {
            let sink = StructuredLogSink::new(&fixture.log_path)?;
            let event = PolyLogEvent::info(
                "",
                "validate",
                "POLY_SYNTHETIC_EMPTY_COMPONENT",
                "this event must fail before persistence",
                log_context(&[]),
            )?;
            sink.append_event(&event)?;
            Ok(json!({"code": "UNREACHABLE"}))
        }
        Scenario::OversizedContext => {
            let sink = StructuredLogSink::new(&fixture.log_path)?;
            let mut context = std::collections::BTreeMap::new();
            for index in 0..=POLY_LOG_MAX_CONTEXT_FIELDS {
                context.insert(format!("field_{index:02}"), "x".to_string());
            }
            let event = PolyLogEvent::info(
                "forecast_agent",
                "validate_context_limit",
                "POLY_SYNTHETIC_CONTEXT_LIMIT",
                "this event must fail before persistence",
                context,
            )?;
            sink.append_event(&event)?;
            Ok(json!({"code": "UNREACHABLE"}))
        }
        Scenario::MalformedReadback => {
            fs::write(&fixture.log_path, "not-json\n").expect("write malformed log fixture");
            read_structured_log_events(&fixture.log_path)?;
            Ok(json!({"code": "UNREACHABLE"}))
        }
    }
}

fn decision_record(result: calyx_poly::Result<Value>) -> (String, Value) {
    match result {
        Ok(value) => (
            value["code"].as_str().unwrap_or("MISSING_CODE").to_string(),
            value,
        ),
        Err(error) => (
            error.code().to_string(),
            json!({
                "code": error.code(),
                "error": error.diagnostic()
            }),
        ),
    }
}

fn state(fixture: &Fixture) -> Value {
    let bytes = fs::read(&fixture.log_path).ok();
    let readback_bytes = fs::read(&fixture.readback_path).ok();
    json!({
        "log": {
            "path": fixture.log_path.display().to_string(),
            "exists": fixture.log_path.exists(),
            "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
            "blake3": bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes())),
            "readback": read_structured_log_events(&fixture.log_path)
                .map(|events| json!({"ok": true, "event_count": events.len(), "events": events}))
                .unwrap_or_else(|error| json!({"ok": false, "error": error.diagnostic()}))
        },
        "readback_file": {
            "path": fixture.readback_path.display().to_string(),
            "exists": fixture.readback_path.exists(),
            "bytes": readback_bytes.as_ref().map(Vec::len).unwrap_or(0),
            "blake3": readback_bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes()))
        }
    })
}
