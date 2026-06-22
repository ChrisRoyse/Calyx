use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CxId, SlotId, content_address};
use calyx_loom::{
    CALYX_LOOM_DIM_MISMATCH, CALYX_LOOM_FORGE_UNAVAILABLE, CALYX_LOOM_NON_FINITE_VECTOR,
    CALYX_LOOM_SLOT_MISSING, CALYX_LOOM_ZERO_NORM_VECTOR, CrossTermKind, LoomStore,
    agreement_batch_gpu, agreement_scalar, agreement_weight, delta_vec, interaction_vec,
};
use serde_json::json;

#[test]
fn cross_terms_fail_closed_on_invalid_vectors() {
    assert!((agreement_scalar(&[1.0, 0.0], &[0.0, 1.0]).unwrap() - 0.0).abs() < 1.0e-6);
    assert_eq!(agreement_weight(-1.0).unwrap(), 0.0);
    assert_eq!(agreement_weight(0.75).unwrap(), 0.75);

    let zero = agreement_scalar(&[0.0, 0.0], &[1.0, 0.0]).unwrap_err();
    assert_eq!(zero.code, CALYX_LOOM_ZERO_NORM_VECTOR);
    let mismatch = delta_vec(&[1.0, 0.0], &[1.0]).unwrap_err();
    assert_eq!(mismatch.code, CALYX_LOOM_DIM_MISMATCH);
    let nonfinite = interaction_vec(&[f32::NAN], &[1.0]).unwrap_err();
    assert_eq!(nonfinite.code, CALYX_LOOM_NON_FINITE_VECTOR);
    assert_gpu_path();

    let mut store = LoomStore::new(4);
    let missing = store
        .cross_term(
            cx(1),
            slot(1),
            slot(9),
            CrossTermKind::Delta,
            &two_slot_map(vec![1.0, 0.0], vec![0.0, 1.0]),
        )
        .unwrap_err();
    assert_eq!(missing.code, CALYX_LOOM_SLOT_MISSING);
}

#[test]
#[ignore = "manual FSV writes Loom fail-closed source-of-truth artifacts"]
fn loom_cross_term_fail_closed_manual_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let cf_root = root.join("loom-xterm-cf");
    let _ = fs::remove_dir_all(&cf_root);
    let mut router = CfRouter::open(&cf_root, 1_048_576).unwrap();

    let mut store = LoomStore::new(8);
    let slots = two_slot_map(vec![1.0, 0.0], vec![0.5, 3.0_f32.sqrt() * 0.5]);
    let inserted = store.weave(cx(1), &slots).unwrap();
    let lazy = store
        .cross_term(cx(1), slot(1), slot(2), CrossTermKind::Delta, &slots)
        .unwrap();
    let persisted = store.persist_xterms_to_aster(&mut router).unwrap();
    let loaded = LoomStore::load_xterms_from_aster(&router, 8).unwrap();
    let edge = loaded.agreement_graph().pop().unwrap();

    let zero = agreement_scalar(&[0.0, 0.0], &[1.0, 0.0]).unwrap_err();
    let mismatch = delta_vec(&[1.0, 0.0], &[1.0]).unwrap_err();
    let nonfinite = interaction_vec(&[f32::INFINITY], &[1.0]).unwrap_err();
    let gpu = gpu_readback_code();
    let missing = store
        .cross_term(cx(1), slot(1), slot(9), CrossTermKind::Delta, &slots)
        .unwrap_err();

    let report = json!({
        "inserted_agreements": inserted,
        "persisted_xterms": persisted,
        "raw_cf_rows": router.iter_cf(ColumnFamily::XTerm).unwrap().len(),
        "lazy_delta": lazy,
        "edge": edge,
        "antipodal_weight": agreement_weight(-1.0).unwrap(),
        "errors": {
            "zero_norm": zero.code,
            "dim_mismatch": mismatch.code,
            "non_finite": nonfinite.code,
            "gpu": gpu,
            "missing_slot": missing.code,
        },
    });
    let path = root.join("loom-cross-term-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let digest = digest_hex(&bytes);

    println!("PH27_LOOM_FSV_ROOT={}", root.display());
    println!("PH27_LOOM_CROSS_TERM_REPORT={}", path.display());
    println!("PH27_LOOM_CROSS_TERM_REPORT_BLAKE3={digest}");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["inserted_agreements"], 1);
    assert_eq!(readback["persisted_xterms"], 1);
    assert_eq!(readback["raw_cf_rows"], 1);
    assert_eq!(readback["edge"]["raw_mean_agreement"], 0.5);
    assert_eq!(readback["edge"]["agreement_weight"], 0.5);
    assert_eq!(readback["antipodal_weight"], 0.0);
    assert_eq!(readback["errors"]["zero_norm"], CALYX_LOOM_ZERO_NORM_VECTOR);
    assert_eq!(readback["errors"]["dim_mismatch"], CALYX_LOOM_DIM_MISMATCH);
    assert_eq!(
        readback["errors"]["non_finite"],
        CALYX_LOOM_NON_FINITE_VECTOR
    );
    assert_eq!(readback["errors"]["gpu"], expected_gpu_readback());
    assert_eq!(readback["errors"]["missing_slot"], CALYX_LOOM_SLOT_MISSING);
}

fn two_slot_map(a: Vec<f32>, b: Vec<f32>) -> BTreeMap<SlotId, Vec<f32>> {
    BTreeMap::from([(slot(1), a), (slot(2), b)])
}

fn cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn assert_gpu_path() {
    let result = agreement_batch_gpu(&[(&[1.0, 0.0], &[0.0, 1.0])]);
    if cfg!(feature = "cuda") {
        assert!(result.unwrap()[0].abs() <= 1.0e-6);
    } else {
        assert_eq!(result.unwrap_err().code, CALYX_LOOM_FORGE_UNAVAILABLE);
    }
}

fn gpu_readback_code() -> &'static str {
    match agreement_batch_gpu(&[(&[1.0, 0.0], &[0.0, 1.0])]) {
        Ok(_) => "forge_cuda",
        Err(error) => error.code,
    }
}

fn expected_gpu_readback() -> &'static str {
    if cfg!(feature = "cuda") {
        "forge_cuda"
    } else {
        CALYX_LOOM_FORGE_UNAVAILABLE
    }
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-loom-cross-term-fsv"))
}

fn digest_hex(bytes: &[u8]) -> String {
    content_address([bytes])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
