use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::AsterAssayMaterializationGate;
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_loom::{CrossTermKind, LoomStore, MaterializationAction};
use serde_json::json;

#[allow(dead_code)]
mod stage5_helpers;
use stage5_helpers::{assay_vault, complementary_pair_samples, slot};

type SampleSlots = Vec<(CxId, BTreeMap<SlotId, Vec<f32>>)>;
type SampleVault = (AsterVault, Vec<CxId>, SampleSlots);

#[test]
fn aster_gate_drives_live_materialization_policy() {
    let root = fsv_root().join(format!("unit-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let (vault, cx_ids, _) = write_sample_vault(&root.join("vault"), false, true, true);
    let gate = AsterAssayMaterializationGate::new(
        &vault,
        cx_ids.clone(),
        AnchorKind::Label("issue319-passfail".to_string()),
    );

    let gain_bits = gate.pair_gain(slot(1), slot(2)).unwrap().gain_bits;
    let plan = gate.materialization_plan(&[slot(1), slot(2)]).unwrap();

    assert!(gain_bits > 0.05);
    assert_eq!(
        plan_count(
            &plan,
            CrossTermKind::Interaction,
            MaterializationAction::EagerStore
        ),
        1
    );
    assert!(gate.last_error().is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn aster_gate_errors_are_observable_by_default() {
    let root = fsv_root().join(format!("error-default-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let (vault, cx_ids, _) = write_sample_vault(&root.join("vault"), false, false, true);
    let gate = AsterAssayMaterializationGate::new(
        &vault,
        cx_ids,
        AnchorKind::Label("issue319-passfail".to_string()),
    );

    let err = gate.materialization_plan(&[slot(1), slot(2)]).unwrap_err();

    assert_eq!(err.code, "CALYX_STALE_DERIVED");
    assert_eq!(gate.last_error().unwrap().code, "CALYX_STALE_DERIVED");
    assert_eq!(gate.pair_gain_bits_fail_safe_lazy(slot(1), slot(2)), 0.0);
    let fallback_plan = gate.materialization_plan_fail_safe_lazy(&[slot(1), slot(2)]);
    assert_eq!(
        plan_count(
            &fallback_plan,
            CrossTermKind::Agreement,
            MaterializationAction::EagerStore
        ),
        1
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "manual FSV writes Aster-backed Loom materialization readbacks"]
fn aster_materialization_gate_manual_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let _ = fs::remove_dir_all(root.join("source-vault"));
    let _ = fs::remove_dir_all(root.join("missing-anchor-vault"));
    let _ = fs::remove_dir_all(root.join("missing-slot-vault"));
    let _ = fs::remove_dir_all(root.join("xterm-live-cf"));
    let _ = fs::remove_dir_all(root.join("xterm-missing-anchor-cf"));

    let (vault, cx_ids, sample_slots) =
        write_sample_vault(&root.join("source-vault"), true, true, true);
    let source_counts = source_cf_counts(&root.join("source-vault"));
    let gate = AsterAssayMaterializationGate::new(
        &vault,
        cx_ids.clone(),
        AnchorKind::Label("issue319-passfail".to_string()),
    );
    let live_gain = gate.pair_gain(slot(1), slot(2)).unwrap().gain_bits;
    let live_plan = gate.materialization_plan(&[slot(1), slot(2)]).unwrap();
    let live_xterms = materialize_samples(&sample_slots, &live_plan);
    let live_cf = persist_and_reload(&root.join("xterm-live-cf"), &live_xterms);

    let (missing_anchor_vault, missing_anchor_ids, missing_anchor_slots) =
        write_sample_vault(&root.join("missing-anchor-vault"), true, false, true);
    let missing_anchor_source_counts = source_cf_counts(&root.join("missing-anchor-vault"));
    let missing_anchor_gate = AsterAssayMaterializationGate::new(
        &missing_anchor_vault,
        missing_anchor_ids,
        AnchorKind::Label("issue319-passfail".to_string()),
    );
    let missing_anchor_error = missing_anchor_gate
        .materialization_plan(&[slot(1), slot(2)])
        .unwrap_err();
    let missing_anchor_gain = missing_anchor_gate.pair_gain_bits_fail_safe_lazy(slot(1), slot(2));
    let missing_anchor_plan =
        missing_anchor_gate.materialization_plan_fail_safe_lazy(&[slot(1), slot(2)]);
    let missing_anchor_xterms = materialize_samples(&missing_anchor_slots, &missing_anchor_plan);
    let missing_anchor_cf = persist_and_reload(
        &root.join("xterm-missing-anchor-cf"),
        &missing_anchor_xterms,
    );

    let (missing_slot_vault, missing_slot_ids, _) =
        write_sample_vault(&root.join("missing-slot-vault"), true, true, false);
    let missing_slot_gate = AsterAssayMaterializationGate::new(
        &missing_slot_vault,
        missing_slot_ids,
        AnchorKind::Label("issue319-passfail".to_string()),
    );
    let missing_slot_error = missing_slot_gate
        .materialization_plan(&[slot(1), slot(2)])
        .unwrap_err();
    let missing_slot_gain = missing_slot_gate.pair_gain_bits_fail_safe_lazy(slot(1), slot(2));
    let missing_slot_plan =
        missing_slot_gate.materialization_plan_fail_safe_lazy(&[slot(1), slot(2)]);

    let readback = json!({
        "source_of_truth": "AsterVault slot/anchor CF rows feeding AsterAssayMaterializationGate and persisted Loom xterm CF rows",
        "anchor_kind": AnchorKind::Label("issue319-passfail".to_string()),
        "live": {
            "source_cf_counts": source_counts,
            "pair_gain_bits": live_gain,
            "plan_counts": plan_counts(&live_plan),
            "gate_last_error": gate.last_error().map(|error| error.code),
            "xterm_cf": live_cf,
        },
        "missing_anchor": {
            "source_cf_counts": missing_anchor_source_counts,
            "pair_gain_bits": missing_anchor_gain,
            "default_error": missing_anchor_error.code,
            "plan_counts": plan_counts(&missing_anchor_plan),
            "gate_last_error": missing_anchor_gate.last_error().map(|error| error.code),
            "xterm_cf": missing_anchor_cf,
        },
        "missing_slot": {
            "pair_gain_bits": missing_slot_gain,
            "default_error": missing_slot_error.code,
            "fallback_plan_counts": plan_counts(&missing_slot_plan),
            "gate_last_error": missing_slot_gate.last_error().map(|error| error.code),
        },
    });
    let path = root.join("aster-materialization-gate-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("ASTER_MATERIALIZATION_GATE_READBACK={}", path.display());
}

fn write_sample_vault(
    dir: &Path,
    durable: bool,
    include_anchor: bool,
    include_right_slot: bool,
) -> SampleVault {
    let vault = if durable {
        AsterVault::new_durable(
            dir,
            vault_id(),
            b"issue319-materialization-gate",
            VaultOptions::default(),
        )
        .unwrap()
    } else {
        AsterVault::new(vault_id(), b"issue319-materialization-gate")
    };
    let (left, right, labels) = complementary_pair_samples();
    let mut cx_ids = Vec::with_capacity(labels.len());
    let mut sample_slots = Vec::with_capacity(labels.len());
    for index in 0..labels.len() {
        let cx_id = CxId::from_bytes([index as u8; 16]);
        let mut slots = BTreeMap::from([(
            slot(1),
            SlotVector::Dense {
                dim: left[index].len() as u32,
                data: left[index].clone(),
            },
        )]);
        let mut materialization_slots = BTreeMap::from([(slot(1), left[index].clone())]);
        if include_right_slot {
            slots.insert(
                slot(2),
                SlotVector::Dense {
                    dim: right[index].len() as u32,
                    data: right[index].clone(),
                },
            );
            materialization_slots.insert(slot(2), right[index].clone());
        }
        let anchors = include_anchor
            .then(|| Anchor {
                kind: AnchorKind::Label("issue319-passfail".to_string()),
                value: AnchorValue::Bool(labels[index]),
                source: "issue319-grounded-synthetic".to_string(),
                observed_at: 1_785_400_000 + index as u64,
                confidence: 1.0,
            })
            .into_iter()
            .collect();
        vault.put(constellation(cx_id, slots, anchors)).unwrap();
        cx_ids.push(cx_id);
        sample_slots.push((cx_id, materialization_slots));
    }
    if durable {
        vault.flush().unwrap();
    }
    (vault, cx_ids, sample_slots)
}

fn constellation(
    cx_id: CxId,
    slots: BTreeMap<SlotId, SlotVector>,
    anchors: Vec<Anchor>,
) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 28,
        created_at: 1_785_400_000,
        input_ref: InputRef {
            hash: [cx_id.as_bytes()[0]; 32],
            pointer: Some(format!("synthetic://issue319/{}", cx_id.as_bytes()[0])),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors,
        provenance: LedgerRef {
            seq: 3_190 + cx_id.as_bytes()[0] as u64,
            hash: [3; 32],
        },
        flags: CxFlags::default(),
    }
}

fn materialize_samples(
    samples: &[(CxId, BTreeMap<SlotId, Vec<f32>>)],
    plan: &calyx_loom::MaterializationPlan,
) -> LoomStore {
    let mut store = LoomStore::new(64);
    for (cx_id, slots) in samples {
        store.materialize_plan(*cx_id, slots, plan).unwrap();
    }
    store
}

fn persist_and_reload(dir: &Path, store: &LoomStore) -> serde_json::Value {
    let mut router = CfRouter::open(dir, 1_048_576).unwrap();
    let persisted = store.persist_xterms_to_aster(&mut router).unwrap();
    let raw_cf_rows = router.iter_cf(ColumnFamily::XTerm).unwrap().len();
    let loaded = LoomStore::load_xterms_from_aster(&router, 64).unwrap();
    json!({
        "cf_root": dir.join("cf/xterm").display().to_string(),
        "persisted_rows": persisted,
        "raw_cf_rows": raw_cf_rows,
        "sst_files": router.level_file_count(ColumnFamily::XTerm),
        "kind_counts": kind_counts(&loaded),
        "agreement_edges": loaded.agreement_graph(),
    })
}

fn source_cf_counts(dir: &Path) -> serde_json::Value {
    let router = CfRouter::open(dir, 1_048_576).unwrap();
    json!({
        "base": router.iter_cf(ColumnFamily::Base).unwrap().len(),
        "slot_1": router.iter_cf(ColumnFamily::slot(slot(1))).unwrap().len(),
        "slot_2": router.iter_cf(ColumnFamily::slot(slot(2))).unwrap().len(),
        "anchors": router.iter_cf(ColumnFamily::Anchors).unwrap().len(),
    })
}

fn plan_counts(plan: &calyx_loom::MaterializationPlan) -> serde_json::Value {
    json!({
        "agreement_eager": plan_count(plan, CrossTermKind::Agreement, MaterializationAction::EagerStore),
        "interaction_eager": plan_count(plan, CrossTermKind::Interaction, MaterializationAction::EagerStore),
        "interaction_lazy": plan_count(plan, CrossTermKind::Interaction, MaterializationAction::LazyCache),
    })
}

fn kind_counts(store: &LoomStore) -> serde_json::Value {
    let mut counts = BTreeMap::<&'static str, usize>::new();
    for row in store.xterm_rows() {
        let key = match row.key.kind {
            CrossTermKind::Agreement => "agreement",
            CrossTermKind::Delta => "delta",
            CrossTermKind::Interaction => "interaction",
            CrossTermKind::Concat => "concat",
        };
        *counts.entry(key).or_default() += 1;
    }
    json!(counts)
}

fn plan_count(
    plan: &calyx_loom::MaterializationPlan,
    kind: CrossTermKind,
    action: MaterializationAction,
) -> usize {
    plan.entries
        .iter()
        .filter(|entry| entry.kind == kind && entry.action == action)
        .count()
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue319-materialization-fsv"))
}

fn vault_id() -> VaultId {
    assay_vault()
}
