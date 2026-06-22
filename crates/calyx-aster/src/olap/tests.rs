use super::*;
use crate::cf::ColumnFamily;
use calyx_core::{
    Clock, Constellation, CxFlags, FixedClock, InputRef, LedgerRef, Modality, SlotVector, VaultId,
    VaultStore,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn olap_aggregate_reads_materialized_column_bytes() {
    let root = test_dir("olap-aggregate");
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(42));
    let slot = SlotId::new(3);
    put_rows(&vault, slot);
    let output = root.join("slot_03");
    let plan = OlapScanPlan::new(0).with_group_by(2);

    let from_vault = vault
        .olap_scan_aggregate_slot_at(vault.latest_seq(), slot, &output, plan)
        .expect("scan aggregate");
    let standalone =
        scan_materialized_slot_column_aggregate(output.join("slot-column-manifest.json"), plan)
            .expect("standalone aggregate");

    assert_aggregate(&from_vault.aggregate, 5, 70.0, -5.0, 30.0, 14.0);
    assert_eq!(standalone.aggregate, from_vault.aggregate);
    assert_eq!(from_vault.groups.len(), 2);
    assert_aggregate(&from_vault.groups[0].aggregate, 2, 30.0, 10.0, 20.0, 15.0);
    assert_aggregate(
        &from_vault.groups[1].aggregate,
        3,
        40.0,
        -5.0,
        30.0,
        40.0 / 3.0,
    );
    let chunk = fs::read(output.join("slot-column.cxa1")).expect("read chunk");
    assert_eq!(&chunk[..4], b"CXA1");
    let columns = crate::sst::arrow::decode_column_shape(&chunk).expect("decode columns");
    let mut actual_values = columns.column_values(0).unwrap().collect::<Vec<_>>();
    actual_values.sort_by(f32::total_cmp);
    assert_eq!(actual_values, vec![-5.0, 10.0, 15.0, 20.0, 30.0]);
    cleanup(root);
}

#[test]
fn olap_edges_fail_closed() {
    let root = test_dir("olap-edges");
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(42));
    let slot = SlotId::new(3);
    let empty = vault
        .olap_scan_aggregate_slot_at(
            vault.latest_seq(),
            slot,
            root.join("empty"),
            OlapScanPlan::new(0),
        )
        .expect_err("empty slot rejected");
    assert_eq!(empty.code, "CALYX_STALE_DERIVED");

    put_rows(&vault, slot);
    let output = root.join("slot_03");
    vault
        .materialize_slot_column_at(vault.latest_seq(), slot, &output)
        .expect("materialize");
    let manifest = output.join("slot-column-manifest.json");
    let bad_column = scan_materialized_slot_column_aggregate(&manifest, OlapScanPlan::new(99))
        .expect_err("bad column rejected");
    assert_eq!(bad_column.code, "CALYX_OLAP_INVALID_PLAN");
    let row_limit =
        scan_materialized_slot_column_aggregate(&manifest, OlapScanPlan::new(0).with_limits(4, 8))
            .expect_err("row cap rejected");
    assert_eq!(row_limit.code, "CALYX_OLAP_SCAN_LIMIT");
    let group_limit = scan_materialized_slot_column_aggregate(
        &manifest,
        OlapScanPlan::new(0).with_group_by(2).with_limits(8, 1),
    )
    .expect_err("group cap rejected");
    assert_eq!(group_limit.code, "CALYX_OLAP_SCAN_LIMIT");

    let chunk = output.join("slot-column.cxa1");
    let mut bytes = fs::read(&chunk).expect("read chunk");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    fs::write(&chunk, bytes).expect("corrupt chunk");
    let corrupt = scan_materialized_slot_column_aggregate(&manifest, OlapScanPlan::new(0))
        .expect_err("corrupt chunk rejected");
    assert_eq!(corrupt.code, "CALYX_ASTER_CORRUPT_SHARD");
    cleanup(root);
}

#[test]
#[ignore = "manual FSV for issue #586 columnar OLAP aggregate"]
fn issue586_columnar_olap_fsv() {
    let root = fsv_root();
    let vault_dir = root.join("vault.calyx");
    let column_dir = root.join("slot_03_column");
    let report_path = root.join("issue586-columnar-olap-fsv.json");
    clean_path(&vault_dir);
    clean_path(&column_dir);
    clean_path(&root.join("empty"));
    clean_path(&report_path);
    fs::create_dir_all(&root).expect("create root");

    let before_artifacts = list_artifacts(&column_dir);
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue586-salt".to_vec(),
        crate::vault::VaultOptions::default(),
    )
    .expect("open durable vault");
    let slot = SlotId::new(3);
    let empty_before = list_artifacts(&root.join("empty"));
    let empty = vault
        .olap_scan_aggregate_slot_at(
            vault.latest_seq(),
            slot,
            root.join("empty"),
            OlapScanPlan::new(0),
        )
        .expect_err("empty slot rejected");
    let empty_after = list_artifacts(&root.join("empty"));

    put_rows(&vault, slot);
    vault.flush().expect("flush slot cf");
    let snapshot = vault.latest_seq();
    let slot_cf_rows = vault
        .scan_cf_at(snapshot, ColumnFamily::slot(slot))
        .expect("scan slot cf");
    let plan = OlapScanPlan::new(0).with_group_by(2);
    let result = vault
        .olap_scan_aggregate_slot_at(snapshot, slot, &column_dir, plan)
        .expect("olap aggregate");
    drop(vault);
    let standalone =
        scan_materialized_slot_column_aggregate(column_dir.join("slot-column-manifest.json"), plan)
            .expect("standalone aggregate");
    let chunk = fs::read(&result.source_chunk_path).expect("read chunk");

    let bad_column_before = file_sha(&result.source_chunk_path);
    let bad_column = scan_materialized_slot_column_aggregate(
        &result.source_manifest_path,
        OlapScanPlan::new(99),
    )
    .expect_err("bad column rejected");
    let bad_column_after = file_sha(&result.source_chunk_path);
    let row_limit = scan_materialized_slot_column_aggregate(
        &result.source_manifest_path,
        OlapScanPlan::new(0).with_limits(4, 8),
    )
    .expect_err("row cap rejected");
    let group_limit = scan_materialized_slot_column_aggregate(
        &result.source_manifest_path,
        OlapScanPlan::new(0).with_group_by(2).with_limits(8, 1),
    )
    .expect_err("group cap rejected");

    let report = json!({
        "issue": 586,
        "source_of_truth": {
            "vault": vault_dir,
            "manifest": result.source_manifest_path,
            "chunk": result.source_chunk_path,
        },
        "before_artifacts": before_artifacts,
        "slot_cf_rows_after_flush": slot_cf_rows.len(),
        "input_rows": input_rows(),
        "hand_expected": expected_json(),
        "actual": result,
        "standalone_actual": standalone,
        "chunk_header_hex": bytes_hex(&chunk[..16]),
        "value_column_0_hex": column_bytes_hex(&chunk, 5, 0),
        "group_column_2_hex": column_bytes_hex(&chunk, 5, 2),
        "edge_cases": {
            "empty_slot": {
                "before": empty_before,
                "after": empty_after,
                "code": empty.code,
            },
            "invalid_column": {
                "before_chunk_sha256": bad_column_before,
                "after_chunk_sha256": bad_column_after,
                "code": bad_column.code,
            },
            "row_limit": {"code": row_limit.code},
            "group_limit": {"code": group_limit.code},
        }
    });
    fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).expect("write report");
    println!("ISSUE586_FSV_REPORT={}", report_path.display());
    println!("ISSUE586_CHUNK={}", result.source_chunk_path.display());
}

fn put_rows(vault: &AsterVault<impl Clock>, slot: SlotId) {
    for (name, row) in [
        ("a", [10.0, 1.0, 0.0]),
        ("b", [20.0, 2.0, 0.0]),
        ("c", [30.0, 3.0, 1.0]),
        ("d", [-5.0, 4.0, 1.0]),
        ("e", [15.0, 5.0, 1.0]),
    ] {
        let cx = constellation(vault, name.as_bytes(), slot, &row);
        vault.put(cx).expect("put row");
    }
}

fn constellation(
    vault: &AsterVault<impl Clock>,
    input: &[u8],
    slot: SlotId,
    values: &[f32],
) -> Constellation {
    let cx_id = vault.cx_id_for_input(input, 1);
    let mut slots = BTreeMap::new();
    slots.insert(
        slot,
        SlotVector::Dense {
            dim: values.len() as u32,
            data: values.to_vec(),
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 42,
        input_ref: InputRef {
            hash: input_hash(input),
            pointer: Some(format!(
                "synthetic://issue586/{}",
                String::from_utf8_lossy(input)
            )),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
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

fn input_rows() -> Vec<Vec<f32>> {
    vec![
        vec![10.0, 1.0, 0.0],
        vec![20.0, 2.0, 0.0],
        vec![30.0, 3.0, 1.0],
        vec![-5.0, 4.0, 1.0],
        vec![15.0, 5.0, 1.0],
    ]
}

fn expected_json() -> serde_json::Value {
    json!({
        "count": 5,
        "sum": 70.0,
        "min": -5.0,
        "max": 30.0,
        "avg": 14.0,
        "groups": [
            {"group": 0.0, "count": 2, "sum": 30.0, "avg": 15.0},
            {"group": 1.0, "count": 3, "sum": 40.0, "avg": 40.0 / 3.0}
        ]
    })
}

fn assert_aggregate(got: &OlapAggregate, count: usize, sum: f64, min: f32, max: f32, avg: f64) {
    assert_eq!(got.count, count);
    assert_eq!(got.sum, sum);
    assert_eq!(got.min, min);
    assert_eq!(got.max, max);
    assert!((got.avg - avg).abs() < f64::EPSILON);
}

fn input_hash(input: &[u8]) -> [u8; 32] {
    let mut hash = [0_u8; 32];
    hash[..input.len()].copy_from_slice(input);
    hash
}

fn column_bytes_hex(chunk: &[u8], rows: usize, column: usize) -> String {
    let start = 16 + column * rows * 4;
    bytes_hex(&chunk[start..start + rows * 4])
}

fn bytes_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn file_sha(path: &Path) -> String {
    let bytes = fs::read(path).expect("read sha input");
    format!("{:x}", sha2::Sha256::digest(&bytes))
}

fn list_artifacts(path: &Path) -> Vec<String> {
    match fs::read_dir(path) {
        Ok(entries) => entries
            .map(|entry| entry.expect("dir entry").path().display().to_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
    clean_path(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE586_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| test_dir("issue586-fsv"))
}

fn clean_path(path: &Path) {
    if path.exists() {
        if path.is_dir() {
            fs::remove_dir_all(path).expect("remove dir");
        } else {
            fs::remove_file(path).expect("remove file");
        }
    }
}

fn cleanup(dir: PathBuf) {
    clean_path(&dir);
}
