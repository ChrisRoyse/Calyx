//! FSV for `calyx build-info` (#1108): run the real built binary and verify
//! the printed identity against the git checkout that produced it.

use std::process::Command;

fn run_calyx(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args(args)
        .output()
        .expect("spawn calyx binary")
}

#[test]
fn build_info_reports_the_checkout_head() {
    let output = run_calyx(&["build-info"]);
    assert!(
        output.status.success(),
        "build-info must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("build-info prints JSON");

    let expected = calyx_buildinfo::compute_for_dir(env!("CARGO_MANIFEST_DIR"))
        .expect("compute identity in the real checkout");
    assert_eq!(report["binary"], "calyx");
    assert_eq!(report["package"], "calyx-cli");
    assert_eq!(report["git_sha"], expected.git_sha.as_str());
    assert_eq!(
        report["git_commit_unix_secs"].as_u64().expect("timestamp"),
        expected.git_commit_unix_secs
    );
    let executable = report["executable"].as_str().expect("executable path");
    assert!(
        std::path::Path::new(executable).is_file(),
        "reported executable must exist on disk: {executable}"
    );

    // #1116: the enabled cargo feature set must be embedded so deploy gates
    // can verify artifact configuration (e.g. calyxd requires cuda). The
    // exact contents depend on how this test build was invoked; the contract
    // is a present array of lowercase hyphenated names matching what this
    // test process itself was compiled with.
    let features = report["features"]
        .as_array()
        .expect("build-info must report a features array");
    for feature in features {
        let name = feature.as_str().expect("feature names are strings");
        assert!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "feature names must be lowercase hyphenated: {name:?}"
        );
    }
    assert_eq!(
        features
            .iter()
            .map(|feature| feature.as_str().expect("string"))
            .collect::<Vec<_>>(),
        calyx_buildinfo::build_info!().features,
        "binary-reported features must match the features this test was compiled with"
    );
}

#[test]
fn build_info_rejects_extra_arguments() {
    let output = run_calyx(&["build-info", "--json"]);
    assert!(
        !output.status.success(),
        "extra arguments must fail, got stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("takes no arguments"),
        "stderr must name the rejection: {stderr}"
    );
}

#[test]
fn build_info_help_prints_usage() {
    let output = run_calyx(&["build-info", "--help"]);
    assert!(
        output.status.success(),
        "--help must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("calyx build-info"),
        "usage text must mention the command: {stdout}"
    );
}
