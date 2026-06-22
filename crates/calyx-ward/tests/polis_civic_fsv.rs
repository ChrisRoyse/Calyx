use std::fs;
use std::path::{Path, PathBuf};

use calyx_ward::{
    CALYX_POLIS_EMPTY_PERSONA_SET, CALYX_POLIS_INVALID_AXIS, CALYX_POLIS_SLOT_COUNT_MISMATCH,
    CALYX_POLIS_TIE_MISMATCH, CIVIC_SLOT_COUNT, CivicPersonaPair, PolisCivicError,
    evaluate_polis_civic_pairs, synthetic_polis_persona_pairs,
};
use serde_json::{Value, json};

#[test]
fn issue611_polis_civic_guard_matches_planted_ties() {
    let proof = evaluate_polis_civic_pairs(&synthetic_polis_persona_pairs()).expect("proof");

    assert_eq!(proof.civic_slot_count, CIVIC_SLOT_COUNT);
    assert_eq!(proof.required_slots.len(), CIVIC_SLOT_COUNT);
    assert_eq!(proof.temporal_slots_excluded, vec![22, 23, 24]);
    assert!(proof.all_expected_outcomes_match);
    assert_eq!(proof.pairs.len(), 4);
    assert_eq!(proof.pairs.iter().filter(|pair| pair.actual_tie).count(), 2);
    assert!(proof.pairs.iter().all(|pair| pair.tie_outcome_matches));
    assert!(
        proof
            .pairs
            .iter()
            .filter(|pair| !pair.actual_tie)
            .all(|pair| !pair.failing_slots.is_empty())
    );
    assert_eq!(
        proof
            .pairs
            .iter()
            .find(|pair| pair.pair_id == "reject-single-axis-07")
            .unwrap()
            .failing_slots,
        vec![7]
    );
}

#[test]
fn issue611_edges_fail_closed_with_codes() {
    let cases = [
        (empty_pairs(), CALYX_POLIS_EMPTY_PERSONA_SET),
        (slot_count_mismatch(), CALYX_POLIS_SLOT_COUNT_MISMATCH),
        (invalid_zero_axis(), CALYX_POLIS_INVALID_AXIS),
        (tie_mismatch(), CALYX_POLIS_TIE_MISMATCH),
    ];

    for (pairs, code) in cases {
        let error = evaluate_polis_civic_pairs(&pairs).expect_err("edge error");
        assert_eq!(error.code(), code);
        assert!(error.to_string().contains(code));
    }
}

#[test]
#[ignore = "manual FSV for issue #611 Polis civic synthetic personas"]
fn issue611_polis_civic_guard_fsv_writes_readbacks() {
    let root = fsv_root();
    assert!(
        !root.exists(),
        "choose a fresh CALYX_ISSUE611_FSV_ROOT; already exists: {}",
        root.display()
    );
    fs::create_dir_all(&root).expect("create root");
    let pairs = synthetic_polis_persona_pairs();
    let personas_path = root.join("synthetic-persona-pairs.json");
    let proof_path = root.join("polis-civic-proof.json");

    write_json(&personas_path, &pairs);
    let before = file_state(&proof_path);
    let proof = evaluate_polis_civic_pairs(&pairs).expect("issue611 proof");
    write_json(&proof_path, &proof);
    let after = file_state(&proof_path);

    let edge_root = root.join("edges");
    fs::create_dir_all(&edge_root).expect("create edge root");
    let readback = json!({
        "issue": 611,
        "trigger": "calyx_ward::polis evaluate_polis_civic_pairs over deterministic 21-slot synthetic personas",
        "source_of_truth": {
            "root": display(&root),
            "personas": display(&personas_path),
            "proof": display(&proof_path),
        },
        "known_io": {
            "civic_slots": CIVIC_SLOT_COUNT,
            "required_slots": proof.required_slots,
            "tau": proof.tau,
            "expected_pairs": [
                {"pair_id": "tie-alpha-beta", "tie": true, "failing_slots": []},
                {"pair_id": "tie-gamma-delta", "tie": true, "failing_slots": []},
                {"pair_id": "reject-single-axis-07", "tie": false, "failing_slots": [7]},
                {"pair_id": "reject-majority-shift", "tie": false, "failing_slots": [1,2,3,4,5,6,7,8,9,10,11]},
            ],
        },
        "happy": {
            "before": before,
            "after": after,
            "artifact": proof,
        },
        "edges": {
            "empty_pairs": run_edge(&edge_root, "empty_pairs", empty_pairs()),
            "slot_count_mismatch": run_edge(&edge_root, "slot_count_mismatch", slot_count_mismatch()),
            "invalid_zero_axis": run_edge(&edge_root, "invalid_zero_axis", invalid_zero_axis()),
            "tie_mismatch": run_edge(&edge_root, "tie_mismatch", tie_mismatch()),
        }
    });
    let readback_path = root.join("issue611-fsv-readback.json");
    write_json(&readback_path, &readback);
    let manifest = write_blake3_manifest(&root);

    assert_eq!(readback["happy"]["artifact"]["civic_slot_count"], json!(21));
    assert_eq!(
        readback["happy"]["artifact"]["all_expected_outcomes_match"],
        json!(true)
    );
    assert_eq!(
        readback["edges"]["empty_pairs"]["after"]["exists"],
        json!(false)
    );

    println!("ISSUE611_FSV_ROOT={}", root.display());
    println!("ISSUE611_READBACK={}", readback_path.display());
    println!("ISSUE611_BLAKE3={}", manifest.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
}

fn empty_pairs() -> Vec<CivicPersonaPair> {
    Vec::new()
}

fn slot_count_mismatch() -> Vec<CivicPersonaPair> {
    let mut pairs = synthetic_polis_persona_pairs();
    pairs[0].right.axes.pop();
    pairs
}

fn invalid_zero_axis() -> Vec<CivicPersonaPair> {
    let mut pairs = synthetic_polis_persona_pairs();
    pairs[0].left.axes[3] = 0.0;
    pairs
}

fn tie_mismatch() -> Vec<CivicPersonaPair> {
    let mut pairs = synthetic_polis_persona_pairs();
    pairs[2].planted_tie = true;
    pairs
}

fn run_edge(root: &Path, label: &str, pairs: Vec<CivicPersonaPair>) -> Value {
    let case_root = root.join(label);
    fs::create_dir_all(&case_root).expect("create edge");
    let out = case_root.join("polis-civic-proof.json");
    let pairs_path = case_root.join("synthetic-persona-pairs.json");
    write_json(&pairs_path, &pairs);
    let before = file_state(&out);
    let result = evaluate_polis_civic_pairs(&pairs);
    let after = file_state(&out);
    match result {
        Ok(_) => json!({
            "before": before,
            "after": after,
            "success": true,
            "error_code": Value::Null,
            "message": Value::Null,
        }),
        Err(error) => edge_error(before, after, error),
    }
}

fn edge_error(before: Value, after: Value, error: PolisCivicError) -> Value {
    json!({
        "before": before,
        "after": after,
        "success": false,
        "error_code": error.code(),
        "message": error.to_string(),
    })
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).expect("write json")
}

fn file_state(path: &Path) -> Value {
    if !path.exists() {
        return json!({"path": display(path), "exists": false});
    }
    let bytes = fs::read(path).expect("read file");
    json!({
        "path": display(path),
        "exists": true,
        "len": bytes.len(),
        "blake3": blake3::hash(&bytes).to_string(),
        "hex_prefix": bytes.iter().take(64).map(|byte| format!("{byte:02x}")).collect::<String>(),
    })
}

fn write_blake3_manifest(root: &Path) -> PathBuf {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    let mut manifest = String::new();
    for path in files {
        let bytes = fs::read(&path).expect("read manifest input");
        let relative = path
            .strip_prefix(root)
            .expect("relative")
            .display()
            .to_string()
            .replace('\\', "/");
        manifest.push_str(&format!("{}  {relative}\n", blake3::hash(&bytes)));
    }
    let path = root.join("BLAKE3SUMS.txt");
    fs::write(&path, manifest).expect("write manifest");
    path
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(current).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else if path != root.join("BLAKE3SUMS.txt") {
            files.push(path);
        }
    }
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE611_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_ISSUE611_FSV_ROOT to a fresh manual verification path")
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
