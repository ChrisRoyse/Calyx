use std::sync::Arc;

use calyx_anneal::{
    AnnealLedger, AnnealLedgerAction, ChangeId, GoodhartChecker, GoodhartLedgerContext,
    GoodhartViolation, HeldOutSet, JTerms, JValue, JWeights, LensContributionDelta, WardGtau,
    record_goodhart_report,
};
use calyx_core::{CalyxError, FixedClock, LensId, Result as CalyxResult};
use calyx_ledger::{ActorId, LedgerAppender, MemoryLedgerStore};
use proptest::prelude::*;

#[test]
fn heldout_regression_fails_and_penalizes_train_gain() {
    let checker = checker(
        HeldOutSet::sealed("held", 8, j(10.0), j(9.5)),
        WardMode::Frac(0.98),
    );
    let report = checker
        .check(&j(10.0), &j(11.0), &[lens_delta(1, 0.20)])
        .expect("check");

    assert!(!report.passed);
    assert_eq!(report.p_goodhart_increment, 1.0);
    assert!(matches!(
        report.violations.as_slice(),
        [GoodhartViolation::HeldOutRegression {
            j_train_delta: 1.0,
            j_heldout_delta
        }] if *j_heldout_delta == -0.5
    ));
}

#[test]
fn dominant_single_lens_delta_fails_cross_lens_check() {
    let checker = checker(
        HeldOutSet::sealed("held", 8, j(10.0), j(10.2)),
        WardMode::Frac(0.98),
    );
    let report = checker
        .check(&j(10.0), &j(11.0), &[lens_delta(7, 0.85)])
        .expect("check");

    assert!(!report.passed);
    assert!(matches!(
        report.violations.as_slice(),
        [GoodhartViolation::CrossLensAnomaly {
            anomalous_lens,
            delta_fraction
        }] if *anomalous_lens == lens(7) && (*delta_fraction - 0.85).abs() < 1e-12
    ));
}

#[test]
fn all_goodhart_checks_pass_when_heldout_gtau_and_lenses_are_clean() {
    let checker = checker(
        HeldOutSet::sealed("held", 8, j(10.0), j(10.2)),
        WardMode::Frac(0.98),
    );
    let report = checker
        .check(
            &j(10.0),
            &j(11.0),
            &[lens_delta(1, 0.40), lens_delta(2, 0.30)],
        )
        .expect("check");

    assert!(report.passed);
    assert!(report.violations.is_empty());
    assert_eq!(report.p_goodhart_increment, 0.0);
    assert!((report.j_heldout_delta.unwrap() - 0.2).abs() < 1e-12);
    assert_eq!(report.in_region_frac, Some(0.98));
}

#[test]
fn empty_heldout_set_skips_heldout_check_with_warning() {
    let checker = checker(HeldOutSet::empty("empty"), WardMode::Frac(0.98));
    let report = checker
        .check(&j(10.0), &j(11.0), &[lens_delta(1, 0.40)])
        .expect("check");

    assert!(report.passed);
    assert_eq!(report.j_heldout_delta, None);
    assert_eq!(
        report.warnings,
        vec!["held_out_set_empty_skip_held_out_check".to_string()]
    );
}

#[test]
fn unavailable_ward_gtau_fails_closed_as_zero_in_region() {
    let checker = checker(
        HeldOutSet::sealed("held", 8, j(10.0), j(10.2)),
        WardMode::Error,
    );
    let report = checker
        .check(&j(10.0), &j(11.0), &[lens_delta(1, 0.40)])
        .expect("check");

    assert!(!report.passed);
    assert!(matches!(
        report.violations.as_slice(),
        [GoodhartViolation::GtauViolation {
            in_region_frac: 0.0,
            threshold: 0.95
        }]
    ));
    assert_eq!(report.in_region_frac, Some(0.0));
    assert!(report.warnings[0].starts_with("ward_gtau_error_treated_as_zero"));
}

#[test]
fn goodhart_ledger_description_passes_secret_redaction() {
    let checker = checker(
        HeldOutSet::sealed("held", 8, j(10.0), j(9.5)),
        WardMode::Frac(0.98),
    );
    let report = checker.check(&j(10.0), &j(11.0), &[]).expect("check");
    let clock = FixedClock::new(1_785_500_424);
    let appender =
        LedgerAppender::open(MemoryLedgerStore::default(), clock).expect("memory ledger appender");
    let mut ledger = AnnealLedger::new(
        appender,
        ActorId::Service("goodhart-redaction-test".to_string()),
    )
    .expect("ledger");

    record_goodhart_report(
        &report,
        GoodhartLedgerContext {
            change_id: ChangeId(424),
            artifact_id: "issue424".to_string(),
            prior_ptr_hash: [0x11; 32],
            candidate_ptr_hash: [0x22; 32],
            ts: 1_785_500_424,
        },
        &mut ledger,
    )
    .expect("ledger write");
    let readback = ledger.read_recent(1).expect("read ledger");

    assert_eq!(readback[0].action, AnnealLedgerAction::GoodhartFailed);
    assert!(readback[0].description.contains("Goodhart report v1"));
}

proptest! {
    #[test]
    fn passed_state_matches_violation_vector(
        before in 0.0f64..100.0,
        train_delta in -5.0f64..5.0,
        heldout_delta in -5.0f64..5.0,
        ward_frac in 0.0f64..1.0,
        lens_fraction in 0.0f64..2.0,
    ) {
        let after = before + train_delta;
        let held_before = before.abs() + 1.0;
        let held_after = held_before + heldout_delta;
        let checker = checker(
            HeldOutSet::sealed("held", 4, j(held_before), j(held_after)),
            WardMode::Frac(ward_frac),
        );
        let lens_delta = train_delta * lens_fraction;
        let report = checker
            .check(&j(before), &j(after), &[self::lens_delta(3, lens_delta)])
            .expect("finite proptest inputs");

        prop_assert_eq!(report.passed, report.violations.is_empty());
        if report.passed {
            prop_assert_eq!(report.p_goodhart_increment, 0.0);
        } else {
            prop_assert!(!report.violations.is_empty());
            prop_assert!(report.p_goodhart_increment >= 0.0);
        }
    }
}

fn checker(held_out_set: HeldOutSet, ward_mode: WardMode) -> GoodhartChecker {
    GoodhartChecker::new(held_out_set, Arc::new(StaticWard { mode: ward_mode }))
}

fn j(value: f64) -> JValue {
    JValue {
        j: value,
        terms: JTerms {
            w1_info: value.abs(),
            w2_n_eff: 0.0,
            w3_sufficiency: 0.0,
            w4_kernel_recall: 0.0,
            w5_oracle_accuracy: 0.0,
            w6_mistake_rate: 0.0,
            w7_compression: 0.0,
            w8_coverage: 0.0,
            p_redundant: 0.0,
            p_ungrounded: 0.0,
            p_goodhart: 0.0,
        },
        dpi_ceiling: value.abs() + 10.0,
        dpi_headroom: 10.0,
        provisional_excluded: 0,
        weights: JWeights::default(),
    }
}

fn lens_delta(byte: u8, delta: f64) -> LensContributionDelta {
    LensContributionDelta {
        lens_id: lens(byte),
        delta,
    }
}

fn lens(byte: u8) -> LensId {
    LensId::from_bytes([byte; 16])
}

struct StaticWard {
    mode: WardMode,
}

enum WardMode {
    Frac(f64),
    Error,
}

impl WardGtau for StaticWard {
    fn in_region_fraction(&self, _held_out_set: &HeldOutSet) -> CalyxResult<Option<f64>> {
        match self.mode {
            WardMode::Frac(value) => Ok(Some(value)),
            WardMode::Error => Err(CalyxError {
                code: "CALYX_WARD_GTAU_UNAVAILABLE",
                message: "test ward unavailable".to_string(),
                remediation: "Goodhart checker must fail closed",
            }),
        }
    }
}
