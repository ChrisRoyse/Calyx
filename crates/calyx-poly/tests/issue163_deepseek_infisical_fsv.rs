use std::path::Path;

use calyx_poly::{
    DeepSeekRuntimeSecrets, InfisicalDeepSeekSource, PolyError,
    agent_secrets::{
        POLY_DEEPSEEK_BASE_URL, POLY_DEEPSEEK_MODEL_PRO, POLY_DEEPSEEK_PROJECT_ID,
        POLY_DEEPSEEK_SECRET_PATH,
    },
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue163_secret_validation_edges_fsv() {
    let root = case_root("edges");
    reset_dir(&root);

    let empty_key = edge_refuses(
        &root,
        "edge-empty-api-key",
        "api key secret value is empty",
        DeepSeekRuntimeSecrets::from_values(
            InfisicalDeepSeekSource::default(),
            String::new(),
            POLY_DEEPSEEK_BASE_URL.to_string(),
            POLY_DEEPSEEK_MODEL_PRO.to_string(),
        ),
        "POLY_INFISICAL_SECRET_EMPTY_OR_MISSING",
    );
    let invalid_project = edge_refuses(
        &root,
        "edge-invalid-project",
        "source project id does not match Poly Infisical project",
        DeepSeekRuntimeSecrets::from_values(
            InfisicalDeepSeekSource {
                project_id: "00000000-0000-0000-0000-000000000000".to_string(),
                ..InfisicalDeepSeekSource::default()
            },
            "sk-x".to_string(),
            POLY_DEEPSEEK_BASE_URL.to_string(),
            POLY_DEEPSEEK_MODEL_PRO.to_string(),
        ),
        "POLY_INFISICAL_PROJECT_MISMATCH",
    );
    let wrong_base_url = edge_refuses(
        &root,
        "edge-wrong-base-url",
        "provider base URL is not the DeepSeek OpenAI-compatible endpoint",
        DeepSeekRuntimeSecrets::from_values(
            InfisicalDeepSeekSource::default(),
            "sk-x".to_string(),
            "https://example.invalid".to_string(),
            POLY_DEEPSEEK_MODEL_PRO.to_string(),
        ),
        "POLY_DEEPSEEK_BASE_URL_INVALID",
    );
    let unsupported_model = edge_refuses(
        &root,
        "edge-unsupported-model",
        "model secret contains an unsupported or deprecated model id",
        DeepSeekRuntimeSecrets::from_values(
            InfisicalDeepSeekSource::default(),
            "sk-x".to_string(),
            POLY_DEEPSEEK_BASE_URL.to_string(),
            "deepseek-chat".to_string(),
        ),
        "POLY_DEEPSEEK_MODEL_UNSUPPORTED",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 163,
            "source_of_truth": "physical FSV JSON edge-case artifacts under the issue163 root",
            "edge_cases": {
                "empty_key": empty_key,
                "invalid_project": invalid_project,
                "wrong_base_url": wrong_base_url,
                "unsupported_model": unsupported_model
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);
}

#[test]
#[ignore = "requires infisical run with real Poly DeepSeek secrets"]
fn issue163_real_infisical_env_happy_path_fsv() {
    let root = case_root("happy");
    reset_dir(&root);

    let artifact = root.join("happy-runtime-readback.json");
    let before = file_state(&artifact);
    let secrets = DeepSeekRuntimeSecrets::from_env().expect("load Infisical-injected secrets");
    let debug = format!("{secrets:?}");
    assert!(
        !debug.contains("sk-"),
        "Debug output must not contain key material"
    );
    let metadata = secrets.metadata();
    assert_eq!(metadata.project_id, POLY_DEEPSEEK_PROJECT_ID);
    assert_eq!(metadata.secret_path, POLY_DEEPSEEK_SECRET_PATH);
    assert_eq!(metadata.key_length, 35);
    assert!(metadata.key_has_sk_prefix);
    assert_eq!(metadata.key_sha256_prefix, "8e7788955344");
    assert_eq!(metadata.base_url, POLY_DEEPSEEK_BASE_URL);
    assert_eq!(metadata.model, POLY_DEEPSEEK_MODEL_PRO);

    write_json(
        &artifact,
        &json!({
            "issue": 163,
            "trigger": "process launched through infisical run with Poly project/env/path",
            "source_of_truth": "Infisical-injected process environment, persisted as non-secret metadata",
            "before": before,
            "metadata": metadata,
            "debug_redacted": !debug.contains("sk-")
        }),
    );
    let after = read_json(&artifact);
    assert_eq!(after["metadata"]["key_present"], json!(true));
    assert_eq!(
        after["metadata"]["key_sha256_prefix"],
        json!("8e7788955344")
    );
    assert_eq!(after["metadata"]["model"], json!(POLY_DEEPSEEK_MODEL_PRO));
    assert_eq!(after["debug_redacted"], json!(true));

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 163,
            "source_of_truth": "Infisical-injected process environment plus readback JSON artifact",
            "happy_path": {
                "before": before,
                "after_file": file_state(&artifact),
                "readback": after
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);
}

fn edge_refuses(
    root: &Path,
    name: &str,
    trigger: &str,
    result: Result<DeepSeekRuntimeSecrets, PolyError>,
    expected_code: &str,
) -> Value {
    let artifact = root.join(format!("{name}-runtime-readback.json"));
    let before = file_state(&artifact);
    let error = result.expect_err("edge case must fail closed");
    let code = match error {
        PolyError::AgentSecret { code, .. } => code,
        other => panic!("unexpected error variant: {other:?}"),
    };
    assert_eq!(code, expected_code);
    let after = file_state(&artifact);
    assert_eq!(before, after);
    let evidence = json!({
        "trigger": trigger,
        "expected_code": expected_code,
        "actual_code": code,
        "before": before,
        "after": after
    });
    write_json(&root.join(format!("{name}-edge.json")), &evidence);
    evidence
}

fn case_root(case_name: &str) -> std::path::PathBuf {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE163_FSV_ROOT", "poly-issue163-deepseek");
    if keep_root {
        println!("poly_issue163_fsv_root={}", root.display());
    }
    root.join(case_name)
}

fn file_state(path: &Path) -> Value {
    let bytes = std::fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len),
        "blake3": bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes()))
    })
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("read JSON source of truth"))
        .expect("decode JSON source of truth")
}
