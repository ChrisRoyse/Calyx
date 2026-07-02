//! Arg-surface tests for the SPANN closure/pruning knobs (#712/#550): the
//! query-aware pruning epsilon on `bench partitioned-search` and the balance
//! cap / assignment epsilon / replication / RNG-rule flags on
//! `build-partitioned-vault`. All invalid values must fail closed as usage
//! errors before any vault or search work starts.

use super::args::SearchArgs;
use super::build::BuildArgs;

fn strings(items: impl IntoIterator<Item = &'static str>) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}

#[test]
fn partitioned_search_parses_and_validates_pruning_epsilon() {
    let args = strings(["--vault", "vault", "--pruning-epsilon", "7.0"]);
    let parsed = SearchArgs::parse(&args).unwrap();
    assert_eq!(parsed.pruning_epsilon, Some(7.0));

    let default = SearchArgs::parse(&strings(["--vault", "vault"])).unwrap();
    assert_eq!(default.pruning_epsilon, None);

    for bad in ["-0.5", "NaN", "inf"] {
        let args = strings(["--vault", "vault", "--pruning-epsilon", bad]);
        let err = match SearchArgs::parse(&args) {
            Ok(_) => panic!("invalid pruning epsilon {bad} accepted"),
            Err(err) => err,
        };
        assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
        assert!(
            err.message()
                .contains("--pruning-epsilon expects a finite value >= 0")
        );
    }
}

#[test]
fn partitioned_build_parses_spann_closure_flags_and_rejects_bad_values() {
    let args = strings([
        "--vault",
        "vault",
        "--n-cx",
        "1000",
        "--regions",
        "8",
        "--balance-cap",
        "256",
        "--assignment-epsilon",
        "1.5",
        "--max-replication",
        "4",
        "--rng-rule",
        "false",
        "--rng-factor",
        "4.0",
    ]);
    let parsed = BuildArgs::parse(&args).unwrap();
    assert_eq!(parsed.p.balance_cap, Some(256));
    assert_eq!(parsed.p.assignment_boundary_epsilon, 1.5);
    assert_eq!(parsed.p.assignment_max_replication, 4);
    assert!(!parsed.p.assignment_rng_rule);
    assert_eq!(parsed.p.assignment_rng_factor, 4.0);

    let defaults = BuildArgs::parse(&strings([
        "--vault",
        "vault",
        "--n-cx",
        "10",
        "--regions",
        "2",
    ]))
    .unwrap();
    assert_eq!(defaults.p.balance_cap, None);
    assert_eq!(defaults.p.assignment_boundary_epsilon, 0.10);
    // Replication defaults OFF (#1129): measured no-op under the strict RNG
    // rule at real SpaceV geometries; opt in explicitly when trading bytes
    // for probes.
    assert_eq!(defaults.p.assignment_max_replication, 1);
    assert!(defaults.p.assignment_rng_rule);
    assert_eq!(defaults.p.assignment_rng_factor, 1.0);

    for (flag, value, expect) in [
        ("--balance-cap", "0", "--balance-cap must be > 0"),
        (
            "--assignment-epsilon",
            "-1",
            "--assignment-epsilon must be finite and >= 0",
        ),
        ("--max-replication", "0", "--max-replication must be >= 1"),
        ("--rng-rule", "yes", "--rng-rule expects true or false"),
        ("--rng-factor", "0", "--rng-factor must be finite and > 0"),
        ("--rng-factor", "-2", "--rng-factor must be finite and > 0"),
        ("--rng-factor", "NaN", "--rng-factor must be finite and > 0"),
    ] {
        let args = strings([
            "--vault",
            "vault",
            "--n-cx",
            "1000",
            "--regions",
            "8",
            flag,
            value,
        ]);
        let err = match BuildArgs::parse(&args) {
            Ok(_) => panic!("{flag} {value} accepted"),
            Err(err) => err,
        };
        assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
        assert!(err.message().contains(expect), "got: {}", err.message());
    }
}
