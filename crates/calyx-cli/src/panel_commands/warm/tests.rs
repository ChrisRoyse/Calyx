use super::*;

#[test]
fn resident_vram_estimate_ceilings_declared_bytes() {
    let declared = 10_u64 * 1024 * 1024 * 1024;
    assert_eq!(estimate_resident_vram_mib(declared, 2100), 21 * 1024);
}

#[test]
fn parses_warm_limit_flags() {
    let flags = Flags::parse(&[
        "--template".to_string(),
        "blackwell-42".to_string(),
        "--max-resident-vram-mib".to_string(),
        "22528".to_string(),
        "--resident-overhead-multiplier".to_string(),
        "2.1".to_string(),
        "--max-load-secs".to_string(),
        "30".to_string(),
        "--load-parallelism".to_string(),
        "4".to_string(),
    ])
    .unwrap();

    assert_eq!(flags.template.as_deref(), Some("blackwell-42"));
    assert_eq!(flags.max_resident_vram_mib, Some(22 * 1024));
    assert_eq!(flags.resident_overhead_multiplier_milli, Some(2100));
    assert_eq!(flags.max_load_secs, Some(30));
    assert_eq!(flags.load_parallelism, Some(4));
}

#[test]
fn warm_defaults_use_sixty_second_template_parallel_readiness() {
    let flags = Flags::parse(&["--template".to_string(), "blackwell-42".to_string()]).unwrap();

    assert_eq!(flags.max_load_secs.unwrap_or(DEFAULT_MAX_LOAD_SECS), 60);
    assert_eq!(flags.load_parallelism, None);
    assert_eq!(default_load_parallelism(23), 8);
    assert_eq!(default_load_parallelism(4), 4);
    assert_eq!(default_load_parallelism(0), 1);
}

#[test]
fn rejects_zero_load_parallelism() {
    let error = Flags::parse(&[
        "--template".to_string(),
        "blackwell-42".to_string(),
        "--load-parallelism".to_string(),
        "0".to_string(),
    ])
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.message().contains("--load-parallelism must be > 0"));
}

#[test]
fn rejects_non_positive_resident_multiplier() {
    let error = parse_multiplier_milli("0").unwrap_err();
    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        error
            .message()
            .contains("--resident-overhead-multiplier must be a positive")
    );
}
