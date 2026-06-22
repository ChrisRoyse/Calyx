use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::{AsterVault, VaultOptions, read_materialized_slot_column};
use calyx_core::{
    AbsentReason, Clock, Constellation, CxFlags, FixedClock, InputRef, LedgerRef, Modality, SlotId,
    SlotVector, VaultId, VaultStore,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

mod fsv_support;
use fsv_support::{named_fsv_root, reset_dir, write_json};

#[test]
fn slot_column_materialization_fsv_writes_readbacks() {
    let (root, keep_root) = named_fsv_root("CALYX_ASTER_SLOT_COLUMN_FSV_ROOT", "slot-column-fsv");
    reset_dir(&root);

    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"slot-column-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let slot = SlotId::new(6);
    let rows = [
        (b"alpha".as_slice(), vec![1.0, 2.0, 3.0, 4.0]),
        (b"beta".as_slice(), vec![5.0, 6.5, 7.25, 8.125]),
        (b"gamma".as_slice(), vec![9.0, 10.0, 11.0, 12.0]),
    ];
    let cx_ids = rows
        .iter()
        .map(|(input, values)| {
            let cx = constellation(
                &vault,
                input,
                slot,
                SlotVector::Dense {
                    dim: values.len() as u32,
                    data: values.clone(),
                },
            );
            let id = cx.cx_id;
            vault.put(cx).expect("put constellation");
            id
        })
        .collect::<Vec<_>>();
    vault.flush().expect("flush durable row CF");
    let snapshot = vault.latest_seq();

    let row_bytes = vault
        .read_cf_at(snapshot, ColumnFamily::slot(slot), &slot_key(cx_ids[0]))
        .expect("read first row")
        .expect("first row present");
    write_json(
        &root.join("row-codec-readback.json"),
        &json!({
            "slot": slot,
            "snapshot": snapshot,
            "first_cx": cx_ids[0],
            "row_hex": hex(&row_bytes),
            "row_prefix_hex": hex(&row_bytes[..9]),
            "row_codec_tag": row_bytes[0],
            "row_is_cxa1": row_bytes.starts_with(b"CXA1"),
            "decoded": decoded_values(&row_bytes),
        }),
    );

    let output_dir = root.join("materialized").join("slot_06");
    let materialized = vault
        .materialize_slot_column_at(snapshot, slot, &output_dir)
        .expect("materialize slot column");
    let readback =
        read_materialized_slot_column(&materialized.manifest_path).expect("read materialized");
    let chunk_bytes = fs::read(&materialized.chunk_path).expect("read chunk bytes");
    write_json(
        &root.join("slot-column-readback.json"),
        &json!({
            "manifest_path": materialized.manifest_path,
            "chunk_path": materialized.chunk_path,
            "manifest_sha256": materialized.manifest_sha256,
            "chunk_sha256": materialized.chunk_sha256,
            "chunk_prefix_hex": hex(&chunk_bytes[..16]),
            "chunk_payload_layout": "dimension-contiguous column-major f32 values",
            "chunk_payload_prefix_hex": hex(&chunk_bytes[16..40]),
            "chunk_is_cxa1": chunk_bytes.starts_with(b"CXA1"),
            "manifest": readback.manifest,
            "columns": column_major_values(
                &chunk_bytes,
                readback.manifest.rows,
                readback.manifest.dim as usize
            ),
            "rows": readback.rows.iter().map(|row| {
                json!({"cx_id": row.cx_id, "values": row.values})
            }).collect::<Vec<_>>(),
        }),
    );

    write_edge_readbacks(&root, slot);
    write_manifest(&root);
    println!("slot_column_fsv_root={}", root.display());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn write_edge_readbacks(root: &Path, slot: SlotId) {
    let edge_root = root.join("edges");
    fs::create_dir_all(&edge_root).expect("create edge root");
    let empty = AsterVault::with_clock(vault_id(), b"edge-empty".to_vec(), FixedClock::new(20));
    let empty_error = empty
        .materialize_slot_column_at(empty.latest_seq(), slot, edge_root.join("empty"))
        .expect_err("empty slot rejected");

    let absent = AsterVault::with_clock(vault_id(), b"edge-absent".to_vec(), FixedClock::new(21));
    absent
        .put(constellation(
            &absent,
            b"absent",
            slot,
            SlotVector::Absent {
                reason: AbsentReason::Deferred,
            },
        ))
        .expect("put absent");
    let absent_error = absent
        .materialize_slot_column_at(absent.latest_seq(), slot, edge_root.join("absent"))
        .expect_err("absent slot rejected");

    let corrupt = AsterVault::with_clock(vault_id(), b"edge-corrupt".to_vec(), FixedClock::new(22));
    corrupt
        .put(constellation(
            &corrupt,
            b"corrupt",
            slot,
            SlotVector::Dense {
                dim: 2,
                data: vec![1.0, 2.0],
            },
        ))
        .expect("put corrupt fixture");
    let corrupt_output = edge_root.join("corrupt");
    let materialized = corrupt
        .materialize_slot_column_at(corrupt.latest_seq(), slot, &corrupt_output)
        .expect("materialize corrupt fixture");
    let mut chunk = fs::read(&materialized.chunk_path).expect("read corrupt chunk");
    let last = chunk.len() - 1;
    chunk[last] ^= 0x01;
    fs::write(&materialized.chunk_path, chunk).expect("write corrupt chunk");
    let corrupt_error = read_materialized_slot_column(&materialized.manifest_path)
        .expect_err("corrupt chunk rejected");
    let mut bad_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&materialized.manifest_path).expect("read manifest"))
            .expect("manifest json");
    bad_manifest["chunk_file"] = json!("../slot-column.cxa1");
    let bad_manifest_path = corrupt_output.join("bad-manifest-path.json");
    write_json(&bad_manifest_path, &bad_manifest);
    let path_error =
        read_materialized_slot_column(&bad_manifest_path).expect_err("bad path rejected");

    write_json(
        &root.join("edge-readback.json"),
        &json!({
            "empty_slot_error": empty_error.code,
            "absent_slot_error": absent_error.code,
            "corrupt_chunk_error": corrupt_error.code,
            "bad_manifest_path_error": path_error.code,
        }),
    );
}

fn constellation(
    vault: &AsterVault<impl Clock>,
    input: &[u8],
    slot: SlotId,
    vector: SlotVector,
) -> Constellation {
    let cx_id = vault.cx_id_for_input(input, 1);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input);
    let mut slots = BTreeMap::new();
    slots.insert(slot, vector);
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 10,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://{}", String::from_utf8_lossy(input))),
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

fn decoded_values(bytes: &[u8]) -> Vec<f32> {
    match calyx_aster::vault::encode::decode_slot_vector(bytes).expect("decode row") {
        SlotVector::Dense { data, .. } => data,
        _ => Vec::new(),
    }
}

fn column_major_values(bytes: &[u8], rows: usize, dim: usize) -> Vec<Vec<f32>> {
    let payload = &bytes[16..];
    (0..dim)
        .map(|column| {
            (0..rows)
                .map(|row| {
                    let offset = (column * rows + row) * 4;
                    f32::from_le_bytes(payload[offset..offset + 4].try_into().expect("f32"))
                })
                .collect()
        })
        .collect()
}

fn write_manifest(root: &Path) {
    let mut entries = Vec::new();
    collect_files(root, &mut entries);
    entries.sort();
    let mut lines = Vec::new();
    for path in entries {
        if path.file_name().and_then(|value| value.to_str()) == Some("SHA256SUMS.txt") {
            continue;
        }
        let bytes = fs::read(&path).expect("read artifact");
        let name = path
            .strip_prefix(root)
            .expect("relative path")
            .to_string_lossy()
            .replace('\\', "/");
        lines.push(format!("{:x}  {name}\n", Sha256::digest(bytes)));
    }
    fs::write(root.join("SHA256SUMS.txt"), lines.concat()).expect("write sha manifest");
}

fn collect_files(dir: &Path, entries: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            collect_files(&path, entries);
        } else if path.is_file() {
            entries.push(path);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}
