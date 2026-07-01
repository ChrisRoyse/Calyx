use super::*;

pub(super) fn fixture_rows(dim: usize) -> Vec<(calyx_core::CxId, Vec<f32>)> {
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

pub(super) fn opposing_rows() -> Vec<(calyx_core::CxId, Vec<f32>)> {
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

pub(super) fn mrl_rows() -> Vec<(calyx_core::CxId, Vec<f32>)> {
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

pub(super) fn make_slot(name: &str, slot_id: SlotId, shape: SlotShape, quant: QuantPolicy) -> Slot {
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

pub(super) fn panel_with_slots(slots: Vec<Slot>) -> calyx_core::Panel {
    calyx_core::Panel {
        version: 1,
        slots,
        created_at: 1_785_400_000,
        kernel_ref: None,
        guard_ref: None,
    }
}

pub(super) fn lens_spec(
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

pub(super) fn constellation_multi(
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

pub(super) fn temp_root(label: &str) -> PathBuf {
    if let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") {
        return root;
    }
    let serial = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("calyx-{label}-{}-{serial}", std::process::id()))
}

pub(super) fn keep_fsv_root() -> bool {
    calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_some()
}

pub(super) fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

pub(super) fn maybe_write_json(name: &str, value: &serde_json::Value) {
    if let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") {
        fs::create_dir_all(&root).unwrap();
        write_json(&root.join(name), value);
    }
}

pub(super) fn write_json(path: &PathBuf, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
