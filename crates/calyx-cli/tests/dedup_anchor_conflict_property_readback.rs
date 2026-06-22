use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::dedup::{
    AnchorConflictResult, DedupAction, DedupPolicy, EpochSecs, IngestInput, TauStrategy,
    TctCosineConfig, check_anchor_conflict, ingest_at,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId,
};
use serde_json::{Value, json};

#[allow(dead_code)]
#[path = "support/dedup_invariants_io.rs"]
mod io;

use io::{fsv_root, list_files, reset_dir, write_blake3_sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &str = "dedup-anchor-conflict-property-salt";
const PANEL_VERSION: u32 = 41;
const SPEAKERS: [&str; 4] = ["alice", "alice", "bob", "bob"];

#[test]
fn anchor_conflict_property_never_merges_forbidden_pairs_readback() {
    let (root, keep_root) = fsv_root(
        "CALYX_DEDUP_ANCHOR_PROPERTY_FSV_ROOT",
        "calyx-dedup-anchor-property-fsv",
    );
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let scenarios = vec![
        run_scenario(&root, "collapse-direct", DedupAction::Collapse),
        run_scenario(&root, "recurrence-series", DedupAction::RecurrenceSeries),
    ];
    let readback = json!({
        "before": before,
        "scenarios": scenarios,
        "after": {"files": list_files(&root)},
    });
    write_json(
        &root.join("dedup-anchor-conflict-property-readback.json"),
        &readback,
    );
    write_blake3_sums(&root);

    for scenario in readback["scenarios"].as_array().unwrap() {
        assert_eq!(scenario["before_counts"]["ledger_cf"], json!(0));
        assert_eq!(scenario["before_counts"]["wal"], json!(0));
        assert!(scenario["audit_block_pairs"].as_array().unwrap().len() >= 2);
        assert!(
            scenario["merge_groups"]
                .as_array()
                .unwrap()
                .iter()
                .any(|group| group["members"].as_array().unwrap().len() > 1)
        );
        assert!(
            scenario["forbidden_pair_violations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    assert!(
        readback["scenarios"][1]["after_counts"]["recurrence_cf"]
            .as_u64()
            .unwrap()
            > 0
    );

    println!("dedup_anchor_conflict_property_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn run_scenario(root: &Path, name: &str, action: DedupAction) -> Value {
    let vault_dir = root.join(name).join("vault");
    let vault = durable_vault(&vault_dir, action.clone());
    let raw_before = raw_readbacks(&vault_dir);
    write_raw_files(root, name, "before", &raw_before);

    let mut events = Vec::new();
    let mut results = Vec::new();
    for (index, speaker) in SPEAKERS.iter().enumerate() {
        let name = format!("{name}-{index}-{speaker}");
        let input = input(&name, speaker);
        let cx_id = vault.cx_id_for_input(name.as_bytes(), PANEL_VERSION);
        let result = ingest_at(&vault, &input, EpochSecs(1000 + index as i64 * 100), None)
            .expect("ingest anchor property event");
        events.push(CaseEvent::new(
            cx_id,
            &name,
            speaker,
            1000 + index as u64 * 100,
        ));
        results.push(json!({
            "index": index,
            "speaker": speaker,
            "cx_id": cx_id,
            "result": result,
        }));
    }
    vault.flush().expect("flush anchor property vault");

    let raw_after = raw_readbacks(&vault_dir);
    write_raw_files(root, name, "after", &raw_after);
    let cx_list = cx_list(&vault_dir);
    let audits = audits_for(&vault_dir, &cx_list);
    let groups = merge_groups(&audits);
    let forbidden_pairs = forbidden_pairs(&events);
    let violations = forbidden_pair_violations(&groups, &forbidden_pairs);
    let verify_chain = command_stdout(&[
        "verify-chain",
        "--vault",
        &display(&vault_dir),
        "--range",
        "0..4",
    ]);

    json!({
        "name": name,
        "action": action_name(&action),
        "events": results,
        "expected_forbidden_pairs": pair_values(&forbidden_pairs),
        "cx_list": cx_list,
        "audits": audits,
        "audit_block_pairs": audit_block_pairs(&audits),
        "merge_groups": group_values(&groups),
        "forbidden_pair_violations": violations,
        "before_counts": raw_counts(&raw_before),
        "after_counts": raw_counts(&raw_after),
        "raw_before": raw_before,
        "raw_after": raw_after,
        "verify_chain": verify_chain,
    })
}

#[derive(Clone)]
struct CaseEvent {
    id: CxId,
    name: String,
    speaker: String,
    created_at: u64,
}

impl CaseEvent {
    fn new(id: CxId, name: &str, speaker: &str, created_at: u64) -> Self {
        Self {
            id,
            name: name.to_string(),
            speaker: speaker.to_string(),
            created_at,
        }
    }

    fn cx(&self) -> Constellation {
        Constellation {
            cx_id: self.id,
            vault_id: vault_id(),
            panel_version: PANEL_VERSION,
            created_at: self.created_at,
            input_ref: InputRef {
                hash: [0; 32],
                pointer: Some(format!(
                    "synthetic://anchor-conflict-property/{}",
                    self.name
                )),
                redacted: true,
            },
            modality: Modality::Text,
            slots: BTreeMap::from([(
                slot(0),
                SlotVector::Dense {
                    dim: 2,
                    data: vec![1.0, 0.0],
                },
            )]),
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: vec![speaker(&self.speaker)],
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            flags: CxFlags {
                redacted_input: true,
                ..CxFlags::default()
            },
        }
    }
}

#[derive(Clone)]
struct ForbiddenPair {
    left: String,
    right: String,
    left_speaker: String,
    right_speaker: String,
}

struct MergeGroup {
    canonical: String,
    members: BTreeSet<String>,
}

fn forbidden_pairs(events: &[CaseEvent]) -> Vec<ForbiddenPair> {
    let mut pairs = Vec::new();
    for (left_index, left) in events.iter().enumerate() {
        for right in events.iter().skip(left_index + 1) {
            if matches!(
                check_anchor_conflict(&left.cx(), &right.cx()),
                AnchorConflictResult::Conflicting { .. }
            ) {
                pairs.push(ForbiddenPair {
                    left: left.id.to_string(),
                    right: right.id.to_string(),
                    left_speaker: left.speaker.clone(),
                    right_speaker: right.speaker.clone(),
                });
            }
        }
    }
    pairs
}

fn merge_groups(audits: &[Value]) -> Vec<MergeGroup> {
    audits
        .iter()
        .map(|audit| {
            let canonical = audit["cx_id"].as_str().unwrap().to_string();
            let mut members = BTreeSet::from([canonical.clone()]);
            for merge in audit["merges"].as_array().unwrap() {
                members.insert(merge["merged_from"].as_str().unwrap().to_string());
            }
            MergeGroup { canonical, members }
        })
        .collect()
}

fn forbidden_pair_violations(groups: &[MergeGroup], pairs: &[ForbiddenPair]) -> Value {
    let mut violations = Vec::new();
    for group in groups {
        for pair in pairs {
            if group.members.contains(&pair.left) && group.members.contains(&pair.right) {
                violations.push(json!({
                    "canonical": group.canonical,
                    "left": pair.left,
                    "right": pair.right,
                }));
            }
        }
    }
    json!(violations)
}

fn audits_for(vault_dir: &Path, cx_list: &Value) -> Vec<Value> {
    cx_list
        .as_array()
        .unwrap()
        .iter()
        .map(|row| dedup_audit(vault_dir, row["cx_id"].as_str().unwrap()))
        .collect()
}

fn audit_block_pairs(audits: &[Value]) -> Value {
    let mut pairs = Vec::new();
    for audit in audits {
        for blocked in audit["anchor_conflict_blocks"].as_array().unwrap() {
            pairs.push(json!({
                "cx_id": audit["cx_id"],
                "blocked": blocked,
            }));
        }
    }
    json!(pairs)
}

fn pair_values(pairs: &[ForbiddenPair]) -> Value {
    json!(
        pairs
            .iter()
            .map(|pair| json!({
                "left": pair.left,
                "right": pair.right,
                "left_speaker": pair.left_speaker,
                "right_speaker": pair.right_speaker,
            }))
            .collect::<Vec<_>>()
    )
}

fn group_values(groups: &[MergeGroup]) -> Value {
    json!(
        groups
            .iter()
            .map(|group| json!({
                "canonical": group.canonical,
                "members": group.members.iter().cloned().collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>()
    )
}

fn raw_readbacks(vault_dir: &Path) -> BTreeMap<String, String> {
    let mut readbacks = BTreeMap::new();
    for cf in ["base", "recurrence", "online", "ledger"] {
        readbacks.insert(format!("{cf}_cf"), raw_cf(vault_dir, cf));
    }
    readbacks.insert("wal".to_string(), raw_wal(vault_dir));
    readbacks
}

fn raw_counts(raw: &BTreeMap<String, String>) -> Value {
    let mut counts = BTreeMap::new();
    for (name, bytes) in raw {
        counts.insert(name.clone(), bytes.lines().count());
    }
    json!(counts)
}

fn write_raw_files(root: &Path, scenario: &str, phase: &str, raw: &BTreeMap<String, String>) {
    for (name, bytes) in raw {
        let path = root.join(format!("{scenario}-{phase}-{name}.tsv"));
        fs::write(path, bytes).expect("write raw readback");
    }
}

fn durable_vault(vault_dir: &Path, action: DedupAction) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions {
            dedup_policy: Some(policy(action)),
            ..VaultOptions::default()
        },
    )
    .expect("open durable vault")
}

fn policy(action: DedupAction) -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![slot(0)],
            TauStrategy::PerSlot(vec![(slot(0), 0.90)]),
            action,
        )
        .expect("policy"),
    )
}

fn input(name: &str, speaker_name: &str) -> IngestInput {
    IngestInput::new(name.as_bytes().to_vec(), PANEL_VERSION, Modality::Text)
        .with_slot(
            slot(0),
            SlotVector::Dense {
                dim: 2,
                data: vec![1.0, 0.0],
            },
        )
        .with_anchor(speaker(speaker_name))
}

fn speaker(name: &str) -> Anchor {
    Anchor {
        kind: AnchorKind::SpeakerMatch,
        value: AnchorValue::Text(name.to_string()),
        source: "synthetic-anchor-conflict-property".to_string(),
        observed_at: 1_786_406_600,
        confidence: 1.0,
    }
}

fn cx_list(vault_dir: &Path) -> Value {
    stdout_json(readback(&[
        "readback",
        "cx-list",
        "--vault",
        &display(vault_dir),
    ]))
}

fn dedup_audit(vault_dir: &Path, cx_id: &str) -> Value {
    stdout_json(readback(&[
        "readback",
        "dedup-audit",
        "--vault",
        &display(vault_dir),
        "--cx-id",
        cx_id,
    ]))
}

fn raw_cf(vault_dir: &Path, cf: &str) -> String {
    command_stdout(&["readback", "--cf", cf, "--vault", &display(vault_dir)])
}

fn raw_wal(vault_dir: &Path) -> String {
    command_stdout(&["readback", "--wal", "--vault", &display(vault_dir)])
}

fn readback(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args(args)
        .output()
        .expect("run calyx")
}

fn command_stdout(args: &[&str]) -> String {
    stdout(readback(args))
}

fn stdout_json(output: Output) -> Value {
    serde_json::from_str(&stdout(output)).expect("json stdout")
}

fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn action_name(action: &DedupAction) -> &'static str {
    match action {
        DedupAction::Collapse => "Collapse",
        DedupAction::Link => "Link",
        DedupAction::RecurrenceSeries => "RecurrenceSeries",
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid vault id")
}
