use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{
    Asymmetry, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState,
};
use serde::Serialize;
use serde_json::json;

mod reverse_query;
mod super_intel;

use reverse_query::reverse_cases;
use super_intel::{SuperFixtures, super_case};

const SWE_DOMAIN: &str = "swe_bench_lite_form_only";
const PASS_DOMAIN: &str = "ph50_exit_all_pass";
const GOODHART_DOMAIN: &str = "ph50_exit_goodhart_down";
const REVERSE_DOMAIN: &str = "ph50_exit_reverse";

pub fn run_issue439_fsv() {
    let root = fsv_root();
    assert!(
        !root.exists(),
        "fresh FSV root required: {}",
        root.display()
    );
    fs::create_dir_all(&root).expect("create FSV root");
    let tmp = std::env::temp_dir();
    let clock = calyx_core::FixedClock::new(439_042);
    let panel = panel();
    let held_out = calyx_oracle::HeldOutSplit::new(
        "ph50-exit-held-out",
        vec![cx(1), cx(2)],
        vec![cx(3), cx(4)],
    );

    let fail = super_case(
        root.join("super-failing-vault"),
        SWE_DOMAIN,
        &panel,
        &held_out,
        &clock,
        SuperFixtures::panel_fails(),
    );
    assert_eq!(
        fail.report.failing_tier,
        Some(calyx_oracle::Tier::PanelSufficient)
    );
    assert!(!fail.report.overall);
    assert!(
        fail.report
            .cheapest_fix
            .as_deref()
            .unwrap_or("")
            .contains("lens")
    );
    write_json(&root.join("ph50_super_intel.json"), &fail.report);
    write_json(&tmp.join("ph50_super_intel.json"), &fail.report);

    let pass = super_case(
        root.join("super-all-pass-vault"),
        PASS_DOMAIN,
        &panel,
        &held_out,
        &clock,
        SuperFixtures::all_pass(),
    );
    assert!(pass.report.overall);
    assert_eq!(pass.report.failing_tier, None);
    write_json(&root.join("ph50_super_intel_all_pass.json"), &pass.report);

    let goodhart = super_case(
        root.join("super-goodhart-fail-vault"),
        GOODHART_DOMAIN,
        &panel,
        &held_out,
        &clock,
        SuperFixtures::goodhart_source_fails(),
    );
    assert_eq!(
        goodhart.report.failing_tier,
        Some(calyx_oracle::Tier::GoodhartDefended)
    );
    assert!(
        goodhart
            .report
            .cheapest_fix
            .as_deref()
            .unwrap_or("")
            .contains("CALYX_SYNTHETIC_GOODHART_DOWN")
    );
    write_json(
        &root.join("ph50_super_intel_goodhart_failure.json"),
        &goodhart.report,
    );

    let reverse = reverse_cases(root.join("reverse-vault"), &clock);
    write_json(&root.join("ph50_reverse_query.json"), &reverse);
    write_json(&tmp.join("ph50_reverse_query.json"), &reverse);

    write_json(
        &root.join("manifest.json"),
        &json!({
            "issue": 439,
            "tmp_super_intel": tmp.join("ph50_super_intel.json"),
            "tmp_reverse_query": tmp.join("ph50_reverse_query.json"),
            "expected": {
                "failing_tier": "panel_sufficient",
                "all_pass_overall": true,
                "goodhart_failure_tier": "goodhart_defended",
                "planted_cause": "code_change_X",
                "planted_provisional": false,
                "ungrounded_provisional": true,
                "missing_error": "CALYX_ORACLE_DOMAIN_NOT_FOUND"
            },
            "super_ledger": [fail.ledger_b3, pass.ledger_b3, goodhart.ledger_b3],
            "reverse_ledger": [reverse.planted_ledger_b3, reverse.ungrounded_ledger_b3]
        }),
    );
}

fn panel() -> Panel {
    Panel {
        version: 50,
        slots: vec![slot(1)],
        created_at: 439,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn slot(id: u16) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("form-only-slot-{id}")),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Code,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("form-only".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: std::collections::BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 50,
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize json"),
    )
    .expect("write json");
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_FSV_ROOT")
}

fn vault_id() -> calyx_core::VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn cx(seed: u8) -> calyx_core::CxId {
    calyx_core::CxId::from_bytes([seed; 16])
}
