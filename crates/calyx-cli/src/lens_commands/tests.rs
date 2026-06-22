use super::run;

#[test]
fn lens_dispatch_rejects_unknown_subcommand() {
    let error = run("missing", &[]).unwrap_err();

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        error
            .message()
            .contains("expected add, list, remove, card, explain")
    );
}

#[test]
fn commission_rejects_unsupported_runtime_before_side_effects() {
    let error = run(
        "commission",
        &[
            "--hf".to_string(),
            "sentence-transformers/all-MiniLM-L6-v2".to_string(),
            "--runtime".to_string(),
            "unsupported".to_string(),
        ],
    )
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("unsupported --runtime"));
}

#[test]
fn commission_accepts_onnx_fp32_runtime_name() {
    let error = run(
        "commission",
        &["--runtime".to_string(), "onnx-fp32".to_string()],
    )
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("--hf is required"));
}

#[test]
fn commission_accepts_onnx_colbert_runtime_name() {
    let error = run(
        "commission",
        &["--runtime".to_string(), "onnx-colbert".to_string()],
    )
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("--hf is required"));
}

#[test]
fn commission_accepts_fastembed_qwen3_runtime_name() {
    let error = run(
        "commission",
        &["--runtime".to_string(), "fastembed-qwen3".to_string()],
    )
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("--hf is required"));
}

#[test]
fn commission_rejects_zero_max_batch() {
    let error = run(
        "commission",
        &[
            "--runtime".to_string(),
            "onnx-fp32".to_string(),
            "--max-batch".to_string(),
            "0".to_string(),
        ],
    )
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("--max-batch must be > 0"));
}
