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
        StoredSlotCodec::TurboQuantV4Bits3p5
    );
    assert!(turbo_report.stored_bytes_total < turbo_report.raw_bytes_total);
    assert!(turbo_report.recall_at_k_compressed >= 0.75);

    let mxfp8_slot = make_slot(
        "mxfp8",
        SlotId::new(1),
        SlotShape::Dense(96),
        QuantPolicy::Float8,
    );
    let mxfp8 = lens_spec("mxfp8", QuantPolicy::Float8, None, 96, 1.0);
    let mxfp8_report = compress_slot_batch(&mxfp8_slot, &mxfp8, &rows, &[], 2).unwrap();

    assert_eq!(mxfp8_report.stored_codec, StoredSlotCodec::MxFp8);
    assert!(mxfp8_report.recall_at_k_compressed >= 0.0);
    maybe_write_json(
        "roundtrip-codecs.json",
        &json!({
            "turbo_codec": "turbo_quant_v4_bits3p5",
            "turbo_raw_bytes": turbo_report.raw_bytes_total,
            "turbo_stored_bytes": turbo_report.stored_bytes_total,
            "turbo_recall_at_k": turbo_report.recall_at_k_compressed,
            "mxfp8_codec": format!("{:?}", mxfp8_report.stored_codec),
            "mxfp8_recall_at_k": mxfp8_report.recall_at_k_compressed,
        }),
    );
}

#[test]
fn scalar_int8_codec_has_real_bits8_envelope() {
    let rows = fixture_rows(32);
    let slot = make_slot(
        "scalar-int8",
        SlotId::new(6),
        SlotShape::Dense(32),
        QuantPolicy::TurboQuant {
            bits_per_channel_x2: 16,
        },
    );
    let lens = lens_spec("scalar-int8", slot.quant, None, 32, 0.25);
    let report = compress_slot_batch(&slot, &lens, &rows, &[], 2).unwrap();
    let envelope = decode_stored_slot_envelope(&report.rows[0].compressed_bytes).unwrap();

    assert_eq!(report.stored_codec, StoredSlotCodec::ScalarInt8);
    assert_eq!(envelope.codec, StoredSlotCodec::ScalarInt8);
    assert_eq!(envelope.level, "Bits8");
    assert_eq!(envelope.raw_dim, 32);
    assert_eq!(envelope.stored_dim, 32);
    assert!(!envelope.fallback);
    assert_eq!(envelope.payload_bytes, 32);
    assert!(report.fallback_reason.is_none());
    maybe_write_json(
        "scalar-int8-envelope.json",
        &json!({
            "source_of_truth": "compressed SlotCompressionRow bytes decoded independently by decode_stored_slot_envelope",
            "requested_quant": "turbo_quant bits_per_channel_x2=16",
            "stored_codec": format!("{:?}", report.stored_codec),
            "envelope": envelope,
            "row0_prefix_hex": hex(&report.rows[0].compressed_bytes[..32]),
        }),
    );
}

#[test]
fn turboquant_v1_v2_v3_and_v4_envelope_codes_dual_read() {
    let v1_bits3 = decode_stored_slot_envelope(&minimal_envelope(1, 4)).unwrap();
    let v1_bits2 = decode_stored_slot_envelope(&minimal_envelope(2, 5)).unwrap();
    let v2_bits3 = decode_stored_slot_envelope(&minimal_envelope(7, 4)).unwrap();
    let v2_bits2 = decode_stored_slot_envelope(&minimal_envelope(8, 5)).unwrap();
    let v3_bits3 = decode_stored_slot_envelope(&minimal_envelope(9, 4)).unwrap();
    let v3_bits2 = decode_stored_slot_envelope(&minimal_envelope(10, 5)).unwrap();
    let v4_bits3 = decode_stored_slot_envelope(&minimal_envelope(11, 4)).unwrap();
    let v4_bits2 = decode_stored_slot_envelope(&minimal_envelope(12, 5)).unwrap();

    assert_eq!(v1_bits3.codec, StoredSlotCodec::TurboQuantBits3p5);
    assert_eq!(v1_bits2.codec, StoredSlotCodec::TurboQuantBits2p5);
    assert_eq!(v2_bits3.codec, StoredSlotCodec::TurboQuantV2Bits3p5);
    assert_eq!(v2_bits2.codec, StoredSlotCodec::TurboQuantV2Bits2p5);
    assert_eq!(v3_bits3.codec, StoredSlotCodec::TurboQuantV3Bits3p5);
    assert_eq!(v3_bits2.codec, StoredSlotCodec::TurboQuantV3Bits2p5);
    assert_eq!(v4_bits3.codec, StoredSlotCodec::TurboQuantV4Bits3p5);
    assert_eq!(v4_bits2.codec, StoredSlotCodec::TurboQuantV4Bits2p5);
    assert_eq!(v2_bits3.level, "Bits3p5");
    assert_eq!(v2_bits2.level, "Bits2p5");
    assert_eq!(v3_bits3.level, "Bits3p5");
    assert_eq!(v3_bits2.level, "Bits2p5");
    assert_eq!(v4_bits3.level, "Bits3p5");
    assert_eq!(v4_bits2.level, "Bits2p5");
    maybe_write_json(
        "turboquant-dual-read-envelope.json",
        &json!({
            "legacy_bits3_codec": format!("{:?}", v1_bits3.codec),
            "legacy_bits2_codec": format!("{:?}", v1_bits2.codec),
            "v2_bits3_codec": format!("{:?}", v2_bits3.codec),
            "v2_bits2_codec": format!("{:?}", v2_bits2.codec),
            "v3_bits3_codec": format!("{:?}", v3_bits3.codec),
            "v3_bits2_codec": format!("{:?}", v3_bits2.codec),
            "v4_bits3_codec": format!("{:?}", v4_bits3.codec),
            "v4_bits2_codec": format!("{:?}", v4_bits2.codec),
            "source_of_truth": "decode_stored_slot_envelope over explicit envelope codec bytes",
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
fn compression_breach_empty_batch_and_invalid_envelope_fail_closed() {
    let rows = opposing_rows();
    let slot = make_slot(
        "binary",
        SlotId::new(2),
        SlotShape::Dense(8),
        QuantPolicy::Binary,
    );
    let lens = lens_spec("binary", QuantPolicy::Binary, Some(1), 8, 0.0);
    let breach_error = compress_slot_batch(&slot, &lens, &rows, &[], 1).unwrap_err();

    assert_eq!(breach_error.code, CALYX_VECTOR_COMPRESSION_INVALID);
    assert!(
        breach_error
            .message
            .contains("no fallback codec was written")
    );

    let error = compress_slot_batch(&slot, &lens, &[], &[], 1).unwrap_err();
    assert_eq!(error.code, CALYX_VECTOR_COMPRESSION_EMPTY);
    let invalid_query =
        compress_slot_batch(&slot, &lens, &rows, &[vec![f32::NAN; 8]], 1).unwrap_err();
    assert_eq!(invalid_query.code, CALYX_VECTOR_COMPRESSION_INVALID);
    let mut mismatched = vec![calyx_registry::COMPRESSED_SLOT_TAG, 1, 1, 1];
    mismatched.extend_from_slice(&8_u32.to_be_bytes());
    mismatched.extend_from_slice(&8_u32.to_be_bytes());
    mismatched.push(0);
    mismatched.extend_from_slice(&1.0_f32.to_bits().to_be_bytes());
    mismatched.extend_from_slice(&[0_u8; 32]);
    mismatched.extend_from_slice(&0_u32.to_be_bytes());
    let envelope_error = decode_stored_slot_envelope(&mismatched).unwrap_err();
    assert_eq!(envelope_error.code, CALYX_VECTOR_COMPRESSION_INVALID);
    maybe_write_json(
        "edge-fail-closed.json",
        &json!({
            "breach_requested": "binary",
            "breach_error_code": breach_error.code,
            "breach_error_message": breach_error.message,
            "empty_error_code": error.code,
            "invalid_query_error_code": invalid_query.code,
            "mismatched_envelope_error_code": envelope_error.code,
            "mismatched_envelope_error_message": envelope_error.message,
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
    let mxfp8_slot = make_slot(
        "mxfp8-companion",
        SlotId::new(4),
        SlotShape::Dense(128),
        QuantPolicy::Float8,
    );
    let lens = lens_spec("mrl-semantic", slot.quant, Some(64), 128, 0.34);
    let mxfp8_lens = lens_spec("mxfp8-companion", mxfp8_slot.quant, None, 128, 1.0);
    let panel_vault = root.join("panel-status-vault");
    fs::create_dir_all(&panel_vault).unwrap();
    let panel = panel_with_slots(vec![slot.clone(), mxfp8_slot.clone()]);
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
                    (mxfp8_slot.slot_id, values.clone()),
                ],
            ))
            .unwrap();
    }
    let report = write_compressed_slot_batch(&vault, &slot, &lens, &rows, &[], 2).unwrap();
    let mxfp8_report =
        write_compressed_slot_batch(&vault, &mxfp8_slot, &mxfp8_lens, &rows, &[], 2).unwrap();
    vault.flush().unwrap();
    let snapshot = mxfp8_report.snapshot.unwrap();
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
    let mxfp8_compressed = vault
        .read_cf_at(
            snapshot,
            ColumnFamily::slot(mxfp8_slot.slot_id),
            &slot_key(first_cx),
        )
        .unwrap()
        .unwrap();
    let mxfp8_envelope = decode_stored_slot_envelope(&mxfp8_compressed).unwrap();
    let vault_get_error = vault
        .get(first_cx, snapshot)
        .expect_err("VaultStore::get must not raw-sidecar fallback compressed slot rows");

    assert_eq!(compressed[0], calyx_registry::COMPRESSED_SLOT_TAG);
    assert_eq!(envelope.codec, StoredSlotCodec::TurboQuantV4Bits3p5);
    assert!(!envelope.fallback);
    assert_eq!(envelope.raw_dim, 128);
    assert_eq!(envelope.stored_dim, 64);
    assert_eq!(mxfp8_envelope.codec, StoredSlotCodec::MxFp8);
    assert!(report.stored_bytes_total < report.raw_bytes_total);
    assert!(mxfp8_report.stored_bytes_total < mxfp8_report.raw_bytes_total);
    assert_eq!(raw_sidecar[0], 0);
    assert_eq!(vault_get_error.code, "CALYX_ASTER_CORRUPT_SHARD");
    write_json(
        &root.join("summary.json"),
        &json!({
            "source_of_truth": "Aster durable vault CF rows slot_03, slot_04, slot_03.raw; VaultStore::get fails closed on compressed slot CF rows",
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
                "len": mxfp8_compressed.len(),
                "tag": mxfp8_compressed[0],
                "prefix_hex": hex(&mxfp8_compressed[..mxfp8_compressed.len().min(32)]),
                "envelope": mxfp8_envelope,
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
            "mxfp8_report": {
                "raw_bytes_total": mxfp8_report.raw_bytes_total,
                "stored_bytes_total": mxfp8_report.stored_bytes_total,
                "recall_at_k_raw": mxfp8_report.recall_at_k_raw,
                "recall_at_k_compressed": mxfp8_report.recall_at_k_compressed,
                "stored_codec": format!("{:?}", mxfp8_report.stored_codec),
            },
            "vault_get_error": {
                "code": vault_get_error.code,
                "message": vault_get_error.message,
            },
        }),
    );

    if !keep_fsv_root() {
        fs::remove_dir_all(root).unwrap();
    }
}

fn minimal_envelope(codec_code: u8, level_code: u8) -> Vec<u8> {
    let mut envelope = vec![
        calyx_registry::COMPRESSED_SLOT_TAG,
        1,
        codec_code,
        level_code,
    ];
    envelope.extend_from_slice(&8_u32.to_be_bytes());
    envelope.extend_from_slice(&8_u32.to_be_bytes());
    envelope.push(0);
    envelope.extend_from_slice(&1.0_f32.to_bits().to_be_bytes());
    envelope.extend_from_slice(&[0_u8; 32]);
    envelope.extend_from_slice(&0_u32.to_be_bytes());
    envelope
}

#[path = "issue790_vector_compression_fsv/support.rs"]
mod support;
use support::*;
