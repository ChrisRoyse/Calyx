//! Regression for #1952: the exact pre-#1902 TEI drift, reproduced in-repo.
//!
//! Three law vaults were orphaned because a frozen contract hash input changed
//! with no typed migration signal. This suite pins the behaviour that makes
//! that class of failure legible and keeps the fail-closed guard intact.

use calyx_aster::manifest::ImmutableRef;
use calyx_core::{Asymmetry, Modality, QuantPolicy, SlotShape};

use crate::derivation::{self, DerivationImpact};
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::lens::DeterminismProof;
use crate::persistence::VaultRegistrySnapshot;
use crate::persistence_contracts::{
    audit_registry_snapshot_contracts, ensure_repair_is_honest, registry_contract_audit_error,
};
use crate::spec::default_recall_delta;
use crate::{LensRuntime, LensSpec, RegistryLensSnapshot};

/// The corpus_hash every TEI lens carried before #1902 (`19dcc189`).
///
/// `LD-SHA256("tei-http-runtime")`, reproduced here so the test fails if the
/// pre-#1902 digest is ever misremembered.
const PRE_1902_TEI_CORPUS_HASH: &str =
    "d419262604e2c2f2e915f8cb0bc9d154896b5b29d859e02bec3284873c5ad899";

const ENDPOINT: &str = "http://127.0.0.1:8088/embed";

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn pre_1902_corpus_hash_is_the_digest_recorded_in_issue_1952() {
    assert_eq!(
        hex32(&sha256_digest(&[b"tei-http-runtime"])),
        PRE_1902_TEI_CORPUS_HASH
    );
}

/// Builds the registry record a law vault persisted before #1902: a TEI lens
/// whose corpus_hash is the old policy digest, with no derivation stamp.
fn pre_1902_tei_snapshot(name: &str) -> RegistryLensSnapshot {
    let contract = FrozenLensContract::new(
        name,
        sha256_digest(&[ENDPOINT.as_bytes()]),
        sha256_digest(&[b"tei-http-runtime"]),
        SlotShape::Dense(768),
        Modality::Text,
        LensDType::F32,
        NormPolicy::unit(),
    );
    let spec = LensSpec {
        name: name.to_string(),
        runtime: LensRuntime::TeiHttp {
            endpoint: ENDPOINT.to_string(),
        },
        output: SlotShape::Dense(768),
        modality: Modality::Text,
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: NormPolicy::unit(),
        max_batch: Some(4),
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    RegistryLensSnapshot {
        lens_id: contract.lens_id(),
        contract,
        spec: Some(spec),
        determinism: DeterminismProof::ProbeVerified,
        runtime_golden: None,
        // The defining property of the orphaned vaults: no stamp.
        derivation_version: None,
    }
}

fn snapshot_of(lenses: Vec<RegistryLensSnapshot>) -> VaultRegistrySnapshot {
    VaultRegistrySnapshot {
        version: 1,
        panel_ref: ImmutableRef::from_bytes("panel.json", b"{}").expect("panel ref"),
        lenses,
    }
}

#[test]
fn pre_1902_tei_lens_still_fails_the_audit() {
    let audit = audit_registry_snapshot_contracts(&snapshot_of(vec![pre_1902_tei_snapshot(
        "semantic-gte-base-tei",
    )]));
    assert!(
        !audit.valid,
        "the fail-closed guard must not be weakened by derivation classification"
    );
    assert_eq!(audit.diffs.len(), 1);
}

#[test]
fn drift_is_attributed_to_the_tei_request_policy_step() {
    let audit = audit_registry_snapshot_contracts(&snapshot_of(vec![pre_1902_tei_snapshot(
        "semantic-gte-base-tei",
    )]));
    let diff = &audit.diffs[0];

    // The observed drift is exactly the two fields #1952 reconstructed.
    let mut moved: Vec<&str> = diff.fields.iter().map(|f| f.field.as_str()).collect();
    moved.sort_unstable();
    moved.dedup();
    assert!(
        moved
            .iter()
            .all(|f| f.ends_with("corpus_hash") || f.ends_with("lens_id")),
        "unexpected drifted fields: {moved:?}"
    );

    let explanation = diff
        .derivation_explanation
        .as_ref()
        .expect("the TEI request-policy change must explain this drift");
    assert_eq!(
        explanation.impact,
        DerivationImpact::MeasurementChanging.to_string()
    );
    assert!(
        !explanation.re_attestable,
        "truncated measurements must never be marked re-attestable"
    );
    assert!(
        explanation.deltas.iter().any(|d| d.contains("19dcc189")),
        "the delta must name the culprit commit: {:?}",
        explanation.deltas
    );
    assert_eq!(diff.persisted_derivation_version, None);
    assert_eq!(
        diff.binary_derivation_version,
        derivation::LENS_DERIVATION_VERSION
    );
}

#[test]
fn audit_error_is_the_typed_migration_code_not_an_opaque_drift() {
    let audit = audit_registry_snapshot_contracts(&snapshot_of(vec![
        pre_1902_tei_snapshot("semantic-gte-base-tei"),
        pre_1902_tei_snapshot("domain-modernbert-legal-tei"),
    ]));
    let error = registry_contract_audit_error(&audit);

    assert_eq!(error.code, derivation::LENS_DERIVATION_STALE);
    assert!(error.message.contains("truncate=true"), "{}", error.message);
    assert!(
        error.message.contains("MUST be re-measured"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains("semantic-gte-base-tei")
            && error.message.contains("domain-modernbert-legal-tei"),
        "both affected lenses must be named: {}",
        error.message
    );
    assert!(
        error.remediation.contains("re-ingest into a fresh vault"),
        "{}",
        error.remediation
    );
}

#[test]
fn repair_refuses_to_re_stamp_a_measurement_changing_drift() {
    let lens = pre_1902_tei_snapshot("semantic-gte-base-tei");
    let error = ensure_repair_is_honest(&lens)
        .expect_err("re-stamping truncated vectors as no-truncate measurements is dishonest");
    assert_eq!(error.code, derivation::LENS_DERIVATION_STALE);
    assert!(
        error.message.contains("refusing to re-stamp"),
        "{}",
        error.message
    );
}

#[test]
fn an_unexplained_field_keeps_the_tamper_suspect_drift_code() {
    // Same stale derivation, but the weights hash also moved. No recorded step
    // can move `weights_sha256`, so this must NOT be laundered into a
    // migration notice.
    let mut lens = pre_1902_tei_snapshot("semantic-gte-base-tei");
    let spec = lens.spec.as_mut().expect("spec");
    spec.weights_sha256[0] ^= 0xff;

    let audit = audit_registry_snapshot_contracts(&snapshot_of(vec![lens]));
    assert!(!audit.valid);
    assert!(
        audit.diffs[0].derivation_explanation.is_none(),
        "a weights_sha256 drift is not explained by any derivation step"
    );
    assert_eq!(
        registry_contract_audit_error(&audit).code,
        "CALYX_REGISTRY_CONTRACT_DRIFT"
    );
}

#[test]
fn a_currently_stamped_clean_lens_audits_valid() {
    let contract =
        FrozenLensContract::tei_http("semantic-gte-base-tei", ENDPOINT, Modality::Text, 768);
    let mut lens = pre_1902_tei_snapshot("semantic-gte-base-tei");
    let spec = lens.spec.as_mut().expect("spec");
    spec.corpus_hash = contract.corpus_hash();
    lens.contract = contract.clone();
    lens.lens_id = contract.lens_id();
    lens.derivation_version = Some(derivation::LENS_DERIVATION_VERSION);

    let audit = audit_registry_snapshot_contracts(&snapshot_of(vec![lens]));
    assert!(audit.valid, "unexpected diffs: {:?}", audit.diffs);
}
