use std::process::Command;

#[test]
fn issue197_cli_help_exits_zero_without_failure_payloads() {
    let cases = [
        (
            "calyx-poly-raw-source-sample",
            env!("CARGO_BIN_EXE_calyx-poly-raw-source-sample"),
            "per-source network/read timeout",
        ),
        (
            "calyx-poly-large-corpus-sample",
            env!("CARGO_BIN_EXE_calyx-poly-large-corpus-sample"),
            "per-source network/read timeout",
        ),
        (
            "calyx-poly-schema-derive",
            env!("CARGO_BIN_EXE_calyx-poly-schema-derive"),
            "usage: calyx-poly-schema-derive",
        ),
        (
            "calyx-poly-file-size-lint",
            env!("CARGO_BIN_EXE_calyx-poly-file-size-lint"),
            "usage: calyx-poly-file-size-lint",
        ),
    ];

    for (name, bin, expected_text) in cases {
        let output = Command::new(bin)
            .arg("--help")
            .output()
            .unwrap_or_else(|err| panic!("run {name} --help: {err}"));
        assert!(output.status.success(), "{name} --help should exit 0");
        let stdout = String::from_utf8(output.stdout).expect("help stdout should be utf8");
        let stderr = String::from_utf8(output.stderr).expect("help stderr should be utf8");
        assert!(
            stdout.contains(&format!("usage: {name}")),
            "{name} stdout should contain its usage line: {stdout}"
        );
        assert!(
            stdout.contains(expected_text),
            "{name} stdout should explain expected semantics: {stdout}"
        );
        assert!(
            !stdout.contains("\"ok\":false"),
            "{name} stdout must not contain a failure payload"
        );
        assert!(stderr.trim().is_empty(), "{name} stderr was {stderr:?}");
    }
}
