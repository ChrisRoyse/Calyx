use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, CxFlags, CxId, InputRef, LedgerRef, LensId, Modality, QuantPolicy, Slot, SlotId,
    SlotKey, SlotShape, SlotState, SlotVector, VaultId, VaultStore,
};
use calyx_forge::AssayQuantSafety;
use calyx_registry::frozen::{NormPolicy, sha256_digest};
use calyx_registry::{
    CALYX_VECTOR_COMPRESSION_INVALID, LensRuntime, LensSpec, MxFp4AssayEvidence, StoredSlotCodec,
    decode_stored_slot_envelope, write_compressed_slot_batch,
    write_compressed_slot_batch_with_assay_evidence,
};
use serde_json::json;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn issue934_mxfp4_requires_current_evidence_and_persists_exact_codecs() {
    let root = temp_root("issue934-mxfp");
    let _ = fs::remove_dir_all(&root);
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).unwrap();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue934-mxfp-no-fallback".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let rows = fixture_rows(64);
    let mxfp4_slot = make_slot(
        "mxfp4-exact",
        SlotId::new(7),
        SlotShape::Dense(64),
        QuantPolicy::MxFp4,
    );
    let mxfp8_slot = make_slot(
        "mxfp8-explicit",
        SlotId::new(8),
        SlotShape::Dense(64),
        QuantPolicy::Float8,
    );
    let mxfp4_lens = lens_spec("mxfp4-exact", QuantPolicy::MxFp4, 64);
    let mxfp8_lens = lens_spec("mxfp8-explicit", QuantPolicy::Float8, 64);

    for (idx, (cx_id, values)) in rows.iter().enumerate() {
        vault
            .put(constellation_multi(
                *cx_id,
                idx as u64,
                vec![
                    (mxfp4_slot.slot_id, values.clone()),
                    (mxfp8_slot.slot_id, values.clone()),
                ],
            ))
            .unwrap();
    }
    vault.flush().unwrap();

    let first_cx = rows[0].0;
    let before_snapshot = vault.snapshot();
    let before_mxfp4 = read_slot_bytes(&vault, before_snapshot, mxfp4_slot.slot_id, first_cx);
    let before_mxfp8 = read_slot_bytes(&vault, before_snapshot, mxfp8_slot.slot_id, first_cx);

    let no_evidence = write_compressed_slot_batch_with_assay_evidence(
        &vault,
        &mxfp4_slot,
        &mxfp4_lens,
        &rows,
        &[],
        2,
        None,
    )
    .unwrap_err();
    let stale = write_compressed_slot_batch_with_assay_evidence(
        &vault,
        &mxfp4_slot,
        &mxfp4_lens,
        &rows,
        &[],
        2,
        Some(&evidence(&mxfp4_slot, &mxfp4_lens, 64, 10, 11)),
    )
    .unwrap_err();
    let mut wrong_slot = evidence(&mxfp4_slot, &mxfp4_lens, 64, 11, 11);
    wrong_slot.slot_id = mxfp8_slot.slot_id.get();
    let wrong_slot = write_compressed_slot_batch_with_assay_evidence(
        &vault,
        &mxfp4_slot,
        &mxfp4_lens,
        &rows,
        &[],
        2,
        Some(&wrong_slot),
    )
    .unwrap_err();
    let wrong_dim = write_compressed_slot_batch_with_assay_evidence(
        &vault,
        &mxfp4_slot,
        &mxfp4_lens,
        &rows,
        &[],
        2,
        Some(&evidence(&mxfp4_slot, &mxfp4_lens, 63, 11, 11)),
    )
    .unwrap_err();

    for error in [&no_evidence, &stale, &wrong_slot, &wrong_dim] {
        assert_eq!(error.code, CALYX_VECTOR_COMPRESSION_INVALID);
        assert!(error.message.contains("no fallback codec was written"));
    }
    let after_edges_snapshot = vault.snapshot();
    assert_eq!(after_edges_snapshot, before_snapshot);
    let after_edges_mxfp4 = read_slot_bytes(&vault, before_snapshot, mxfp4_slot.slot_id, first_cx);
    let after_edges_mxfp8 = read_slot_bytes(&vault, before_snapshot, mxfp8_slot.slot_id, first_cx);
    assert_eq!(before_mxfp4, after_edges_mxfp4);
    assert_eq!(before_mxfp8, after_edges_mxfp8);

    let good_evidence = evidence(&mxfp4_slot, &mxfp4_lens, 64, 11, 11);
    let mxfp4_report = write_compressed_slot_batch_with_assay_evidence(
        &vault,
        &mxfp4_slot,
        &mxfp4_lens,
        &rows,
        &[],
        2,
        Some(&good_evidence),
    )
    .unwrap();
    let mxfp8_report =
        write_compressed_slot_batch(&vault, &mxfp8_slot, &mxfp8_lens, &rows, &[], 2).unwrap();
    vault.flush().unwrap();

    let snapshot = mxfp8_report.snapshot.unwrap();
    let mxfp4_compressed = read_slot_bytes(&vault, snapshot, mxfp4_slot.slot_id, first_cx).unwrap();
    let mxfp8_compressed = read_slot_bytes(&vault, snapshot, mxfp8_slot.slot_id, first_cx).unwrap();
    let mxfp4_raw = read_raw_bytes(&vault, snapshot, mxfp4_slot.slot_id, first_cx).unwrap();
    let mxfp8_raw = read_raw_bytes(&vault, snapshot, mxfp8_slot.slot_id, first_cx).unwrap();
    let mxfp4_envelope = decode_stored_slot_envelope(&mxfp4_compressed).unwrap();
    let mxfp8_envelope = decode_stored_slot_envelope(&mxfp8_compressed).unwrap();

    assert_eq!(mxfp4_report.stored_codec, StoredSlotCodec::MxFp4);
    assert_eq!(mxfp8_report.stored_codec, StoredSlotCodec::MxFp8);
    assert_eq!(mxfp4_envelope.codec, StoredSlotCodec::MxFp4);
    assert_eq!(mxfp4_envelope.level, "Bits4Fp");
    assert!(!mxfp4_envelope.fallback);
    assert_eq!(mxfp8_envelope.codec, StoredSlotCodec::MxFp8);
    assert_eq!(mxfp8_envelope.level, "Bits8Fp");
    assert!(!mxfp8_envelope.fallback);

    write_json(
        &root.join("issue934-mxfp-no-fallback-readback.json"),
        &json!({
            "source_of_truth": "Aster durable vault slot_07, slot_08, slot_07.raw, and slot_08.raw CF rows decoded after compression",
            "vault_dir": vault_dir,
            "before_edges": {
                "snapshot": before_snapshot,
                "slot_07": bytes_state(&before_mxfp4),
                "slot_08": bytes_state(&before_mxfp8),
            },
            "edge_errors": [
                error_state("no_evidence", &no_evidence),
                error_state("stale", &stale),
                error_state("wrong_slot", &wrong_slot),
                error_state("wrong_dim", &wrong_dim),
            ],
            "after_edges": {
                "snapshot": after_edges_snapshot,
                "slot_07": bytes_state(&after_edges_mxfp4),
                "slot_08": bytes_state(&after_edges_mxfp8),
            },
            "success": {
                "snapshot": snapshot,
                "mxfp4": {
                    "slot": 7,
                    "requested_quant": "mx_fp4",
                    "stored_codec": format!("{:?}", mxfp4_report.stored_codec),
                    "envelope": mxfp4_envelope,
                    "compressed_prefix_hex": hex(&mxfp4_compressed[..mxfp4_compressed.len().min(32)]),
                    "raw_sidecar_prefix_hex": hex(&mxfp4_raw[..mxfp4_raw.len().min(32)]),
                },
                "mxfp8": {
                    "slot": 8,
                    "requested_quant": "float8",
                    "stored_codec": format!("{:?}", mxfp8_report.stored_codec),
                    "envelope": mxfp8_envelope,
                    "compressed_prefix_hex": hex(&mxfp8_compressed[..mxfp8_compressed.len().min(32)]),
                    "raw_sidecar_prefix_hex": hex(&mxfp8_raw[..mxfp8_raw.len().min(32)]),
                },
            },
        }),
    );

    if !keep_fsv_root() {
        fs::remove_dir_all(root).unwrap();
    }
}

fn evidence(
    slot: &Slot,
    lens: &LensSpec,
    dim: u32,
    written_at_seq: u64,
    current_seq: u64,
) -> MxFp4AssayEvidence {
    MxFp4AssayEvidence {
        slot_id: slot.slot_id.get(),
        slot_key: slot.slot_key.key().to_string(),
        lens_id: lens.lens_id(),
        dim,
        written_at_seq,
        current_seq,
        safety: AssayQuantSafety {
            baseline_bits: 1.0,
            quantized_bits: 0.97,
            cosine: 0.995,
            far_delta: 0.005,
        },
    }
}

fn fixture_rows(dim: usize) -> Vec<(CxId, Vec<f32>)> {
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
            (CxId::from_bytes(bytes), values)
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

fn lens_spec(name: &str, quant_default: QuantPolicy, dim: u32) -> LensSpec {
    let weights = sha256_digest(&[name.as_bytes(), b"weights"]);
    let corpus = sha256_digest(&[name.as_bytes(), b"corpus"]);
    LensSpec {
        name: name.to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "issue934-mxfp-no-fallback".to_string(),
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
        truncate_dim: None,
        recall_delta: 1.0,
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}

fn constellation_multi(
    cx_id: CxId,
    seq: u64,
    slot_vectors: Vec<(SlotId, Vec<f32>)>,
) -> calyx_core::Constellation {
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
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1_785_400_934 + seq,
        input_ref: InputRef {
            hash: sha256_digest(&[cx_id.as_bytes()]),
            pointer: Some(format!("synthetic://issue934/{seq}")),
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

fn read_slot_bytes<C: calyx_core::Clock>(
    vault: &AsterVault<C>,
    snapshot: u64,
    slot_id: SlotId,
    cx_id: CxId,
) -> Option<Vec<u8>> {
    vault
        .read_cf_at(snapshot, ColumnFamily::slot(slot_id), &slot_key(cx_id))
        .unwrap()
}

fn read_raw_bytes<C: calyx_core::Clock>(
    vault: &AsterVault<C>,
    snapshot: u64,
    slot_id: SlotId,
    cx_id: CxId,
) -> Option<Vec<u8>> {
    vault
        .read_cf_at(snapshot, ColumnFamily::slot_raw(slot_id), &slot_key(cx_id))
        .unwrap()
}

fn bytes_state(bytes: &Option<Vec<u8>>) -> serde_json::Value {
    match bytes {
        Some(bytes) => json!({
            "exists": true,
            "len": bytes.len(),
            "prefix_hex": hex(&bytes[..bytes.len().min(32)]),
        }),
        None => json!({"exists": false}),
    }
}

fn error_state(name: &str, error: &calyx_core::CalyxError) -> serde_json::Value {
    json!({
        "case": name,
        "code": error.code,
        "message": error.message,
    })
}

fn temp_root(label: &str) -> PathBuf {
    if let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") {
        return root;
    }
    let serial = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("calyx-{label}-{}-{serial}", std::process::id()))
}

fn keep_fsv_root() -> bool {
    calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_some()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
