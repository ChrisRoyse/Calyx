//! FSV integration tests for `readback cx-list --include-slots` (#1060).
//!
//! Base rows persist only slot ids + payload hashes, so `--include-slots`
//! must hydrate slot state from the physical slot CFs — the same source of
//! truth `readback --cf slot_XX` and weave-loom dense coverage read — and
//! must fail closed (`CALYX_ASTER_CORRUPT_SHARD`) when a base-listed slot
//! has no decodable physical payload row.
//!
//! No mocks: happy paths run the real ingest write path into a durable
//! vault and then execute the actual `calyx` binary; corruption edges write
//! real SST files with the production encoders and delete/garble physical
//! rows on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::dedup::{DedupResult, EpochSecs, IngestInput, ingest_at};
use calyx_aster::mvcc::tombstone_value;
use calyx_aster::sst::write_sst;
use calyx_aster::vault::encode::{encode_constellation_base, encode_slot_vector};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector,
    SparseEntry, VaultId,
};
use serde_json::Value;

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

#[test]
fn cx_list_include_slots_reports_physical_slot_payloads() {
    let root = temp_root("cx-list-slots-happy");
    let vault_dir = root.join("vault");
    let vault = durable_vault(&vault_dir);
    let first = new_cx_id(
        ingest_at(
            &vault,
            &probe_input("cx-list-a", 0.25),
            EpochSecs(100),
            None,
        )
        .expect("ingest first"),
    );
    let second = new_cx_id(
        ingest_at(
            &vault,
            &probe_input("cx-list-b", 0.75),
            EpochSecs(200),
            None,
        )
        .expect("ingest second"),
    );
    vault.flush().expect("flush durable vault");
    drop(vault);

    let rows = stdout_json(run_cx_list(&vault_dir, &["--include-slots"]));
    let rows = rows.as_array().expect("cx-list array");
    assert_eq!(rows.len(), 2, "two live base rows: {rows:?}");
    for row in rows {
        assert_eq!(row["slot_payloads_decoded"], true);
        assert_eq!(row["slot_payload_decode_mode"], "physical_slot_cf_readback");
        let summary = &row["slot_summary"];
        assert_eq!(summary["dense_slots"], 1, "summary: {summary}");
        assert_eq!(summary["sparse_slots"], 1, "summary: {summary}");
        assert_eq!(summary["absent_slots"], 1, "summary: {summary}");
        assert_eq!(summary["tombstoned_slots"], 0, "summary: {summary}");
        assert_eq!(
            summary["absent_reasons"]["lens_inactive"], 1,
            "physical absent reason must surface: {summary}"
        );
        let slots = row["slots"].as_array().expect("slots array");
        let by_slot = |id: u64| {
            slots
                .iter()
                .find(|slot| slot["slot"] == id)
                .unwrap_or_else(|| panic!("slot {id} missing from {slots:?}"))
        };
        let dense = by_slot(0);
        assert_eq!(dense["kind"], "dense", "{dense}");
        assert_eq!(dense["payload_source"], "slot_cf", "{dense}");
        assert_eq!(dense["dim"], 4, "{dense}");
        assert_eq!(dense["values"], 4, "{dense}");
        let sparse = by_slot(3);
        assert_eq!(sparse["kind"], "sparse", "{sparse}");
        assert_eq!(sparse["payload_source"], "slot_cf", "{sparse}");
        let absent = by_slot(5);
        assert_eq!(absent["kind"], "absent", "{absent}");
        assert_eq!(absent["payload_source"], "slot_cf", "{absent}");
        assert_eq!(absent["reason"], "lens_inactive", "{absent}");
    }

    // Single-row --cx-id path must hydrate the same physical state.
    for cx_id in [first, second] {
        let rows = stdout_json(run_cx_list(
            &vault_dir,
            &["--cx-id", &cx_id.to_string(), "--include-slots"],
        ));
        let row = &rows.as_array().expect("array")[0];
        assert_eq!(row["cx_id"], cx_id.to_string());
        assert_eq!(row["slot_summary"]["dense_slots"], 1, "{row}");
        assert_eq!(row["slot_summary"]["absent_reasons"]["lens_inactive"], 1);
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn cx_list_without_include_slots_stays_base_only() {
    let root = temp_root("cx-list-base-only");
    let vault_dir = root.join("vault");
    let vault = durable_vault(&vault_dir);
    ingest_at(&vault, &probe_input("base-only", 0.5), EpochSecs(100), None).expect("ingest");
    vault.flush().expect("flush");
    drop(vault);

    let rows = stdout_json(run_cx_list(&vault_dir, &[]));
    let row = &rows.as_array().expect("array")[0];
    assert_eq!(row["slot_payloads_decoded"], false);
    assert_eq!(row["slot_payload_decode_mode"], "base_only");
    assert!(
        row.get("slots").is_none(),
        "base-only must not claim slot state"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn cx_list_include_slots_empty_vault_renders_empty_list() {
    let root = temp_root("cx-list-empty");
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).expect("create empty vault dir");

    let rows = stdout_json(run_cx_list(&vault_dir, &["--include-slots"]));
    assert_eq!(rows, serde_json::json!([]));

    fs::remove_dir_all(root).ok();
}

#[test]
fn cx_list_include_slots_fails_closed_when_physical_slot_rows_missing() {
    let root = temp_root("cx-list-missing-slot-rows");
    let vault_dir = root.join("vault");
    let cx = synthetic_constellation(
        7,
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 2.0],
        },
    );
    write_base_only(&vault_dir, &cx);

    let output = run_cx_list(&vault_dir, &["--include-slots"]);
    assert!(
        !output.status.success(),
        "missing physical slot rows must fail closed; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CALYX_ASTER_CORRUPT_SHARD"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("slot 7"),
        "stderr must name the slot: {stderr}"
    );
    assert!(
        stderr.contains(&cx.cx_id.to_string()),
        "stderr must name the cx: {stderr}"
    );

    // Without --include-slots the same vault must still list base rows.
    let rows = stdout_json(run_cx_list(&vault_dir, &[]));
    assert_eq!(rows.as_array().expect("array").len(), 1);

    fs::remove_dir_all(root).ok();
}

#[test]
fn cx_list_include_slots_reads_slot_raw_for_undecodable_slot_rows() {
    let root = temp_root("cx-list-slot-raw");
    let vault_dir = root.join("vault");
    let cx = synthetic_constellation(
        8,
        SlotVector::Dense {
            dim: 2,
            data: vec![0.5, 0.25],
        },
    );
    write_base_only(&vault_dir, &cx);
    // Compressed-slot shape: opaque bytes in slot CF, decodable payload in slot_raw.
    write_cf_sst(
        &vault_dir,
        ColumnFamily::slot(SlotId::new(8)),
        &slot_key(cx.cx_id),
        &[0xFF, 0xFF, 0xFF],
    );
    write_cf_sst(
        &vault_dir,
        ColumnFamily::slot_raw(SlotId::new(8)),
        &slot_key(cx.cx_id),
        &encode_slot_vector(&SlotVector::Dense {
            dim: 2,
            data: vec![0.5, 0.25],
        })
        .expect("encode raw payload"),
    );

    let rows = stdout_json(run_cx_list(&vault_dir, &["--include-slots"]));
    let slot = &rows.as_array().expect("array")[0]["slots"][0];
    assert_eq!(slot["kind"], "dense", "{slot}");
    // This synthetic vault has no ledger CF, so the row cannot resolve via
    // its commit batch and is honestly labeled as a full-SST-set read.
    assert_eq!(slot["payload_source"], "slot_raw_cf_full_set", "{slot}");

    fs::remove_dir_all(root).ok();
}

#[test]
fn cx_list_include_slots_fails_closed_when_slot_row_undecodable_and_no_raw() {
    let root = temp_root("cx-list-undecodable");
    let vault_dir = root.join("vault");
    let cx = synthetic_constellation(
        9,
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        },
    );
    write_base_only(&vault_dir, &cx);
    write_cf_sst(
        &vault_dir,
        ColumnFamily::slot(SlotId::new(9)),
        &slot_key(cx.cx_id),
        &[0xFF, 0xFF, 0xFF],
    );

    let output = run_cx_list(&vault_dir, &["--include-slots"]);
    assert!(
        !output.status.success(),
        "undecodable slot row must fail closed"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CALYX_ASTER_CORRUPT_SHARD"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("not decodable"), "stderr: {stderr}");

    fs::remove_dir_all(root).ok();
}

#[test]
fn cx_list_include_slots_reports_tombstoned_slot_rows() {
    let root = temp_root("cx-list-tombstoned");
    let vault_dir = root.join("vault");
    let cx = synthetic_constellation(
        4,
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 2.0],
        },
    );
    write_base_only(&vault_dir, &cx);
    write_cf_sst(
        &vault_dir,
        ColumnFamily::slot(SlotId::new(4)),
        &slot_key(cx.cx_id),
        &tombstone_value(),
    );

    let rows = stdout_json(run_cx_list(&vault_dir, &["--include-slots"]));
    let row = &rows.as_array().expect("array")[0];
    let slot = &row["slots"][0];
    assert_eq!(slot["kind"], "tombstoned", "{slot}");
    // No ledger CF in this synthetic layout: the tombstone resolves through
    // the full SST set (newest-first), not through a commit batch.
    assert_eq!(
        slot["payload_source"], "slot_cf_full_set_tombstone",
        "{slot}"
    );
    assert_eq!(row["slot_summary"]["tombstoned_slots"], 1);
    assert_eq!(row["slot_summary"]["dense_slots"], 0);

    fs::remove_dir_all(root).ok();
}

fn probe_input(name: &str, seed: f32) -> IngestInput {
    IngestInput::new(name.as_bytes().to_vec(), 1, Modality::Text)
        .with_slot(
            SlotId::new(0),
            SlotVector::Dense {
                dim: 4,
                data: vec![seed, seed + 0.5, 1.0 - seed, seed * 2.0],
            },
        )
        .with_slot(
            SlotId::new(3),
            SlotVector::Sparse {
                dim: 16,
                entries: vec![
                    SparseEntry { idx: 1, val: seed },
                    SparseEntry {
                        idx: 7,
                        val: 1.0 - seed,
                    },
                ],
            },
        )
        .with_slot(
            SlotId::new(5),
            SlotVector::Absent {
                reason: AbsentReason::LensInactive,
            },
        )
}

fn synthetic_constellation(slot: u16, vector: SlotVector) -> Constellation {
    let mut slots = std::collections::BTreeMap::new();
    slots.insert(SlotId::new(slot), vector);
    Constellation {
        cx_id: CxId::from_bytes([0x42; 16]),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 10,
        input_ref: InputRef {
            hash: [9; 32],
            pointer: Some("synthetic://cx-list-fsv".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: std::collections::BTreeMap::new(),
        metadata: std::collections::BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [7; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

/// Writes only the base CF row for `cx` as a real SST — the physical layout
/// of a vault whose slot rows are gone.
fn write_base_only(vault_dir: &Path, cx: &Constellation) {
    write_cf_sst(
        vault_dir,
        ColumnFamily::Base,
        &base_key(cx.cx_id),
        &encode_constellation_base(cx).expect("encode base row"),
    );
}

fn write_cf_sst(vault_dir: &Path, cf: ColumnFamily, key: &[u8], value: &[u8]) {
    let dir = vault_dir.join("cf").join(cf.name());
    fs::create_dir_all(&dir).expect("create cf dir");
    write_sst(dir.join("00000000000000000001.sst"), [(key, value)]).expect("write sst");
}

fn durable_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        b"cx-list-include-slots-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn run_cx_list(vault_dir: &Path, extra: &[&str]) -> Output {
    let vault = vault_dir.display().to_string();
    let mut args = vec!["readback", "cx-list", "--vault", vault.as_str()];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args(&args)
        .output()
        .expect("run calyx readback cx-list")
}

fn stdout_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "cx-list failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("json stdout")
}

fn new_cx_id(result: DedupResult) -> CxId {
    match result {
        DedupResult::New(id) => id,
        other => panic!("expected fresh ingest, got {other:?}"),
    }
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid ULID")
}

fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "calyx-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    path
}
