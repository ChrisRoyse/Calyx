use std::fs;

use serde_json::Value;

use super::args::StreamMode;
use super::write;

#[path = "tests/support.rs"]
#[allow(clippy::duplicate_mod)]
#[allow(dead_code)]
mod support;

use support::{Fixture, staging_dir, write_bits_with_panel_names};

#[test]
fn diagnostic_mode_can_measure_unadmitted_lens() {
    let fixture = Fixture::new("stream-fbin-diagnostic-unadmitted", 10, 10, 50);
    write_bits_with_panel_names(
        &fixture.bits,
        10,
        9,
        (0..9).map(|idx| format!("lens-{idx}")).collect(),
        1.25,
        "passed",
        1.0,
    );
    let args = fixture.json_args(8, StreamMode::Diagnostic);

    write::run(&args).unwrap();

    let report: Value =
        serde_json::from_slice(&fs::read(fixture.out.join("stream_fbin_report.json")).unwrap())
            .unwrap();
    assert_eq!(report["pre_encode_gate"]["mode"], "diagnostic");
    assert_eq!(report["pre_encode_gate"]["diagnostic_only"], true);
    assert_eq!(
        report["pre_encode_gate"]["admitted_lenses"]
            .as_array()
            .unwrap()
            .len(),
        9
    );
    assert_eq!(
        report["pre_encode_gate"]["streamed_lenses"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(report["lens_roster"].as_array().unwrap().len(), 10);
    assert!(fixture.out.join("partitioned_rrf_plan.json").exists());
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn gate_mode_rejects_unadmitted_lens_even_when_panel_names_include_it() {
    let fixture = Fixture::new("stream-fbin-gate-unadmitted", 10, 10, 50);
    fixture.rewrite_a37(9, None, 0.2);
    write_bits_with_panel_names(
        &fixture.bits,
        10,
        9,
        (0..10).map(|idx| format!("lens-{idx}")).collect(),
        1.25,
        "passed",
        1.0,
    );
    let args = fixture.args(8);

    let error = write::run(&args).unwrap_err();

    assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_REJECTED");
    assert!(!fixture.out.exists());
    assert!(!staging_dir(&fixture).exists());
    let _ = fs::remove_dir_all(fixture.root);
}
