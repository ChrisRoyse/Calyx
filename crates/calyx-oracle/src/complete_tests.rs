use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{FixedClock, VaultStore};
use calyx_ledger::{EntryKind, decode};
use proptest::prelude::*;

use super::*;
use crate::{CALYX_ORACLE_ENERGY_EMPTY_REGION, CALYX_ORACLE_INSUFFICIENT};

#[path = "complete_test_support.rs"]
mod support;
use support::*;

#[test]
fn complete_tags_three_clamped_and_four_free_slots() {
    let fixture = Fixture::new(7).with_dense_slots();
    let clamp = set(&[1, 2, 3]);
    let free = set(&[4, 5, 6, 7]);
    let result = run_complete(&fixture, clamp, free, 0.67).expect("complete");

    assert_eq!(result.measured_slots().len(), 3);
    assert_eq!(result.inferred_slots().len(), 4);
    assert_eq!(result.provisional_slots().len(), 0);
    assert!(result.confidence <= 0.67);
    assert!(result.converged);
    println!(
        "happy_counts measured={} inferred={} confidence={:.3} ledger_seq={}",
        result.measured_slots().len(),
        result.inferred_slots().len(),
        result.confidence,
        result.provenance.seq
    );
}

#[test]
fn abduction_mode_recovers_planted_cause_slot() {
    let fixture = Fixture::new(2).with_slot(2, vec![0.0, 1.0]);
    let result = run_complete(&fixture, set(&[2]), set(&[1]), 1.0).expect("complete");
    let cause = result
        .filled_cx
        .iter()
        .find(|slot| slot.lens_id == lens(1))
        .expect("cause slot");

    assert!(cosine(&cause.vector, &[1.0, 0.0]) >= 0.99);
    assert_eq!(cause.tag, SlotTag::Inferred);
    println!(
        "abduction cause_cos={:.3} tag={:?}",
        cosine(&cause.vector, &[1.0, 0.0]),
        cause.tag
    );
}

#[test]
fn imputation_mode_converges_to_known_free_slots() {
    let fixture = Fixture::new(7).with_dense_slots();
    let result =
        run_complete(&fixture, set(&[1, 2, 3, 4, 5]), set(&[6, 7]), 1.0).expect("complete");

    for slot in result.inferred_slots() {
        let expected = expected_vector(slot.lens_id);
        assert!(cosine(&slot.vector, &expected) >= 0.99);
    }
    assert!(result.confidence <= 1.0);
    println!(
        "imputation inferred={} energy={:.6}",
        result.inferred_slots().len(),
        result.energy
    );
}

#[test]
fn all_slots_clamped_returns_measured_copy() {
    let fixture = Fixture::new(3).with_dense_slots();
    let result = run_complete(&fixture, set(&[1, 2, 3]), set(&[]), 1.0).expect("complete");

    assert_eq!(result.measured_slots().len(), 3);
    assert_eq!(result.inferred_slots().len(), 0);
    assert_eq!(result.confidence, 1.0);
    for slot in result.measured_slots() {
        assert_eq!(slot.vector, expected_vector(slot.lens_id));
    }
    println!(
        "edge_all_clamped measured={} confidence={}",
        result.measured_slots().len(),
        result.confidence
    );
}

#[test]
fn all_slots_free_is_valid_and_capped() {
    let fixture = Fixture::new(3);
    let result = run_complete(&fixture, set(&[]), set(&[1, 2, 3]), 0.42).expect("complete");

    assert_eq!(result.measured_slots().len(), 0);
    assert_eq!(result.inferred_slots().len(), 3);
    assert!(result.confidence <= 0.42);
    println!(
        "edge_all_free inferred={} confidence={:.3}",
        result.inferred_slots().len(),
        result.confidence
    );
}

#[test]
fn zero_region_members_fail_closed() {
    let fixture = Fixture::new(1);
    let mut region = MapRegion::default();
    region.members.clear();
    let error = complete_with_assay_and_region(
        &FakeAssay::sufficient(),
        &MemoryLedger::default(),
        &fixture.cx,
        &fixture.panel,
        DomainId::from("synthetic"),
        set(&[]),
        set(&[1]),
        &region,
        OracleSelfConsistency::measured(0.0, 1.0),
        &FixedAnneal,
        &fixture.clock,
    )
    .expect_err("empty region should fail");

    assert_eq!(error.code(), CALYX_ORACLE_ENERGY_EMPTY_REGION);
    println!("edge_zero_region code={}", error.code());
}

#[test]
fn insufficient_panel_stops_before_region_or_ledger() {
    let fixture = Fixture::new(2).with_dense_slots();
    let ledger = MemoryLedger::default();
    let error = complete_with_assay_and_region(
        &FakeAssay::insufficient(),
        &ledger,
        &fixture.cx,
        &fixture.panel,
        DomainId::from("synthetic"),
        set(&[1]),
        set(&[2]),
        &PanicRegion,
        OracleSelfConsistency::measured(0.0, 1.0),
        &FixedAnneal,
        &fixture.clock,
    )
    .expect_err("insufficient");

    assert_eq!(error.code(), CALYX_ORACLE_INSUFFICIENT);
    assert!(ledger.payloads().is_empty());
    println!("edge_insufficient code={} ledger_rows=0", error.code());
}

#[test]
fn overlap_partition_reports_slot_conflict() {
    let fixture = Fixture::new(2).with_dense_slots();
    let error =
        run_complete(&fixture, set(&[1]), set(&[1, 2]), 1.0).expect_err("overlap should fail");

    assert_eq!(error.code(), crate::CALYX_ORACLE_SLOT_CONFLICT);
    println!("edge_overlap code={}", error.code());
}

#[test]
fn ledger_write_failure_is_reported() {
    let fixture = Fixture::new(2).with_dense_slots();
    let error = complete_with_assay_and_region(
        &FakeAssay::sufficient(),
        &MemoryLedger::failing(),
        &fixture.cx,
        &fixture.panel,
        DomainId::from("synthetic"),
        set(&[1]),
        set(&[2]),
        &MapRegion::for_panel(&fixture.panel),
        OracleSelfConsistency::measured(0.0, 1.0),
        &FixedAnneal,
        &fixture.clock,
    )
    .expect_err("ledger failure");

    assert_eq!(error.code(), crate::CALYX_ORACLE_LEDGER_WRITE_FAILURE);
    println!("edge_ledger_failure code={}", error.code());
}

#[test]
fn aster_ledger_row_contains_completion_payload_bytes() {
    let fixture = Fixture::new(2).with_dense_slots();
    let vault = AsterVault::with_clock(vault_id(), b"complete-ledger", FixedClock::new(7));
    let ledger = AsterCompletionLedger { vault: &vault };
    let result = complete_with_assay_and_region(
        &FakeAssay::sufficient(),
        &ledger,
        &fixture.cx,
        &fixture.panel,
        DomainId::from("synthetic"),
        set(&[1]),
        set(&[2]),
        &MapRegion::for_panel(&fixture.panel),
        OracleSelfConsistency::measured(0.0, 0.8),
        &FixedAnneal,
        &fixture.clock,
    )
    .expect("complete");

    let bytes = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Ledger,
            &ledger_key(result.provenance.seq),
        )
        .expect("read ledger")
        .expect("ledger row");
    let entry = decode(&bytes).expect("decode ledger");
    let payload: serde_json::Value = serde_json::from_slice(&entry.payload).expect("payload json");

    assert_eq!(entry.kind, EntryKind::Answer);
    assert_eq!(payload["tag"], COMPLETION_LEDGER_TAG);
    assert_eq!(payload["cx_id"], fixture.cx.cx_id.to_string());
    assert_eq!(
        payload["confidence"].as_f64().unwrap() as f32,
        result.confidence
    );
    println!(
        "ledger_readback seq={} kind={} payload_tag={} payload_bytes={}",
        result.provenance.seq,
        entry.kind,
        payload["tag"],
        entry.payload.len()
    );
}

proptest! {
    #[test]
    fn partition_tags_match_clamp_and_free(mask in 0u8..128) {
        let fixture = Fixture::new(7).with_dense_slots();
        let mut clamp = SlotSet::new();
        let mut free = SlotSet::new();
        for index in 0..7 {
            let target = if (mask & (1 << index)) == 0 { &mut clamp } else { &mut free };
            target.insert(lens((index + 1) as u8));
        }

        let result = run_complete(&fixture, clamp.clone(), free.clone(), 1.0).unwrap();
        for slot in result.filled_cx {
            if clamp.contains(&slot.lens_id) {
                prop_assert_eq!(slot.tag, SlotTag::Measured);
            } else {
                prop_assert!(free.contains(&slot.lens_id));
                prop_assert!(matches!(slot.tag, SlotTag::Inferred | SlotTag::Provisional));
            }
        }
    }
}

#[test]
#[ignore = "manual FSV for issue #442 completion primitive"]
fn issue442_complete_fsv_writes_readbacks() {
    let root = std::env::var("CALYX_FSV_ROOT").expect("CALYX_FSV_ROOT required");
    let root = std::path::PathBuf::from(root);
    std::fs::create_dir_all(&root).expect("create fsv root");
    let fixture = Fixture::new(7).with_dense_slots();
    let vault_dir = root.join("aster-vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue442-complete",
        VaultOptions::default(),
    )
    .expect("durable vault");
    let ledger = AsterCompletionLedger { vault: &vault };

    let happy = complete_with_assay_and_region(
        &FakeAssay::sufficient(),
        &ledger,
        &fixture.cx,
        &fixture.panel,
        DomainId::from("synthetic"),
        set(&[1, 2, 3]),
        set(&[4, 5, 6, 7]),
        &MapRegion::for_panel(&fixture.panel),
        OracleSelfConsistency::measured(0.0, 0.72),
        &FixedAnneal,
        &fixture.clock,
    )
    .expect("happy complete");
    write_json(&root.join("happy-result.json"), &happy);
    let ledger_bytes = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Ledger,
            &ledger_key(happy.provenance.seq),
        )
        .expect("read ledger")
        .expect("ledger row");
    std::fs::write(root.join("happy-ledger-row.bin"), &ledger_bytes).expect("write ledger bytes");
    let entry = decode(&ledger_bytes).expect("decode ledger");
    let payload_json: serde_json::Value =
        serde_json::from_slice(&entry.payload).expect("decode payload");
    write_json(&root.join("happy-ledger-payload.json"), &payload_json);

    let all_clamped =
        run_complete(&fixture, set(&[1, 2, 3, 4, 5, 6, 7]), set(&[]), 1.0).expect("all clamped");
    let all_free = run_complete(
        &Fixture::new(7),
        set(&[]),
        set(&[1, 2, 3, 4, 5, 6, 7]),
        0.44,
    )
    .expect("all free");
    write_json(&root.join("edge-all-clamped.json"), &all_clamped);
    write_json(&root.join("edge-all-free.json"), &all_free);

    let edge_fixture = Fixture::new(1);
    let empty_region = complete_with_assay_and_region(
        &FakeAssay::sufficient(),
        &MemoryLedger::default(),
        &edge_fixture.cx,
        &edge_fixture.panel,
        DomainId::from("synthetic"),
        set(&[]),
        set(&[1]),
        &MapRegion::default(),
        OracleSelfConsistency::measured(0.0, 1.0),
        &FixedAnneal,
        &fixture.clock,
    )
    .expect_err("empty region");
    std::fs::write(
        root.join("edge-zero-region-error.txt"),
        empty_region.to_string(),
    )
    .expect("write zero region error");

    let insufficient = complete_with_assay_and_region(
        &FakeAssay::insufficient(),
        &MemoryLedger::default(),
        &fixture.cx,
        &fixture.panel,
        DomainId::from("synthetic"),
        set(&[1]),
        set(&[2, 3, 4, 5, 6, 7]),
        &PanicRegion,
        OracleSelfConsistency::measured(0.0, 1.0),
        &FixedAnneal,
        &fixture.clock,
    )
    .expect_err("insufficient");
    std::fs::write(
        root.join("edge-insufficient-error.txt"),
        insufficient.to_string(),
    )
    .expect("write insufficient error");

    let summary = format!(
        "happy measured={} inferred={} confidence={:.3} ledger_seq={}\n\
         edge_all_clamped measured={} inferred={} confidence={:.3}\n\
         edge_all_free measured={} inferred={} confidence={:.3}\n\
         edge_zero_region code={}\n\
         edge_insufficient code={}\n",
        happy.measured_slots().len(),
        happy.inferred_slots().len(),
        happy.confidence,
        happy.provenance.seq,
        all_clamped.measured_slots().len(),
        all_clamped.inferred_slots().len(),
        all_clamped.confidence,
        all_free.measured_slots().len(),
        all_free.inferred_slots().len(),
        all_free.confidence,
        empty_region.code(),
        insufficient.code()
    );
    std::fs::write(root.join("fsv-summary.txt"), &summary).expect("write summary");
    println!("{summary}");
}

fn write_json(path: &std::path::Path, value: &impl serde::Serialize) {
    let file = std::fs::File::create(path).expect("create json");
    serde_json::to_writer_pretty(file, value).expect("write json");
}
