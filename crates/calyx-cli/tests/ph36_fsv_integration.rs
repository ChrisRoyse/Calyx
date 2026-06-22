mod support;

use calyx_ledger::{VerifyResult, assert_within_tolerance, verify_chain};
use serde_json::json;
use support::{
    broken_at, cx, fsv_root, hit, memory_chain, mutate_row, mutate_row_from_end, reset_dir,
    run_reproduce_fsv, run_tamper_fsv,
};

#[test]
fn tolerance_boundary_is_locked() {
    let original = vec![hit(cx(1), 0.5)];
    let inside = vec![hit(cx(1), 0.5009)];
    let outside = vec![hit(cx(1), 0.5011)];

    let (inside_ok, inside_drift) = assert_within_tolerance(&original, &inside, 1.0e-3);
    let (outside_ok, outside_drift) = assert_within_tolerance(&original, &outside, 1.0e-3);

    assert!(inside_ok);
    assert!(inside_drift <= 1.0e-3);
    assert!(!outside_ok);
    assert!(outside_drift > 1.0e-3);
}

#[test]
fn verify_chain_edges_cover_intact_seq0_and_entry_hash_flip() {
    let intact = memory_chain(20);
    assert_eq!(
        verify_chain(&intact, 0..20).unwrap(),
        VerifyResult::Intact { count: 20 }
    );

    let mut seq0 = memory_chain(20);
    mutate_row(&mut seq0, 0, 8);
    assert_eq!(broken_at(&seq0, 0..20), 0);

    let mut hash_flip = memory_chain(20);
    mutate_row_from_end(&mut hash_flip, 11, 32);
    assert_eq!(broken_at(&hash_flip, 0..20), 11);
}

#[test]
#[ignore = "manual FSV for PH36 exit integration"]
fn ph36_fsv_integration_manual() {
    let root = fsv_root().join("ph36-exit-fsv");
    reset_dir(&root);

    let tamper = run_tamper_fsv(&root);
    let reproduce = run_reproduce_fsv(&root);
    let max_drift = reproduce["result"]["max_drift"].as_f64().unwrap();
    let readback = json!({
        "tamper": tamper,
        "reproduce": reproduce,
        "summary": format!(
            "PH36 FSV PASS: tamper detected at seq=11; reproduce max_drift={max_drift:.6}"
        ),
    });
    let path = root.join("ph36-fsv-integration-readback.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();

    println!("PH36_FSV_ROOT={}", root.display());
    println!("PH36_FSV_READBACK={}", path.display());
    println!("{}", readback["summary"].as_str().unwrap());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["tamper"]["broken_seq"], 11);
    assert_eq!(readback["tamper"]["seq_11_quarantined"], true);
    assert_eq!(readback["reproduce"]["result"]["reproduced"], true);
    assert!(max_drift <= 1.0e-3);
}
