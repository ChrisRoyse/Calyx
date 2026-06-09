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
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn slot_column_materialization_fsv_writes_readbacks() {
    let (root, keep_root) = fsv_root("slot-column-fsv");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create fsv root");

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
            "chunk_is_cxa1": chunk_bytes.starts_with(b"CXA1"),
            "manifest": readback.manifest,
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

    write_json(
        &root.join("edge-readback.json"),
        &json!({
            "empty_slot_error": empty_error.code,
            "absent_slot_error": absent_error.code,
            "corrupt_chunk_error": corrupt_error.code,
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

fn write_manifest(root: &Path) {
    let mut entries = fs::read_dir(root)
        .expect("read root")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    entries.sort();
    let mut lines = Vec::new();
    for path in entries {
        if path.file_name().and_then(|value| value.to_str()) == Some("SHA256SUMS.txt") {
            continue;
        }
        let bytes = fs::read(&path).expect("read artifact");
        let name = path.file_name().expect("file name").to_string_lossy();
        lines.push(format!("{:x}  {}\n", Sha256::digest(bytes), name));
    }
    fs::write(root.join("SHA256SUMS.txt"), lines.concat()).expect("write sha manifest");
}

fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn fsv_root(name: &str) -> (PathBuf, bool) {
    if let Ok(root) = std::env::var("CALYX_ASTER_SLOT_COLUMN_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    (
        std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id())),
        false,
    )
}
