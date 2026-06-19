use super::run;

#[test]
fn lens_dispatch_rejects_unknown_subcommand() {
    let error = run("missing", &[]).unwrap_err();

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        error
            .message()
            .contains("expected add, list, card, explain")
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
