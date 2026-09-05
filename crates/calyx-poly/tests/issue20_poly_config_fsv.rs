use std::fs;
use std::path::{Path, PathBuf};

use calyx_poly::edge_audit::{EdgeCaseDriver, EdgeCaseSpec, EdgeInputClass, drive_edge_case};
use calyx_poly::{POLY_CONFIG_LOADED, PolyConfig, PolyError};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const TOML_PARSE: &str = "POLY_CONFIG_TOML_PARSE";
const ENV_PARSE: &str = "POLY_CONFIG_ENV_PARSE";
const DUPLICATE_DOMAINS: &str = "POLY_CONFIG_DUPLICATE_DOMAINS";
const INFISICAL_REQUIRED: &str = "POLY_CONFIG_INFISICAL_REQUIRED";

#[test]
fn issue20_poly_config_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE20_FSV_ROOT", "poly-issue20-config");
    reset_dir(&root);

    let happy = run_case(
        &root,
        "happy-toml-env",
        EdgeInputClass::HappyPath,
        POLY_CONFIG_LOADED,
        true,
        Scenario::Happy,
    );
    let malformed = run_case(
        &root,
        "edge-malformed-toml",
        EdgeInputClass::InvalidInput,
        TOML_PARSE,
        false,
        Scenario::MalformedToml,
    );
    let missing = run_case(
        &root,
        "edge-missing-required-field",
        EdgeInputClass::EmptyInput,
        TOML_PARSE,
        false,
        Scenario::MissingRequiredField,
    );
    let invalid_env = run_case(
        &root,
        "edge-invalid-env-number",
        EdgeInputClass::InvalidInput,
        ENV_PARSE,
        false,
        Scenario::InvalidEnvNumber,
    );
    let duplicate = run_case(
        &root,
        "edge-duplicate-domains",
        EdgeInputClass::InvalidInput,
        DUPLICATE_DOMAINS,
        false,
        Scenario::DuplicateDomains,
    );
    let unsafe_policy = run_case(
        &root,
        "edge-unsafe-policy",
        EdgeInputClass::InvalidInput,
        INFISICAL_REQUIRED,
        false,
        Scenario::UnsafePolicy,
    );

    for outcome in [
        &happy,
        &malformed,
        &missing,
        &invalid_env,
        &duplicate,
        &unsafe_policy,
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
        "issue": 20,
        "source_of_truth": [
            "physical TOML fixture files",
            "explicit synthetic env override maps",
            "loaded-config.json files read back from disk",
            "per-case before.json, decision.json, after.json, and edge-case-outcome.json"
        ],
        "happy_path": happy,
        "edge_cases": {
            "malformed_toml": malformed,
            "missing_required_field": missing,
            "invalid_env_number": invalid_env,
            "duplicate_domains": duplicate,
            "unsafe_policy": unsafe_policy
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue20_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue20_fsv_root={}", root.display());
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    Happy,
    MalformedToml,
    MissingRequiredField,
    InvalidEnvNumber,
    DuplicateDomains,
    UnsafePolicy,
}

struct Fixture {
    config_path: PathBuf,
    output_path: PathBuf,
    env_vars: Vec<(String, String)>,
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
    let fixture = prepare_fixture(&case_dir, scenario);

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
                let result = PolyConfig::from_toml_file_with_env_vars(
                    &fixture.config_path,
                    fixture.env_vars.clone(),
                );
                if let Ok(config) = &result {
                    write_json(
                        &fixture.output_path,
                        &serde_json::to_value(config).expect("config JSON value"),
                    );
                    let readback =
                        PolyConfig::from_json(&fs::read_to_string(&fixture.output_path).unwrap())
                            .expect("read back loaded config");
                    assert_eq!(readback.calyx_home, config.calyx_home);
                    assert_eq!(readback.panel_version, config.panel_version);
                    assert_eq!(readback.admission.min_p_win, config.admission.min_p_win);
                }
                result
            },
            read_after: || state(&fixture),
            decision_record,
        },
    )
    .expect("drive edge case")
}

fn prepare_fixture(case_dir: &Path, scenario: Scenario) -> Fixture {
    let config_path = case_dir.join("poly.toml");
    let output_path = case_dir.join("loaded-config.json");
    let mut env_vars = Vec::new();
    let toml = match scenario {
        Scenario::Happy => {
            env_vars = vec![
                (
                    "POLY_CONFIG_CALYX_HOME".to_string(),
                    "C:/poly/env-calyx".to_string(),
                ),
                ("POLY_CONFIG_PANEL_VERSION".to_string(), "7".to_string()),
                (
                    "POLY_CONFIG_SNAPSHOT_CADENCE_SECS".to_string(),
                    "120".to_string(),
                ),
                (
                    "POLY_CONFIG_DOMAINS".to_string(),
                    "crypto,politics,sports".to_string(),
                ),
                (
                    "POLY_CONFIG_ADMISSION_MIN_P_WIN".to_string(),
                    "0.91".to_string(),
                ),
                (
                    "POLY_CONFIG_LOCAL_ONLY_ALLOW_FORECAST_AGENTS".to_string(),
                    "false".to_string(),
                ),
            ];
            good_toml()
        }
        Scenario::MalformedToml => "calyx_home = [\n".to_string(),
        Scenario::MissingRequiredField => {
            good_toml().replace("calyx_home = \"C:/poly/calyx\"\n", "")
        }
        Scenario::InvalidEnvNumber => {
            env_vars.push((
                "POLY_CONFIG_PANEL_VERSION".to_string(),
                "not-a-number".to_string(),
            ));
            good_toml()
        }
        Scenario::DuplicateDomains => good_toml().replace(
            "domains = [\"crypto\", \"politics\"]",
            "domains = [\"crypto\", \"crypto\"]",
        ),
        Scenario::UnsafePolicy => good_toml().replace(
            "require_infisical_for_llm = true",
            "require_infisical_for_llm = false",
        ),
    };
    fs::write(&config_path, toml).expect("write TOML fixture");
    Fixture {
        config_path,
        output_path,
        env_vars,
    }
}

fn decision_record(result: calyx_poly::Result<PolyConfig>) -> (String, Value) {
    match result {
        Ok(config) => (
            POLY_CONFIG_LOADED.to_string(),
            json!({
                "code": POLY_CONFIG_LOADED,
                "config": config_summary(&config)
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
    let config_bytes = fs::read(&fixture.config_path).ok();
    let output_bytes = fs::read(&fixture.output_path).ok();
    json!({
        "config_file": {
            "path": fixture.config_path.display().to_string(),
            "exists": fixture.config_path.exists(),
            "bytes": config_bytes.as_ref().map(Vec::len).unwrap_or(0),
            "blake3": config_bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes()))
        },
        "env_vars": fixture.env_vars,
        "loaded_config": {
            "path": fixture.output_path.display().to_string(),
            "exists": fixture.output_path.exists(),
            "bytes": output_bytes.as_ref().map(Vec::len).unwrap_or(0),
            "blake3": output_bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes())),
            "readback": output_bytes.as_ref().and_then(|bytes| {
                serde_json::from_slice::<PolyConfig>(bytes)
                    .ok()
                    .map(|config| config_summary(&config))
            })
        }
    })
}

fn error_code(error: &PolyError) -> String {
    match error {
        PolyError::Config { code, .. } => code.clone(),
        other => format!("UNEXPECTED_POLY_ERROR:{other}"),
    }
}

fn config_summary(config: &PolyConfig) -> Value {
    json!({
        "calyx_home": config.calyx_home,
        "launch_domain": config.launch_domain.slug(),
        "domains": config.domains.iter().map(|domain| domain.slug()).collect::<Vec<_>>(),
        "panel_version": config.panel_version,
        "vault_salt": config.vault_salt,
        "snapshot_cadence_secs": config.snapshot_cadence_secs,
        "admission": {
            "min_p_win": config.admission.min_p_win,
            "target_far": config.admission.target_far,
            "alpha": config.admission.alpha,
            "max_daily_error_score": config.admission.max_daily_error_score,
            "min_grounding_anchors": config.admission.min_grounding_anchors,
            "min_source_derived_evidence": config.admission.min_source_derived_evidence
        },
        "local_only": {
            "allow_forecast_agents": config.local_only.allow_forecast_agents,
            "require_infisical_for_llm": config.local_only.require_infisical_for_llm
        }
    })
}

fn good_toml() -> String {
    r#"calyx_home = "C:/poly/calyx"
launch_domain = "crypto"
domains = ["crypto", "politics"]
panel_version = 1
vault_salt = "poly-config-fsv"
snapshot_cadence_secs = 60

[admission]
min_p_win = 0.90
target_far = 0.10
alpha = 0.05
max_daily_error_score = 10.0
min_grounding_anchors = 50
min_source_derived_evidence = 1

[local_only]
allow_forecast_agents = true
require_infisical_for_llm = true
"#
    .to_string()
}
