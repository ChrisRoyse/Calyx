use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{
    Asymmetry, Constellation, CxFlags, InputRef, LedgerRef, LensId, Modality, QuantPolicy, Slot,
    SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId,
};
use calyx_registry::frozen::{NormPolicy, sha256_digest};
use calyx_registry::{
    CALYX_VECTOR_COMPRESSION_INVALID, LensRuntime, LensSpec, StoredSlotCodec,
    decode_stored_slot_envelope, write_compressed_slot_batch,
};
use serde_json::{Value, json};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn issue925_raw_f32_compression_rows_are_tagged_and_fail_closed() {
    let root = fsv_case_root("issue925-raw-f32-envelope");
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault");
    let before = directory_state(&vault_dir);
    fs::create_dir_all(&vault_dir).unwrap();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue925-raw-f32-envelope".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let rows = fixture_rows(4);
    let first_cx = rows[0].0;
    let slot = make_slot(
        "raw-f32",
        SlotId::new(5),
        SlotShape::Dense(4),
        QuantPolicy::None,
    );
    let lens = lens_spec("raw-f32", QuantPolicy::None, None, 4, 0.0);
    let report = write_compressed_slot_batch(&vault, &slot, &lens, &rows, &[], 2).unwrap();
    vault.flush().unwrap();
    let snapshot = report.snapshot.unwrap();
    let stored = vault
        .read_cf_at(
            snapshot,
            ColumnFamily::slot(slot.slot_id),
            &slot_key(first_cx),
        )
        .unwrap()
        .unwrap();
    let raw_sidecar = vault
        .read_cf_at(
            snapshot,
            ColumnFamily::slot_raw(slot.slot_id),
            &slot_key(first_cx),
        )
        .unwrap()
        .unwrap();
    let envelope = decode_stored_slot_envelope(&stored).unwrap();
    let decoded_sidecar = decode_dense_sidecar(&raw_sidecar);

    assert_eq!(stored[0], calyx_registry::COMPRESSED_SLOT_TAG);
    assert_eq!(envelope.codec, StoredSlotCodec::RawF32);
    assert_eq!(envelope.level, "F32");
    assert_eq!(envelope.raw_dim, 4);
    assert_eq!(envelope.stored_dim, 4);
    assert_eq!(envelope.payload_bytes, 16);
    assert_eq!(report.stored_codec, StoredSlotCodec::RawF32);
    assert_eq!(decoded_sidecar, rows[0].1);

    let mut empty_payload = stored.clone();
    empty_payload[49..53].copy_from_slice(&0_u32.to_be_bytes());
    empty_payload.truncate(53);
    let mut truncated_payload = stored.clone();
    truncated_payload.pop();
    let mut mismatched_dim = stored.clone();
    mismatched_dim[8..12].copy_from_slice(&5_u32.to_be_bytes());
    let mut non_finite = stored.clone();
    non_finite[53..57].copy_from_slice(&f32::NAN.to_bits().to_be_bytes());

    let edges = vec![
        edge_result("legacy_untagged_raw_sidecar", &raw_sidecar),
        edge_result("empty_payload", &empty_payload),
        edge_result("truncated_payload", &truncated_payload),
        edge_result("mismatched_dim", &mismatched_dim),
        edge_result("non_finite_payload", &non_finite),
    ];
    for edge in &edges {
        assert_eq!(
            edge["after"]["error_code"],
            CALYX_VECTOR_COMPRESSION_INVALID
        );
    }

    let readback = json!({
        "issue": 925,
        "trigger": "write_compressed_slot_batch with QuantPolicy::None",
        "source_of_truth": {
            "vault_dir": display(&vault_dir),
            "slot_cf": format!("slot_{:02}", slot.slot_id.get()),
            "raw_sidecar_cf": format!("slot_{:02}.raw", slot.slot_id.get()),
            "readback_file": display(&root.join("issue925-readback.json")),
        },
        "before": before,
        "after": {
            "snapshot": snapshot,
            "vault": directory_state(&vault_dir),
            "slot_cf": {
                "len": stored.len(),
                "tag": stored[0],
                "prefix_hex": hex(&stored[..stored.len().min(64)]),
                "sha256": hex(&sha256_digest(&[&stored])),
                "envelope": envelope,
            },
            "raw_sidecar": {
                "len": raw_sidecar.len(),
                "tag": raw_sidecar[0],
                "prefix_hex": hex(&raw_sidecar[..raw_sidecar.len().min(64)]),
                "sha256": hex(&sha256_digest(&[&raw_sidecar])),
                "decoded_dense": decoded_sidecar,
            },
            "compression_report": {
                "stored_codec": format!("{:?}", report.stored_codec),
                "raw_bytes_total": report.raw_bytes_total,
                "stored_bytes_total": report.stored_bytes_total,
                "row0_codec": format!("{:?}", report.rows[0].codec),
                "row0_stored_dim": report.rows[0].stored_dim,
            },
        },
        "edge_cases": edges,
    });
    let readback_path = root.join("issue925-readback.json");
    write_json(&readback_path, &readback);
    let readback_bytes = fs::read(&readback_path).unwrap();
    let readback_sha256 = hex(&sha256_digest(&[&readback_bytes]));

    println!("ISSUE925_FSV_ROOT={}", root.display());
    println!("ISSUE925_READBACK={}", readback_path.display());
    println!("ISSUE925_READBACK_SHA256={readback_sha256}");
    println!("ISSUE925_STORED_CODEC={:?}", report.stored_codec);
    println!("ISSUE925_SLOT_CF_TAG={}", stored[0]);
    println!(
        "ISSUE925_EDGE_COUNT={}",
        readback["edge_cases"].as_array().unwrap().len()
    );

    if !keep_fsv_root() {
        fs::remove_dir_all(root).unwrap();
    }
}

fn edge_result(name: &str, bytes: &[u8]) -> Value {
    let before = json!({
        "len": bytes.len(),
        "tag": bytes.first().copied(),
        "prefix_hex": hex(&bytes[..bytes.len().min(32)]),
    });
    let error = decode_stored_slot_envelope(bytes).expect_err("edge must fail closed");
    json!({
        "name": name,
        "before": before,
        "after": {
            "error_code": error.code,
            "error_message": error.message,
        }
    })
}

fn decode_dense_sidecar(bytes: &[u8]) -> Vec<f32> {
    match encode::decode_slot_vector(bytes).unwrap() {
        SlotVector::Dense { data, .. } => data,
        other => panic!("expected dense sidecar, got {other:?}"),
    }
}

fn fsv_case_root(label: &str) -> PathBuf {
    let serial = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    if let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") {
        return root.join(format!("{label}-{}-{serial}", std::process::id()));
    }
    std::env::temp_dir().join(format!("calyx-{label}-{}-{serial}", std::process::id()))
}

fn directory_state(path: &Path) -> Value {
    let mut files = Vec::new();
    if path.exists() {
        collect_files(path, path, &mut files);
    }
    json!({
        "path": display(path),
        "exists": path.exists(),
        "files": files,
    })
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<Value>) {
    let mut entries = fs::read_dir(current)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            let bytes = fs::read(&path).unwrap();
            files.push(json!({
                "relative": path.strip_prefix(root).unwrap().display().to_string().replace('\\', "/"),
                "len": bytes.len(),
                "sha256": hex(&sha256_digest(&[&bytes])),
            }));
        }
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

#[allow(dead_code)]
#[path = "issue790_vector_compression_fsv/support.rs"]
mod support;
use support::*;
