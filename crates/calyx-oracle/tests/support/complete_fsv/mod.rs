mod fixture;

use calyx_oracle::{DomainId, OracleSelfConsistency, complete_with_assay_and_region};
use fixture::{
    FakeAssay, FixedAnneal, MemoryLedger, RegionFixture, assert_full_copy, assert_tags, case,
    complete_ok, constellation, cosines_to_full, error_case, make_panel, set, tag_scan_case,
    write_outputs,
};

pub fn run_ph51_complete_fsv() {
    let _rng = calyx_testkit::seeded_rng(42);
    let clock = calyx_testkit::fixed_clock();
    let panel7 = make_panel(7);
    let cx_full = constellation(&panel7, &[1, 2, 3, 4, 5, 6, 7]);
    let cx_partial = constellation(&panel7, &[1, 2, 3]);
    let region = RegionFixture::from_full(&panel7, &cx_full);
    let mut outputs = Vec::new();

    let imputation = complete_ok(
        &cx_partial,
        &panel7,
        &[1, 2, 3],
        &[4, 5, 6, 7],
        &region,
        0.91,
        &clock,
    );
    let imputation_cos = cosines_to_full(&imputation, &cx_full, &panel7, &[4, 5, 6, 7]);
    assert!(imputation_cos.iter().all(|value| *value >= 0.90));
    assert_tags(&imputation, &[1, 2, 3], &[4, 5, 6, 7]);
    outputs.push(case("imputation", &imputation, imputation_cos));

    let prediction_panel = make_panel(4);
    let prediction_full = constellation(&prediction_panel, &[1, 2, 3, 4]);
    let prediction_partial = constellation(&prediction_panel, &[1, 2, 3]);
    let prediction_region = RegionFixture::from_full(&prediction_panel, &prediction_full);
    let prediction = complete_ok(
        &prediction_partial,
        &prediction_panel,
        &[1, 2, 3],
        &[4],
        &prediction_region,
        0.37,
        &clock,
    );
    assert!(prediction.confidence <= 0.37);
    assert_tags(&prediction, &[1, 2, 3], &[4]);
    outputs.push(case(
        "prediction",
        &prediction,
        cosines_to_full(&prediction, &prediction_full, &prediction_panel, &[4]),
    ));

    let cause_panel = make_panel(2);
    let cause_full = constellation(&cause_panel, &[1, 2]);
    let cause_partial = constellation(&cause_panel, &[2]);
    let cause_region = RegionFixture::from_full(&cause_panel, &cause_full);
    let abduction = complete_ok(
        &cause_partial,
        &cause_panel,
        &[2],
        &[1],
        &cause_region,
        0.88,
        &clock,
    );
    let cause_cos = cosines_to_full(&abduction, &cause_full, &cause_panel, &[1]);
    assert!(cause_cos[0] >= 0.85 - 0.01);
    assert_tags(&abduction, &[2], &[1]);
    outputs.push(case("abduction", &abduction, cause_cos));

    let refusing_region = RegionFixture::from_full(&panel7, &cx_full);
    let refusing_ledger = MemoryLedger::default();
    let insufficient = complete_with_assay_and_region(
        &FakeAssay::insufficient(),
        &refusing_ledger,
        &cx_partial,
        &panel7,
        DomainId::from("synthetic"),
        set(&[1, 2, 3]),
        set(&[4, 5, 6, 7]),
        &refusing_region,
        OracleSelfConsistency::measured(0.0, 1.0),
        &FixedAnneal,
        &clock,
    )
    .expect_err("insufficient panel");
    assert_eq!(insufficient.code(), calyx_oracle::CALYX_ORACLE_INSUFFICIENT);
    assert_eq!(refusing_region.calls.get(), 0);
    assert_eq!(refusing_ledger.writes(), 0);
    outputs.push(error_case(
        "insufficient_refused",
        insufficient.code(),
        refusing_region.calls.get(),
        refusing_ledger.writes(),
    ));

    outputs.push(tag_scan_case(
        "tag_discipline_scan",
        [&imputation, &prediction, &abduction],
        [
            (&[1, 2, 3][..], &[4, 5, 6, 7][..]),
            (&[1, 2, 3][..], &[4][..]),
            (&[2][..], &[1][..]),
        ],
    ));

    let all_clamped = complete_ok(
        &cx_full,
        &panel7,
        &[1, 2, 3, 4, 5, 6, 7],
        &[],
        &region,
        1.0,
        &clock,
    );
    assert_full_copy(&all_clamped, &cx_full, &panel7);
    outputs.push(case(
        "edge_all_clamped",
        &all_clamped,
        cosines_to_full(&all_clamped, &cx_full, &panel7, &[1, 2, 3, 4, 5, 6, 7]),
    ));

    let zero_region = RegionFixture::default();
    let zero_region_error = complete_with_assay_and_region(
        &FakeAssay::sufficient(),
        &MemoryLedger::default(),
        &cx_partial,
        &panel7,
        DomainId::from("synthetic"),
        set(&[1, 2, 3]),
        set(&[4, 5, 6, 7]),
        &zero_region,
        OracleSelfConsistency::measured(0.0, 1.0),
        &FixedAnneal,
        &clock,
    )
    .expect_err("zero region");
    assert_eq!(
        zero_region_error.code(),
        calyx_oracle::CALYX_ORACLE_ENERGY_EMPTY_REGION
    );
    outputs.push(error_case(
        "edge_zero_region",
        zero_region_error.code(),
        zero_region.calls.get(),
        0,
    ));

    write_outputs(&outputs);
    println!("{}", serde_json::to_string_pretty(&outputs).unwrap());
}
