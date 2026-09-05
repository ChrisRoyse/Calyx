use std::fs;
use std::path::{Path, PathBuf};

use calyx_poly::agent_artifacts::AGENT_FORECAST_SCHEMA_VERSION;
use calyx_poly::edge_audit::{EdgeCaseDriver, EdgeCaseSpec, EdgeInputClass, drive_edge_case};
use calyx_poly::error::PolyError;
use calyx_poly::{
    AGENT_REPRODUCTION_BIT_FOR_BIT, AgentForecastArtifactRequest, AgentForecastManifest,
    AgentForecastReproductionReport, AgentForecastReproductionRequest, AgentSourceSnapshotRef,
    DeepSeekSecretMetadata, read_agent_reproduction_report, reproduce_agent_forecast_artifacts,
    write_agent_forecast_artifacts, write_agent_reproduction_report,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue135_reproducibility_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE135_FSV_ROOT", "poly-issue135-repro");
    reset_dir(&root);

    let happy = run_case(
        &root,
        "happy-bit-for-bit",
        EdgeInputClass::HappyPath,
        AGENT_REPRODUCTION_BIT_FOR_BIT,
        true,
        Scenario::Happy,
    );
    let missing_prompt = run_case(
        &root,
        "missing-prompt",
        EdgeInputClass::EmptyInput,
        "POLY_AGENT_REPRO_MISSING_PROMPT",
        false,
        Scenario::MissingPrompt,
    );
    let changed_source = run_case(
        &root,
        "changed-source-snapshot",
        EdgeInputClass::InvalidInput,
        "POLY_AGENT_REPRO_SOURCE_SNAPSHOT_MISMATCH",
        false,
        Scenario::ChangedSourceSnapshot,
    );
    let parser_version = run_case(
        &root,
        "changed-parser-version",
        EdgeInputClass::InvalidInput,
        "POLY_AGENT_REPRO_PARSER_VERSION_MISMATCH",
        false,
        Scenario::ChangedParserVersion,
    );

    for outcome in [&happy, &missing_prompt, &changed_source, &parser_version] {
        assert!(
            outcome.ok,
            "{} expected {} got {}",
            outcome.name, outcome.expected_code, outcome.observed_code
        );
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 135,
        "source_of_truth": [
            "original persisted forecast artifact files",
            "persisted local ledger-row.json",
            "reproduced artifact files in a separate root",
            "reproduction-report.json readback",
            "per-case before/action/after edge-case outcome files"
        ],
        "happy_path": happy,
        "edge_cases": {
            "missing_prompt": missing_prompt,
            "changed_source_snapshot": changed_source,
            "changed_parser_version": parser_version
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue135_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue135_fsv_root={}", root.display());
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    Happy,
    MissingPrompt,
    ChangedSourceSnapshot,
    ChangedParserVersion,
}

struct Fixture {
    run_id: String,
    original_run_dir: PathBuf,
    reproduction_root: PathBuf,
    report_path: PathBuf,
    ledger_row_path: PathBuf,
    manifest: AgentForecastManifest,
    expected_source_snapshot_refs: Vec<AgentSourceSnapshotRef>,
    expected_schema_version: String,
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
    let fixture = prepare_fixture(&case_dir, name, scenario);

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
            execute: || {
                let request = reproduction_request(&fixture);
                let result = reproduce_agent_forecast_artifacts(&request);
                if let Ok(report) = &result {
                    write_agent_reproduction_report(&fixture.report_path, report)
                        .expect("write reproduction report");
                    let readback = read_agent_reproduction_report(&fixture.report_path)
                        .expect("read reproduction report");
                    assert_eq!(&readback, report);
                }
                result
            },
            read_after: || state(&fixture),
            decision_record,
        },
    )
    .expect("drive edge case")
}

fn prepare_fixture(case_dir: &Path, name: &str, scenario: Scenario) -> Fixture {
    let artifacts_root = case_dir.join("original-artifacts");
    let ledger_dir = case_dir.join("ledger");
    let reproduction_root = case_dir.join("reproduced-artifacts");
    let report_path = case_dir.join("reproduction-report.json");
    let run_id = format!("issue135_{name}").replace('-', "_");
    let request = artifact_request(&run_id);
    let manifest =
        write_agent_forecast_artifacts(&artifacts_root, &request).expect("write original bundle");
    let original_run_dir = artifacts_root.join(&run_id);
    let ledger_row_path = ledger_dir.join("ledger-row.json");
    write_json(
        &ledger_row_path,
        &json!({
            "kind": "agent_forecast",
            "payload": manifest.provenance_payload()
        }),
    );

    let mut expected_source_snapshot_refs = manifest.source_snapshot_refs.clone();
    let mut expected_schema_version = manifest.schema_version.clone();
    match scenario {
        Scenario::Happy => {}
        Scenario::MissingPrompt => {
            fs::remove_file(original_run_dir.join(&manifest.prompt.rendered_prompt_path))
                .expect("remove prompt for edge case");
        }
        Scenario::ChangedSourceSnapshot => {
            expected_source_snapshot_refs[0].snapshot += 1;
        }
        Scenario::ChangedParserVersion => {
            expected_schema_version = "poly.agent.forecast.v2".to_string();
        }
    }

    Fixture {
        run_id,
        original_run_dir,
        reproduction_root,
        report_path,
        ledger_row_path,
        manifest,
        expected_source_snapshot_refs,
        expected_schema_version,
    }
}

fn reproduction_request(fixture: &Fixture) -> AgentForecastReproductionRequest {
    AgentForecastReproductionRequest {
        original_run_dir: fixture.original_run_dir.clone(),
        reproduction_root: fixture.reproduction_root.clone(),
        ledger_row_path: fixture.ledger_row_path.clone(),
        expected_source_snapshot_refs: fixture.expected_source_snapshot_refs.clone(),
        expected_schema_version: fixture.expected_schema_version.clone(),
    }
}

fn decision_record(result: calyx_poly::Result<AgentForecastReproductionReport>) -> (String, Value) {
    match result {
        Ok(report) => (
            AGENT_REPRODUCTION_BIT_FOR_BIT.to_string(),
            json!({
                "code": AGENT_REPRODUCTION_BIT_FOR_BIT,
                "report": report_summary(&report)
            }),
        ),
        Err(error) => {
            let code = error_code(&error);
            (
                code.clone(),
                json!({
                    "code": code,
                    "error": error.to_string()
                }),
            )
        }
    }
}

fn state(fixture: &Fixture) -> Value {
    let report_bytes = fs::read(&fixture.report_path).ok();
    json!({
        "run_id": fixture.run_id,
        "original": {
            "run_dir": file_state(&fixture.original_run_dir),
            "manifest": file_hash_state(&fixture.original_run_dir.join("manifest.json")),
            "prompt": file_hash_state(&fixture.original_run_dir.join(&fixture.manifest.prompt.rendered_prompt_path)),
            "raw_response": file_hash_state(&fixture.original_run_dir.join(&fixture.manifest.response.raw_response_path)),
            "parsed_forecast": file_hash_state(&fixture.original_run_dir.join(&fixture.manifest.parsed_forecast_path)),
            "markdown_prediction": file_hash_state(&fixture.original_run_dir.join(&fixture.manifest.markdown_prediction_path))
        },
        "ledger": file_hash_state(&fixture.ledger_row_path),
        "expected_contract": {
            "source_snapshot_refs": fixture.expected_source_snapshot_refs,
            "schema_version": fixture.expected_schema_version
        },
        "reproduced_run": file_state(&fixture.reproduction_root.join(&fixture.run_id)),
        "report": {
            "path": fixture.report_path.display().to_string(),
            "exists": fixture.report_path.exists(),
            "bytes": report_bytes.as_ref().map(Vec::len).unwrap_or(0),
            "blake3": report_bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes())),
            "readback": report_bytes.as_ref().and_then(|bytes| {
                serde_json::from_slice::<AgentForecastReproductionReport>(bytes)
                    .ok()
                    .map(|report| report_summary(&report))
            })
        }
    })
}

fn file_state(path: &Path) -> Value {
    json!({
        "path": path.display().to_string(),
        "exists": path.exists()
    })
}

fn file_hash_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes()))
    })
}

fn report_summary(report: &AgentForecastReproductionReport) -> Value {
    json!({
        "schema_version": report.schema_version,
        "run_id": report.run_id,
        "bit_for_bit": report.bit_for_bit,
        "file_count": report.files.len(),
        "ledger_payload_matches_original_manifest": report.ledger_payload_matches_original_manifest,
        "ledger_payload_matches_reproduced_manifest": report.ledger_payload_matches_reproduced_manifest,
        "files": report.files.iter().map(|file| {
            json!({
                "relative_path": file.relative_path,
                "original_bytes": file.original_bytes,
                "reproduced_bytes": file.reproduced_bytes,
                "identical": file.identical,
                "original_blake3": file.original_blake3,
                "reproduced_blake3": file.reproduced_blake3
            })
        }).collect::<Vec<_>>()
    })
}

fn error_code(error: &PolyError) -> String {
    match error {
        PolyError::AgentReproduction { code, .. }
        | PolyError::AgentArtifact { code, .. }
        | PolyError::Calyx { code, .. } => code.clone(),
        other => format!("UNEXPECTED_POLY_ERROR:{other}"),
    }
}

fn artifact_request(run_id: &str) -> AgentForecastArtifactRequest {
    AgentForecastArtifactRequest {
        run_id: run_id.to_string(),
        created_at: "2026-07-03T12:00:00Z".to_string(),
        source_snapshot_refs: vec![AgentSourceSnapshotRef {
            cx_id: "issue135-cx-0001".to_string(),
            role: "candidate_market".to_string(),
            snapshot: 136,
        }],
        prompt_template_id: "poly-local-forecast".to_string(),
        prompt_template_version: AGENT_FORECAST_SCHEMA_VERSION.to_string(),
        rendered_prompt: "Use local Poly evidence only. Produce a no-trade forecast JSON."
            .to_string(),
        provider: provider_metadata(),
        raw_response_json: response_json(),
        markdown_prediction:
            "# Local forecast\n\nProbability: 0.72\n\nNo trading action is authorized.\n"
                .to_string(),
    }
}

fn response_json() -> String {
    serde_json::to_string(&json!({
        "probability": 0.72,
        "confidence": 0.64,
        "rationale": "Local source snapshots support a 0.72 probability without any site use.",
        "constraints": [
            "local_only",
            "no_trade",
            "polymarket_data_only"
        ],
        "no_trade_policy_assertion": true
    }))
    .expect("encode response JSON")
}

fn provider_metadata() -> DeepSeekSecretMetadata {
    DeepSeekSecretMetadata {
        project_id: "poly-test-project".to_string(),
        environment: "dev".to_string(),
        secret_path: "/poly/deepseek".to_string(),
        api_key_name: "DEEPSEEK_API_KEY".to_string(),
        key_present: true,
        key_length: 32,
        key_has_sk_prefix: true,
        key_sha256_prefix: "111111111111".to_string(),
        base_url: "https://api.deepseek.com".to_string(),
        model: "deepseek-chat".to_string(),
        chat_completions_url: "https://api.deepseek.com/chat/completions".to_string(),
    }
}
