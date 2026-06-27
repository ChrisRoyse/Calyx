use super::*;

fn toks(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn parse(parts: &[&str]) -> CliResult<KernelBuildArgs> {
    match super::parse_kernel_build(&toks(parts))? {
        Subcommand::KernelBuild(args) => Ok(args),
        _ => unreachable!("parse_kernel_build must return KernelBuild"),
    }
}

#[test]
fn defaults_apply_when_only_vault_given() {
    let args = parse(&["corpus"]).unwrap();
    assert_eq!(args.vault, "corpus");
    assert_eq!(args.held_out_fraction, DEFAULT_HELD_OUT_FRACTION);
    assert_eq!(args.top_k, DEFAULT_TOP_K);
    assert_eq!(args.min_recall, DEFAULT_MIN_RECALL);
}

#[test]
fn all_flags_parse() {
    let args = parse(&[
        "corpus",
        "--held-out-fraction",
        "0.01",
        "--top-k",
        "5",
        "--min-recall",
        "0.9",
    ])
    .unwrap();
    assert!((args.held_out_fraction - 0.01).abs() < 1e-6);
    assert_eq!(args.top_k, 5);
    assert!((args.min_recall - 0.9).abs() < 1e-6);
}

#[test]
fn missing_vault_fails_closed() {
    let err = super::parse_kernel_build(&[]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn fraction_out_of_range_fails_closed() {
    assert_eq!(
        parse(&["corpus", "--held-out-fraction", "1.5"])
            .unwrap_err()
            .code(),
        "CALYX_CLI_USAGE_ERROR"
    );
    assert_eq!(
        parse(&["corpus", "--min-recall", "nan"])
            .unwrap_err()
            .code(),
        "CALYX_CLI_USAGE_ERROR"
    );
}

#[test]
fn top_k_zero_fails_closed() {
    let err = parse(&["corpus", "--top-k", "0"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn unknown_flag_fails_closed() {
    let err = parse(&["corpus", "--bogus", "1"]).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("unexpected kernel-build flag"));
}
