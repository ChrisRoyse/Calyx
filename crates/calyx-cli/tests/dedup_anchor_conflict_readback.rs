use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::dedup::{
    DedupAction, DedupDecision, DedupPolicy, TauStrategy, TctCosineConfig, check_dedup,
    contested_with_key, decode_contested_with,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId, VaultStore,
};
use serde_json::{Value, json};

#[path = "support/dedup_fsv_io.rs"]
mod dedup_fsv_io;

use dedup_fsv_io::{
    fsv_root, list_dir_files as list_files, reset_dir, write_blake3_sums, write_json,
};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &str = "dedup-anchor-conflict-readback-salt";

#[test]
fn dedup_anchor_conflict_writes_contested_online_rows() {
    let (root, keep_root) = fsv_root("CALYX_DEDUP_ANCHOR_FSV_ROOT", "calyx-dedup-anchor-fsv");
    reset_dir(&root);
    let before = json!({
        "root_exists": root.exists(),
        "files": list_files(&root),
    });

    let speaker = speaker_conflict_scenario(&root);
    let missing_slot = missing_slot_order_scenario(&root);
    let style_conflict = style_conflict_scenario(&root);
    let style_compatible = style_compatible_scenario(&root);
    let exclusive_tag = exclusive_tag_scenario(&root);
    let no_shared = no_shared_anchor_scenario(&root);

    let readback = json!({
        "before": before,
        "speaker_conflict": speaker,
        "missing_slot_conflict_before_cosine": missing_slot,
        "style_conflict": style_conflict,
        "style_compatible": style_compatible,
        "exclusive_tag_conflict": exclusive_tag,
        "no_shared_anchor": no_shared,
        "after": {
            "files": list_files(&root),
        }
    });
    write_json(&root.join("dedup-anchor-conflict-readback.json"), &readback);
    write_blake3_sums(&root);

    assert_eq!(
        readback["speaker_conflict"]["decision"],
        json!({"AnchorConflict": {"existing": "11111111111111111111111111111111"}})
    );
    assert_eq!(
        readback["speaker_conflict"]["candidate_base_present"],
        json!(true)
    );
    assert_eq!(
        readback["missing_slot_conflict_before_cosine"]["decision"],
        json!({"AnchorConflict": {"existing": "31313131313131313131313131313131"}})
    );
    assert_close(
        readback["style_conflict"]["new_contested"]["reason"]["IncompatibleVector"]["cos"]
            .as_f64()
            .expect("style conflict cos"),
        0.65,
    );
    assert_eq!(
        readback["style_compatible"]["decision"]["Match"]["existing"],
        json!("51515151515151515151515151515151")
    );
    assert_eq!(
        readback["exclusive_tag_conflict"]["existing_contested"]["reason"],
        json!("ExclusiveTag")
    );
    assert_eq!(
        readback["no_shared_anchor"]["decision"]["Match"]["existing"],
        json!("71717171717171717171717171717171")
    );
    assert_eq!(
        readback["no_shared_anchor"]["candidate_contested"],
        Value::Null
    );

    println!("dedup_anchor_conflict_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn speaker_conflict_scenario(root: &Path) -> Value {
    let vault_dir = root.join("speaker_conflict").join("vault");
    let vault = durable_vault(&vault_dir);
    let existing = sample_cx(
        0x11,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![speaker("speaker-a")],
    );
    let candidate = sample_cx(
        0x22,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![speaker("speaker-b")],
    );
    vault.put(existing.clone()).expect("put existing");
    let decision = check_dedup(&candidate, &vault, &policy(), None).expect("dedup");
    vault
        .put(candidate.clone())
        .expect("put candidate separate");
    vault.flush().expect("flush vault");
    scenario_json(&vault, &vault_dir, decision, &existing, &candidate)
}

fn missing_slot_order_scenario(root: &Path) -> Value {
    let vault_dir = root.join("missing_slot").join("vault");
    let vault = durable_vault(&vault_dir);
    let existing = sample_cx(
        0x31,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![speaker("speaker-a")],
    );
    let candidate = sample_cx(0x32, [], vec![speaker("speaker-b")]);
    vault.put(existing.clone()).expect("put existing");
    let decision = check_dedup(&candidate, &vault, &policy(), None).expect("dedup");
    vault.flush().expect("flush vault");
    scenario_json(&vault, &vault_dir, decision, &existing, &candidate)
}

fn style_conflict_scenario(root: &Path) -> Value {
    let vault_dir = root.join("style_conflict").join("vault");
    let vault = durable_vault(&vault_dir);
    let existing = sample_cx(0x41, [(slot(0), dense(vec![1.0, 0.0]))], vec![style(1.0)]);
    let candidate = sample_cx(0x42, [(slot(0), dense(vec![1.0, 0.0]))], vec![style(0.65)]);
    vault.put(existing.clone()).expect("put existing");
    let decision = check_dedup(&candidate, &vault, &policy(), None).expect("dedup");
    vault.flush().expect("flush vault");
    scenario_json(&vault, &vault_dir, decision, &existing, &candidate)
}

fn style_compatible_scenario(root: &Path) -> Value {
    let vault_dir = root.join("style_compatible").join("vault");
    let vault = durable_vault(&vault_dir);
    let existing = sample_cx(0x51, [(slot(0), dense(vec![1.0, 0.0]))], vec![style(1.0)]);
    let candidate = sample_cx(0x52, [(slot(0), dense(vec![1.0, 0.0]))], vec![style(0.85)]);
    vault.put(existing.clone()).expect("put existing");
    let decision = check_dedup(&candidate, &vault, &policy(), None).expect("dedup");
    vault.flush().expect("flush vault");
    scenario_json(&vault, &vault_dir, decision, &existing, &candidate)
}

fn exclusive_tag_scenario(root: &Path) -> Value {
    let vault_dir = root.join("exclusive_tag").join("vault");
    let vault = durable_vault(&vault_dir);
    let kind = AnchorKind::Label("exclusive_tag".to_string());
    let existing = sample_cx(
        0x61,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![anchor(kind.clone(), AnchorValue::Text("tag-a".to_string()))],
    );
    let candidate = sample_cx(
        0x62,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![anchor(kind, AnchorValue::Text("tag-b".to_string()))],
    );
    vault.put(existing.clone()).expect("put existing");
    let decision = check_dedup(&candidate, &vault, &policy(), None).expect("dedup");
    vault.flush().expect("flush vault");
    scenario_json(&vault, &vault_dir, decision, &existing, &candidate)
}

fn no_shared_anchor_scenario(root: &Path) -> Value {
    let vault_dir = root.join("no_shared").join("vault");
    let vault = durable_vault(&vault_dir);
    let existing = sample_cx(
        0x71,
        [(slot(0), dense(vec![1.0, 0.0]))],
        vec![speaker("speaker-a")],
    );
    let candidate = sample_cx(0x72, [(slot(0), dense(vec![1.0, 0.0]))], vec![style(1.0)]);
    vault.put(existing.clone()).expect("put existing");
    let decision = check_dedup(&candidate, &vault, &policy(), None).expect("dedup");
    vault.flush().expect("flush vault");
    scenario_json(&vault, &vault_dir, decision, &existing, &candidate)
}

fn scenario_json(
    vault: &AsterVault,
    vault_dir: &Path,
    decision: DedupDecision,
    existing: &Constellation,
    candidate: &Constellation,
) -> Value {
    let snapshot = vault.snapshot();
    json!({
        "decision": decision,
        "existing": existing.cx_id,
        "candidate": candidate.cx_id,
        "existing_base_present": base_present(vault, existing.cx_id, snapshot),
        "candidate_base_present": base_present(vault, candidate.cx_id, snapshot),
        "existing_contested": contested(vault, existing.cx_id),
        "new_contested": contested(vault, candidate.cx_id),
        "candidate_contested": contested(vault, candidate.cx_id),
        "online_cf_stdout": stdout(&readback_online(vault_dir)),
    })
}

fn base_present(vault: &AsterVault, id: CxId, snapshot: u64) -> bool {
    vault
        .read_cf_at(snapshot, ColumnFamily::Base, &base_key(id))
        .expect("base read")
        .is_some()
}

fn contested(vault: &AsterVault, id: CxId) -> Option<Value> {
    let bytes = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Online,
            &contested_with_key(id),
        )
        .expect("contested read")?;
    let decoded = decode_contested_with(&bytes).expect("decode contested");
    Some(serde_json::to_value(decoded).expect("contested json"))
}

fn readback_online(vault_dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("readback")
        .arg("--cf")
        .arg("online")
        .arg("--vault")
        .arg(vault_dir)
        .output()
        .expect("run online readback")
}

fn durable_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![slot(0)],
            TauStrategy::PerSlot(vec![(slot(0), 0.90)]),
            DedupAction::Collapse,
        )
        .expect("policy"),
    )
}

fn sample_cx<const N: usize>(
    seed: u8,
    slots: [(SlotId, SlotVector); N],
    anchors: Vec<Anchor>,
) -> Constellation {
    Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: vault_id(),
        panel_version: 41,
        created_at: 1_786_406_600 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("synthetic://ph41/anchor-conflict/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: slots.into_iter().collect(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors,
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn speaker(name: &str) -> Anchor {
    anchor(
        AnchorKind::SpeakerMatch,
        AnchorValue::Text(name.to_string()),
    )
}

fn style(cos: f32) -> Anchor {
    anchor(
        AnchorKind::StyleHold,
        AnchorValue::Vector(vec![cos, (1.0 - cos * cos).sqrt()]),
    )
}

fn anchor(kind: AnchorKind, value: AnchorValue) -> Anchor {
    Anchor {
        kind,
        value,
        source: "synthetic-anchor-conflict".to_string(),
        observed_at: 1_786_406_600,
        confidence: 1.0,
    }
}

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid vault id")
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-5,
        "actual={actual} expected={expected}"
    );
}
