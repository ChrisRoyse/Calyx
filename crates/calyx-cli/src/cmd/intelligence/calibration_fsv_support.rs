use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Asymmetry, CxFlags, CxId, InputRef, LedgerRef, LensId,
    Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId,
    VaultStore,
};

pub(super) const KNOWN_PAIR_ANCHOR: &str = "metformin_type_2_diabetes";
pub(super) const MEDLINEPLUS_URL: &str = "https://medlineplus.gov/druginfo/meds/a696005.html";
pub(super) const DOMAIN: &str = "issue873_metformin_type_2_diabetes";

pub(super) fn put_oracle_evidence(
    vault: &AsterVault,
    panel: &Panel,
    panel_bits: f32,
    entropy_bits: f32,
    slot_bits: &[(u16, f32)],
) {
    let mut store = AssayStore::default();
    let key = AssayCacheKey::scoped(panel.version, DOMAIN, vault_id(), AnchorKind::Reward);
    store.put(
        key.clone(),
        AssaySubject::Panel,
        estimate(panel_bits, EstimatorKind::PanelSufficiency),
        "issue873 oracle panel bits",
        1,
    );
    store.put(
        key.clone(),
        AssaySubject::OutcomeEntropy,
        estimate(entropy_bits, EstimatorKind::OutcomeEntropy),
        "issue873 oracle outcome entropy",
        1,
    );
    for (slot, bits) in slot_bits {
        store.put(
            key.clone(),
            AssaySubject::Lens {
                slot: SlotId::new(*slot),
            },
            estimate(*bits, EstimatorKind::Ksg),
            "issue873 oracle lens bits",
            1,
        );
    }
    assert_eq!(store.persist_to_vault(vault).expect("persist assay CF"), 4);
}

pub(super) fn planted_known_pair_corpus(vault_id: VaultId) -> Vec<calyx_core::Constellation> {
    (0..100)
        .map(|idx| {
            let positive = idx < 50;
            let mut metadata = BTreeMap::new();
            metadata.insert(
                "drug".to_string(),
                if positive { "metformin" } else { "control" }.to_string(),
            );
            metadata.insert(
                "disease".to_string(),
                if positive {
                    "type_2_diabetes"
                } else {
                    "control_outcome"
                }
                .to_string(),
            );
            metadata.insert("source_url".to_string(), MEDLINEPLUS_URL.to_string());
            constellation(
                idx as u8,
                vault_id,
                metadata,
                vec![Anchor {
                    kind: AnchorKind::Label(KNOWN_PAIR_ANCHOR.to_string()),
                    value: AnchorValue::Bool(positive),
                    source: MEDLINEPLUS_URL.to_string(),
                    observed_at: 1_785_500_000,
                    confidence: 1.0,
                }],
                if positive {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                },
                vec![1.0, 0.0],
            )
        })
        .collect()
}

pub(super) fn ungrounded_corpus(vault_id: VaultId) -> Vec<calyx_core::Constellation> {
    (0..5)
        .map(|idx| {
            constellation(
                (idx + 150) as u8,
                vault_id,
                BTreeMap::new(),
                Vec::new(),
                vec![1.0, 0.0],
                vec![1.0, 0.0],
            )
        })
        .collect()
}

fn constellation(
    seed: u8,
    vault_id: VaultId,
    metadata: BTreeMap<String, String>,
    anchors: Vec<Anchor>,
    slot0: Vec<f32>,
    slot1: Vec<f32>,
) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: slot0,
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Dense {
            dim: 2,
            data: slot1,
        },
    );
    calyx_core::Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id,
        panel_version: 873,
        created_at: 1_785_500_000 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors,
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags {
            ungrounded: false,
            degraded: false,
            novel_region: false,
            redacted_input: false,
        },
    }
}

pub(super) fn panel_two_active() -> Panel {
    Panel {
        version: 873,
        slots: vec![slot(0), slot(1)],
        created_at: 1_785_500_000,
        kernel_ref: None,
        guard_ref: None,
    }
}

pub(super) fn slot(id: u16) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("issue873-slot-{id}")),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("issue873-calibration".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 873,
    }
}

pub(super) fn durable_vault(dir: &Path, vault_id: VaultId) -> AsterVault {
    fs::remove_dir_all(dir).ok();
    fs::create_dir_all(dir).expect("create durable vault dir");
    AsterVault::new_durable(
        dir,
        vault_id,
        b"issue873-calibration-salt",
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

pub(super) fn scan_count(vault: &AsterVault, cf: ColumnFamily) -> usize {
    vault
        .scan_cf_at(vault.snapshot(), cf)
        .expect("scan CF")
        .len()
}

fn estimate(bits: f32, estimator: EstimatorKind) -> MiEstimate {
    MiEstimate::point(bits, 120, estimator, TrustTag::Trusted)
}

pub(super) fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

pub(super) fn assert_close_f32(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1.0e-6,
        "actual={actual} expected={expected}"
    );
}

pub(super) fn assert_close_f64(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-9,
        "actual={actual} expected={expected}"
    );
}

pub(super) struct FsvPaths {
    pub case_dir: PathBuf,
    pub artifact_path: PathBuf,
    pub keep: bool,
}

impl FsvPaths {
    pub fn new() -> Self {
        let (root, keep) = match std::env::var_os("CALYX_FSV_ROOT") {
            Some(value) => (PathBuf::from(value), true),
            None => (
                std::env::temp_dir().join(format!(
                    "calyx-issue873-calibration-fsv-{}",
                    std::process::id()
                )),
                false,
            ),
        };
        fs::create_dir_all(&root).expect("create FSV root");
        let case_dir = root.join(format!("issue873-calibration-case-{}", std::process::id()));
        fs::remove_dir_all(&case_dir).ok();
        fs::create_dir_all(&case_dir).expect("create FSV case root");
        let artifact_path = root.join("issue873_calibration_fsv_readback.json");
        Self {
            case_dir,
            artifact_path,
            keep,
        }
    }
}
