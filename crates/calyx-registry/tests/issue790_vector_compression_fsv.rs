use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, Constellation, CxFlags, InputRef, LedgerRef, LensId, Modality, QuantPolicy, Slot,
    SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId, VaultStore,
};
use calyx_registry::frozen::{NormPolicy, sha256_digest};
use calyx_registry::{
    CALYX_VECTOR_COMPRESSION_EMPTY, CALYX_VECTOR_COMPRESSION_INVALID, LensRuntime, LensSpec,
    Registry, StoredSlotCodec, compress_slot_batch, decode_stored_slot_envelope,
    matryoshka_truncate_renormalize, persist_vault_panel_state, write_compressed_slot_batch,
};
use serde_json::json;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn turboquant_and_mxfp4_roundtrip_fixture_vectors() {
    let rows = fixture_rows(96);
    let slot = make_slot(
        "semantic",
        SlotId::new(0),
        SlotShape::Dense(96),
        QuantPolicy::turboquant_default(),
    );
    let turbo = lens_spec("turbo", slot.quant, None, 96, 0.25);
    let turbo_report = compress_slot_batch(&slot, &turbo, &rows, &[], 2).unwrap();

    assert_eq!(
        turbo_report.stored_codec,
        StoredSlotCodec::TurboQuantBits3p5
    );
    assert!(turbo_report.stored_bytes_total < turbo_report.raw_bytes_total);
    assert!(turbo_report.recall_at_k_compressed >= 0.75);

    let mxfp_slot = make_slot(
        "mxfp",
        SlotId::new(1),
        SlotShape::Dense(96),
        QuantPolicy::MxFp4,
    );
    let mxfp = lens_spec("mxfp", QuantPolicy::MxFp4, None, 96, 1.0);
    let mxfp_report = compress_slot_batch(&mxfp_slot, &mxfp, &rows, &[], 2).unwrap();

    assert!(matches!(
        mxfp_report.stored_codec,
        StoredSlotCodec::MxFp4 | StoredSlotCodec::MxFp8 | StoredSlotCodec::RawF32
    ));
    assert!(mxfp_report.recall_at_k_compressed >= 0.0);
    maybe_write_json(
        "roundtrip-codecs.json",
        &json!({
            "turbo_codec": "turbo_quant_bits3p5",
            "turbo_raw_bytes": turbo_report.raw_bytes_total,
            "turbo_stored_bytes": turbo_report.stored_bytes_total,
            "turbo_recall_at_k": turbo_report.recall_at_k_compressed,
            "mxfp_codec": format!("{:?}", mxfp_report.stored_codec),
            "mxfp_recall_at_k": mxfp_report.recall_at_k_compressed,
        }),
    );
}

#[test]
fn matryoshka_truncate_renormalizes_prefix() {
    let raw = vec![3.0, 4.0, 12.0, 0.0];
    let truncated = matryoshka_truncate_renormalize(&raw, 2).unwrap();
    let norm = truncated
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();

    assert_eq!(truncated.len(), 2);
    assert!((norm - 1.0).abs() < 1e-6);
    assert!((truncated[0] - 0.6).abs() < 1e-6);
    assert!((truncated[1] - 0.8).abs() < 1e-6);
    maybe_write_json(
        "matryoshka-readback.json",
        &json!({
            "input": raw,
            "truncate_dim": 2,
            "output": truncated,
            "output_norm": norm,
        }),
    );
}

#[test]
fn compression_breach_falls_back_and_empty_batch_fails_closed() {
    let rows = opposing_rows();
    let slot = make_slot(
        "binary",
        SlotId::new(2),
        SlotShape::Dense(8),
        QuantPolicy::Binary,
    );
    let lens = lens_spec("binary", QuantPolicy::Binary, Some(1), 8, 0.0);
    let report = compress_slot_batch(&slot, &lens, &rows, &[], 1).unwrap();

    assert!(report.fallback_reason.is_some());
    assert_ne!(report.stored_codec, StoredSlotCodec::Binary);

    let error = compress_slot_batch(&slot, &lens, &[], &[], 1).unwrap_err();
    assert_eq!(error.code, CALYX_VECTOR_COMPRESSION_EMPTY);
    let invalid_query =
        compress_slot_batch(&slot, &lens, &rows, &[vec![f32::NAN; 8]], 1).unwrap_err();
    assert_eq!(invalid_query.code, CALYX_VECTOR_COMPRESSION_INVALID);
    maybe_write_json(
        "edge-fallback-and-empty.json",
        &json!({
            "breach_requested": "binary",
            "stored_codec": format!("{:?}", report.stored_codec),
            "fallback_reason": report.fallback_reason,
            "empty_error_code": error.code,
            "invalid_query_error_code": invalid_query.code,
        }),
    );
}

#[test]
fn compressed_vault_rows_use_slot_cf_and_raw_sidecar() {
    let root = temp_root("issue790-vault");
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).unwrap();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue790-vector-compression".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let rows = mrl_rows();
    let slot = make_slot(
        "mrl-semantic",
        SlotId::new(3),
        SlotShape::Dense(128),
        QuantPolicy::turboquant_default(),
    );
    let mxfp_slot = make_slot(
        "mxfp-companion",
        SlotId::new(4),
        SlotShape::Dense(128),
        QuantPolicy::MxFp4,
    );
    let lens = lens_spec("mrl-semantic", slot.quant, Some(64), 128, 0.02);
    let mxfp_lens = lens_spec("mxfp-companion", mxfp_slot.quant, None, 128, 1.0);
    let panel_vault = root.join("panel-status-vault");
    fs::create_dir_all(&panel_vault).unwrap();
    let panel = panel_with_slots(vec![slot.clone(), mxfp_slot.clone()]);
    let _panel_vault_handle = AsterVault::new_durable(
        &panel_vault,
        vault_id(),
        b"issue790-panel-status".to_vec(),
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&panel_vault, &panel, &Registry::new()).unwrap();

    for (idx, (cx_id, values)) in rows.iter().enumerate() {
        vault
            .put(constellation_multi(
                *cx_id,
                idx as u64,
                vec![
                    (slot.slot_id, values.clone()),
                    (mxfp_slot.slot_id, values.clone()),
                ],
            ))
            .unwrap();
    }
    let report = write_compressed_slot_batch(&vault, &slot, &lens, &rows, &[], 2).unwrap();
    let mxfp_report =
        write_compressed_slot_batch(&vault, &mxfp_slot, &mxfp_lens, &rows, &[], 2).unwrap();
    vault.flush().unwrap();
    let snapshot = mxfp_report.snapshot.unwrap();
    let first_cx = rows[0].0;
    let compressed = vault
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
    let envelope = decode_stored_slot_envelope(&compressed).unwrap();
    let mxfp_compressed = vault
        .read_cf_at(
            snapshot,
            ColumnFamily::slot(mxfp_slot.slot_id),
            &slot_key(first_cx),
        )
        .unwrap()
        .unwrap();
    let mxfp_envelope = decode_stored_slot_envelope(&mxfp_compressed).unwrap();
    let reconstructed = vault.get(first_cx, snapshot).unwrap();

    assert_eq!(compressed[0], calyx_registry::COMPRESSED_SLOT_TAG);
    assert_eq!(envelope.codec, StoredSlotCodec::TurboQuantBits3p5);
    assert_eq!(envelope.raw_dim, 128);
    assert_eq!(envelope.stored_dim, 64);
    assert!(matches!(
        mxfp_envelope.codec,
        StoredSlotCodec::MxFp4 | StoredSlotCodec::MxFp8
    ));
    assert!(report.stored_bytes_total < report.raw_bytes_total);
    assert!(mxfp_report.stored_bytes_total < mxfp_report.raw_bytes_total);
    assert_eq!(raw_sidecar[0], 0);
    assert_eq!(
        reconstructed.slots.get(&slot.slot_id),
        Some(&SlotVector::Dense {
            dim: 128,
            data: rows[0].1.clone()
        })
    );
    write_json(
        &root.join("summary.json"),
        &json!({
            "source_of_truth": "Aster durable vault CF rows slot_03, slot_04, slot_03.raw plus VaultStore::get sidecar fallback",
            "vault_dir": vault_dir,
            "panel_status_vault": panel_vault,
            "snapshot": snapshot,
            "slot_cf": {
                "cf": "slot_03",
                "len": compressed.len(),
                "tag": compressed[0],
                "prefix_hex": hex(&compressed[..compressed.len().min(32)]),
                "envelope": envelope,
            },
            "slot_04_cf": {
                "cf": "slot_04",
                "len": mxfp_compressed.len(),
                "tag": mxfp_compressed[0],
                "prefix_hex": hex(&mxfp_compressed[..mxfp_compressed.len().min(32)]),
                "envelope": mxfp_envelope,
            },
            "raw_sidecar": {
                "cf": "slot_03.raw",
                "len": raw_sidecar.len(),
                "tag": raw_sidecar[0],
                "prefix_hex": hex(&raw_sidecar[..raw_sidecar.len().min(32)]),
            },
            "compression_report": {
                "raw_bytes_total": report.raw_bytes_total,
                "stored_bytes_total": report.stored_bytes_total,
                "recall_at_k_raw": report.recall_at_k_raw,
                "recall_at_k_compressed": report.recall_at_k_compressed,
                "recall_delta": report.recall_delta,
                "stored_codec": format!("{:?}", report.stored_codec),
                "truncate_dim": report.truncate_dim,
            },
            "mxfp_report": {
                "raw_bytes_total": mxfp_report.raw_bytes_total,
                "stored_bytes_total": mxfp_report.stored_bytes_total,
                "recall_at_k_raw": mxfp_report.recall_at_k_raw,
                "recall_at_k_compressed": mxfp_report.recall_at_k_compressed,
                "stored_codec": format!("{:?}", mxfp_report.stored_codec),
            },
            "vault_get_reconstructed": {
                "matches_original": reconstructed.slots.get(&slot.slot_id) == Some(&SlotVector::Dense {
                    dim: 128,
                    data: rows[0].1.clone(),
                }),
                "dim": 128,
            },
        }),
    );

    if !keep_fsv_root() {
        fs::remove_dir_all(root).unwrap();
    }
}

fn fixture_rows(dim: usize) -> Vec<(calyx_core::CxId, Vec<f32>)> {
    (0..6)
        .map(|row| {
            let mut bytes = [0_u8; 16];
            bytes[15] = row as u8;
            let values = (0..dim)
                .map(|idx| {
                    let phase = (idx as f32 + 1.0) * (row as f32 + 1.0);
                    (phase.sin() + 0.25 * phase.cos()) / dim as f32
                })
                .collect();
            (calyx_core::CxId::from_bytes(bytes), values)
        })
        .collect()
}

fn opposing_rows() -> Vec<(calyx_core::CxId, Vec<f32>)> {
    [
        [1.0, 0.0, 0.0, 0.0, 0.9, 0.1, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0, -0.9, -0.1, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.9, 0.1, 0.0],
        [1.0, 0.0, 0.0, 0.0, 0.0, -0.9, -0.1, 0.0],
    ]
    .into_iter()
    .enumerate()
    .map(|(idx, values)| {
        let mut bytes = [0_u8; 16];
        bytes[15] = idx as u8;
        (calyx_core::CxId::from_bytes(bytes), values.to_vec())
    })
    .collect()
}

fn mrl_rows() -> Vec<(calyx_core::CxId, Vec<f32>)> {
    (0..6)
        .map(|row| {
            let mut bytes = [0_u8; 16];
            bytes[15] = (0xA0 + row) as u8;
            let mut values = vec![0.0; 128];
            for (idx, value) in values.iter_mut().take(64).enumerate() {
                let phase = (idx as f32 + 1.0) * (row as f32 + 1.0);
                *value = (phase.sin() + 0.25 * phase.cos()) / 64.0;
            }
            for (idx, value) in values.iter_mut().enumerate().take(128).skip(64) {
                *value = ((idx as f32 + row as f32).sin()) * 0.0001;
            }
            (calyx_core::CxId::from_bytes(bytes), values)
        })
        .collect()
}

fn make_slot(name: &str, slot_id: SlotId, shape: SlotShape, quant: QuantPolicy) -> Slot {
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, name),
        lens_id: LensId::from_bytes([slot_id.get() as u8; 16]),
        shape,
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant,
        resource: Default::default(),
        axis: Some(name.to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn panel_with_slots(slots: Vec<Slot>) -> calyx_core::Panel {
    calyx_core::Panel {
        version: 1,
        slots,
        created_at: 1_785_400_000,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn lens_spec(
    name: &str,
    quant_default: QuantPolicy,
    truncate_dim: Option<u32>,
    dim: u32,
    recall_delta: f32,
) -> LensSpec {
    let weights = sha256_digest(&[name.as_bytes(), b"weights"]);
    let corpus = sha256_digest(&[name.as_bytes(), b"corpus"]);
    LensSpec {
        name: name.to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "issue790-vector-compression".to_string(),
        },
        output: SlotShape::Dense(dim),
        modality: Modality::Text,
        weights_sha256: weights,
        corpus_hash: corpus,
        norm_policy: NormPolicy::None,
        max_batch: None,
        axis: Some(name.to_string()),
        asymmetry: Asymmetry::None,
        quant_default,
        truncate_dim,
        recall_delta,
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}

fn constellation_multi(
    cx_id: calyx_core::CxId,
    seq: u64,
    slot_vectors: Vec<(SlotId, Vec<f32>)>,
) -> Constellation {
    let mut slots = BTreeMap::new();
    for (slot_id, data) in slot_vectors {
        slots.insert(
            slot_id,
            SlotVector::Dense {
                dim: data.len() as u32,
                data,
            },
        );
    }
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1_785_400_000 + seq,
        input_ref: InputRef {
            hash: sha256_digest(&[cx_id.as_bytes()]),
            pointer: Some(format!("synthetic://issue790/{seq}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq,
            hash: [seq as u8; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn temp_root(label: &str) -> PathBuf {
    if let Ok(root) = std::env::var("CALYX_FSV_ROOT") {
        return PathBuf::from(root);
    }
    let serial = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("calyx-{label}-{}-{serial}", std::process::id()))
}

fn keep_fsv_root() -> bool {
    std::env::var_os("CALYX_FSV_ROOT").is_some()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn maybe_write_json(name: &str, value: &serde_json::Value) {
    if let Ok(root) = std::env::var("CALYX_FSV_ROOT") {
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        write_json(&root.join(name), value);
    }
}

fn write_json(path: &PathBuf, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
