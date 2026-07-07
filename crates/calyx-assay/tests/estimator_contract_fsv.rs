//! Full-state verification for the shared estimator contract (#71).
//!
//! Source of truth: a real Aster Assay column-family roundtrip plus a JSON
//! evidence artifact. The corpus is intentionally tiny: one trusted known-good
//! estimate row proves the persisted contract shape, and four focused edge
//! probes prove fail-closed behavior around CI, reliability, redundancy, and
//! unscoped persistence.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{
    AssayCacheKey, AssayStore, AssaySubject, CALYX_ASSAY_UNRESOLVED, CorrelationEvidence,
    EstimateReliability, EstimatorKind, MiEstimate, TrustTag, admit_lens_estimate,
};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{AnchorKind, SlotId, VaultId};
use serde_json::{Value, json};

#[test]
fn estimator_contract_fsv_persists_contract_and_edges() {
    let root = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let report_path = root.join("estimator_contract_fsv_report.json");
    let cf_dir = root.join("assay-cf");
    let mut router = CfRouter::open(&cf_dir, 1024).unwrap();

    let cache_key = AssayCacheKey::scoped(
        71,
        "issue71-known-truth",
        vault_id(),
        AnchorKind::Label("resolved_outcome".to_string()),
    );
    let subject = AssaySubject::Lens {
        slot: SlotId::new(71),
    };
    let reliability = EstimateReliability::new(7, 0.01, false).unwrap();
    let estimate = MiEstimate::new(
        0.125,
        0.09,
        0.16,
        128,
        EstimatorKind::Ksg,
        TrustTag::Trusted,
    )
    .with_reliability(reliability.clone());
    let corr = CorrelationEvidence::new(0.20, 0.10, 0.30).unwrap();
    let admission = admit_lens_estimate(&estimate, corr).unwrap();
    assert!(admission.admitted);

    let payload = json!({
        "schema_version": "poly.estimator_contract_fsv.v1",
        "proof_claim": "MiEstimate carries bits/CI/trust/reliability and persists with provenance",
        "selected_corpus": "one trusted estimate row plus four fail-closed edge probes",
        "why_smaller_is_insufficient": "without the persisted row or any edge probe the contract/provenance/fail-closed claim is unproven",
        "why_larger_is_wasteful": "contract shape and readback invariants are independent of large corpus scale",
    });
    let mut store = AssayStore::default();
    store.put_with_payload(
        cache_key.clone(),
        subject.clone(),
        estimate.clone(),
        "issue71:trusted-known-truth:ksg:v1",
        71_001,
        payload.clone(),
    );
    assert_eq!(store.persist_to_aster(&mut router).unwrap(), 1);
    drop(router);

    let reopened = CfRouter::open(&cf_dir, 1024).unwrap();
    let loaded = AssayStore::load_from_aster(&reopened).unwrap();
    let row = loaded.get(&cache_key, &subject).unwrap();
    assert_eq!(row.estimate.bits, 0.125);
    assert_eq!(row.estimate.ci_low, 0.09);
    assert_eq!(row.estimate.ci_high, 0.16);
    assert_eq!(row.estimate.trust, TrustTag::Trusted);
    assert_eq!(row.estimate.reliability.as_ref(), Some(&reliability));
    assert_eq!(row.provenance, "issue71:trusted-known-truth:ksg:v1");
    assert_eq!(row.payload.as_ref(), Some(&payload));

    let edges = edge_probe_codes();
    let report = json!({
        "schema_version": "poly.estimator_contract_fsv.v1",
        "source_of_truth": {
            "assay_cf_dir": cf_dir.to_string_lossy(),
            "report_path": report_path.to_string_lossy(),
            "before_report": file_state(&report_path),
        },
        "happy_path": {
            "persisted_rows": 1,
            "loaded_rows": loaded.len(),
            "admitted": admission.admitted,
            "signal_bits": admission.signal_bits,
            "max_pairwise_corr": admission.max_pairwise_corr,
            "row": row,
        },
        "edge_probe_codes": edges,
    });
    let bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(&report_path, &bytes).unwrap();

    let readback: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback["happy_path"]["loaded_rows"], 1);
    assert_eq!(
        readback["happy_path"]["row"]["provenance"],
        "issue71:trusted-known-truth:ksg:v1"
    );
    assert_eq!(
        readback["happy_path"]["row"]["estimate"]["trust"],
        "trusted"
    );
    assert_eq!(
        readback["edge_probe_codes"]["unresolved_ci"],
        CALYX_ASSAY_UNRESOLVED
    );
    assert_eq!(
        readback["edge_probe_codes"]["redundant_corr"],
        "CALYX_ASSAY_REDUNDANT"
    );
    assert_eq!(
        readback["edge_probe_codes"]["weak_reliability"],
        CALYX_ASSAY_UNRESOLVED
    );
    assert_eq!(
        readback["edge_probe_codes"]["unscoped_persistence"],
        "CALYX_VAULT_ACCESS_DENIED"
    );

    println!(
        "ESTIMATOR_CONTRACT_FSV report={} blake3={} loaded_rows=1 edge_codes={}",
        report_path.display(),
        blake3::hash(&fs::read(&report_path).unwrap()),
        readback["edge_probe_codes"]
    );
}

fn edge_probe_codes() -> Value {
    let good_reliability = EstimateReliability::new(7, 0.01, false).unwrap();

    let unresolved_ci =
        MiEstimate::new(0.08, 0.04, 0.12, 128, EstimatorKind::Ksg, TrustTag::Trusted)
            .with_reliability(good_reliability.clone());
    let unresolved_ci_code = admit_lens_estimate(
        &unresolved_ci,
        CorrelationEvidence::new(0.20, 0.10, 0.30).unwrap(),
    )
    .unwrap_err()
    .code;

    let redundant = MiEstimate::new(0.14, 0.10, 0.18, 128, EstimatorKind::Ksg, TrustTag::Trusted)
        .with_reliability(good_reliability);
    let redundant_corr_code = admit_lens_estimate(
        &redundant,
        CorrelationEvidence::new(0.75, 0.65, 0.85).unwrap(),
    )
    .unwrap_err()
    .code;

    let weak_reliability =
        MiEstimate::new(0.14, 0.10, 0.18, 128, EstimatorKind::Ksg, TrustTag::Trusted)
            .with_reliability(EstimateReliability::new(2, 0.01, false).unwrap());
    let weak_reliability_code = admit_lens_estimate(
        &weak_reliability,
        CorrelationEvidence::new(0.20, 0.10, 0.30).unwrap(),
    )
    .unwrap_err()
    .code;

    let unscoped_code = unscoped_persistence_error();

    json!({
        "unresolved_ci": unresolved_ci_code,
        "redundant_corr": redundant_corr_code,
        "weak_reliability": weak_reliability_code,
        "unscoped_persistence": unscoped_code,
    })
}

fn unscoped_persistence_error() -> String {
    let dir = fsv_root().join("unscoped-cf");
    fs::create_dir_all(&dir).unwrap();
    let mut router = CfRouter::open(&dir, 1024).unwrap();
    let mut store = AssayStore::default();
    #[allow(deprecated)]
    let key = AssayCacheKey::new(71, "legacy-unscoped");
    store.put(
        key,
        AssaySubject::Panel,
        MiEstimate::new(0.14, 0.10, 0.18, 128, EstimatorKind::Ksg, TrustTag::Trusted),
        "unscoped row must fail",
        71_002,
    );
    let code = store.persist_to_aster(&mut router).unwrap_err().code;
    assert_eq!(router.iter_cf(ColumnFamily::Assay).unwrap().len(), 0);
    code.to_string()
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE071_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "calyx_estimator_contract_fsv_{}",
                std::process::id()
            ))
        })
}

fn file_state(path: &Path) -> Value {
    match fs::read(path) {
        Ok(bytes) => json!({
            "exists": true,
            "len": bytes.len(),
            "blake3": blake3::hash(&bytes).to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({"exists": false}),
        Err(e) => json!({"exists": false, "read_error": e.to_string()}),
    }
}

fn vault_id() -> VaultId {
    "01HZY3ZJ8QK8H7D9W3V6B5N4M2".parse().unwrap()
}
