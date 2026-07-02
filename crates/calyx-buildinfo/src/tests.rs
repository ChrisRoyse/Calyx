//! FSV for the embedded build identity against the real repository checkout.

use super::{BuildInfo, compute_for_dir};
use std::process::Command;

fn real_git(args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("spawn git");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout)
        .expect("utf-8")
        .trim()
        .to_string()
}

#[test]
fn compute_for_dir_matches_real_head() {
    let info = compute_for_dir(env!("CARGO_MANIFEST_DIR")).expect("compute in real checkout");
    assert_eq!(info.git_sha, real_git(&["rev-parse", "HEAD"]));
    assert_eq!(
        info.git_commit_unix_secs.to_string(),
        real_git(&["show", "-s", "--format=%ct", "HEAD"])
    );
    assert_eq!(
        info.git_dirty,
        !real_git(&["status", "--porcelain", "--untracked-files=no"]).is_empty()
    );
    assert!(
        info.rerun_paths
            .iter()
            .any(|path| path.ends_with("HEAD") && path.exists()),
        "rerun paths must watch an existing HEAD file: {:?}",
        info.rerun_paths
    );
}

#[test]
fn compute_for_dir_outside_checkout_errors() {
    let outside = std::env::temp_dir();
    let error = compute_for_dir(outside.to_str().expect("utf-8 temp dir"))
        .expect_err("a non-checkout directory must not produce an identity");
    assert!(
        error.contains("CALYX_BUILD_INFO_GIT_UNAVAILABLE"),
        "error must carry the unavailable code: {error}"
    );
}

#[test]
fn from_embedded_accepts_valid_values() {
    let info = BuildInfo::from_embedded(
        "calyx-cli",
        "0.1.0",
        "cc6f672750530c0246bcb05d0ef9d633f7c095a2",
        "0",
        "1751407200",
        "cuda,default",
    )
    .expect("valid embedded values");
    assert_eq!(info.git_sha, "cc6f672750530c0246bcb05d0ef9d633f7c095a2");
    assert!(!info.git_dirty);
    assert_eq!(info.git_commit_unix_secs, 1_751_407_200);
    assert_eq!(info.features, vec!["cuda", "default"]);
}

#[test]
fn from_embedded_accepts_empty_feature_list() {
    // calyx-mcp declares no cargo features; an empty embedded list is valid.
    let info = BuildInfo::from_embedded(
        "calyx-mcp",
        "0.1.0",
        "cc6f672750530c0246bcb05d0ef9d633f7c095a2",
        "0",
        "1751407200",
        "",
    )
    .expect("empty feature list is valid");
    assert!(info.features.is_empty());
}

#[test]
fn from_embedded_rejects_malformed_feature_names() {
    for raw in ["CUDA", "cuda,", "cuda,,default", "cu da"] {
        let error = BuildInfo::from_embedded(
            "calyx-cli",
            "0.1.0",
            "cc6f672750530c0246bcb05d0ef9d633f7c095a2",
            "0",
            "1751407200",
            raw,
        )
        .expect_err("malformed feature list must be rejected");
        assert!(error.contains("CALYX_BUILD_INFO_INVALID"), "{raw}: {error}");
        assert!(error.contains("feature name"), "{raw}: {error}");
    }
}

#[test]
fn features_from_env_keys_unmangles_sorts_and_dedups() {
    let keys = [
        "CARGO_FEATURE_CUDA",
        "CARGO_FEATURE_DEFAULT",
        "CARGO_FEATURE_CUDA_SEXTANT",
        "CARGO_MANIFEST_DIR",
        "PATH",
        "CARGO_FEATURE_CUDA",
    ]
    .into_iter()
    .map(str::to_string);

    assert_eq!(
        super::features_from_env_keys(keys),
        vec!["cuda", "cuda-sextant", "default"]
    );
}

#[test]
fn features_from_env_keys_without_features_is_empty() {
    let keys = ["CARGO_MANIFEST_DIR", "PATH"]
        .into_iter()
        .map(str::to_string);
    assert!(super::features_from_env_keys(keys).is_empty());
}

#[test]
fn from_embedded_rejects_short_sha() {
    let error = BuildInfo::from_embedded("calyx-cli", "0.1.0", "cc6f6727", "0", "0", "")
        .expect_err("short sha must be rejected");
    assert!(error.contains("CALYX_BUILD_INFO_INVALID"), "{error}");
}

#[test]
fn from_embedded_rejects_uppercase_sha() {
    let error = BuildInfo::from_embedded(
        "calyx-cli",
        "0.1.0",
        "CC6F672750530C0246BCB05D0EF9D633F7C095A2",
        "0",
        "0",
        "",
    )
    .expect_err("uppercase sha must be rejected");
    assert!(error.contains("CALYX_BUILD_INFO_INVALID"), "{error}");
}

#[test]
fn from_embedded_rejects_bad_dirty_flag() {
    let error = BuildInfo::from_embedded(
        "calyx-cli",
        "0.1.0",
        "cc6f672750530c0246bcb05d0ef9d633f7c095a2",
        "yes",
        "0",
        "",
    )
    .expect_err("non 0/1 dirty flag must be rejected");
    assert!(error.contains("dirty flag"), "{error}");
}

#[test]
fn from_embedded_rejects_bad_timestamp() {
    let error = BuildInfo::from_embedded(
        "calyx-cli",
        "0.1.0",
        "cc6f672750530c0246bcb05d0ef9d633f7c095a2",
        "0",
        "not-a-number",
        "",
    )
    .expect_err("non-numeric timestamp must be rejected");
    assert!(error.contains("commit timestamp"), "{error}");
}
