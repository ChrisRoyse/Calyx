use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{
    AnchorKind, Asymmetry, Input, Lens, Modality, QuantPolicy, Slot, SlotId, SlotKey, SlotShape,
    SlotState, SlotVector, VaultId,
};
use calyx_registry::{
    CapabilityGateDecision, CapabilityGateThresholds, CapabilitySignalKind, CommissionRequest,
    LensRuntime, LensSpec, NormPolicy, ProfileProbe, Registry, StaticLookupFileSpec,
    StaticLookupLens, commission_lens, evaluate_capability_gate, profile_lens,
    profile_slot_with_assay, register_commissioned, signal_kind_from_spec,
};
use serde_json::json;
use sha2::{Digest, Sha256};

const MATRIX_MAGIC: &[u8; 8] = b"CXLKUP1\0";
const DTYPE_I8: u8 = 1;

#[test]
fn issue936_commissioned_deterministic_lens_is_not_learned_encoder() {
    let root = temp_root("issue936-commissioned-signal");
    fs::create_dir_all(&root).expect("create FSV root");

    let mut registry = Registry::new();
    let commissioned_dir = root.join("commissioned");
    let request = CommissionRequest {
        name: "issue936-commissioned-esm-name".to_string(),
        base_model: "facebook/esm2_t6_8M_UR50D".to_string(),
        corpus: vec![
            b"alpha commissioned row".to_vec(),
            b"beta commissioned row".to_vec(),
            b"gamma commissioned row".to_vec(),
        ],
        output_dim: 8,
        modality: Modality::Text,
        axis: Some("issue936-deterministic-axis".to_string()),
    };
    let before_artifact_exists = commissioned_dir.exists();
    let artifact = commission_lens(&request, &commissioned_dir).expect("commission lens");
    let commissioned_spec_path = root.join("commissioned-spec.json");
    write_json(&commissioned_spec_path, &artifact.spec);
    let registered_commissioned =
        register_commissioned(&mut registry, artifact.clone()).expect("register commissioned");

    let probes = profile_probes();
    let card = profile_lens(&registry, registered_commissioned, &probes)
        .expect("profile commissioned lens");
    assert_eq!(
        card.signal_kind,
        CapabilitySignalKind::DeterministicContentFeature
    );
    let card_path = root.join("commissioned-card.json");
    write_json(&card_path, &card);
    let card_readback: calyx_registry::CapabilityCard =
        read_json(&card_path).expect("read commissioned card");
    assert_eq!(
        card_readback.signal_kind,
        CapabilitySignalKind::DeterministicContentFeature
    );

    let slot = slot_for_lens(registered_commissioned, "commissioned");
    let cache_key = assay_key();
    let mut assay = AssayStore::default();
    assay.put(
        cache_key.clone(),
        AssaySubject::Lens { slot: slot.slot_id },
        MiEstimate::point(0.42, 96, EstimatorKind::Ksg, TrustTag::Trusted),
        "issue936 grounded assay row",
        42,
    );
    let assay_dir = root.join("assay-cf");
    let mut router = CfRouter::open(&assay_dir, 1024).expect("open assay CF");
    let assay_before_rows = router
        .iter_cf(ColumnFamily::Assay)
        .expect("read before assay CF")
        .len();
    assay
        .persist_to_aster(&mut router)
        .expect("persist assay rows");
    let assay_after_rows = router
        .iter_cf(ColumnFamily::Assay)
        .expect("read after assay CF")
        .len();
    drop(router);
    assert_eq!(assay_before_rows, 0);
    assert_eq!(assay_after_rows, 1);

    let reopened = CfRouter::open(&assay_dir, 1024).expect("reopen assay CF");
    let loaded_assay = AssayStore::load_from_aster(&reopened).expect("load assay rows");
    let grounded_card =
        profile_slot_with_assay(&registry, &slot, &probes, &loaded_assay, &cache_key)
            .expect("profile with assay");
    assert_eq!(
        grounded_card.signal_kind,
        CapabilitySignalKind::DeterministicContentFeature
    );
    assert_eq!(grounded_card.signal, Some(0.42));
    let gate = evaluate_capability_gate(
        grounded_card.clone(),
        0.0,
        CapabilityGateThresholds::default(),
    )
    .expect("evaluate capability gate");
    assert_eq!(gate.decision, CapabilityGateDecision::Park);
    assert!(gate.reason.contains("deterministic_content_feature"));

    let static_runtime = register_static_lookup(&mut registry, &root);
    let static_card =
        profile_lens(&registry, static_runtime.lens_id, &probes).expect("profile static lookup");
    assert_eq!(
        static_card.signal_kind,
        CapabilitySignalKind::LearnedEncoder
    );
    assert_ne!(static_runtime.weights_sha256, [0_u8; 32]);
    assert!(static_runtime.embeddings_file.is_file());
    assert!(static_runtime.tokenizer.is_file());
    assert_eq!(static_runtime.row_count, 4);
    assert_eq!(static_runtime.vector, vec![0.6_f32, 0.8_f32, 0.0]);

    let empty_probe_error = profile_lens(&registry, registered_commissioned, &[])
        .expect_err("empty probes fail closed");
    let missing_hash_spec = LensSpec {
        weights_sha256: [0; 32],
        ..static_runtime.spec.clone()
    };
    let missing_hash_signal_kind = signal_kind_from_spec(&missing_hash_spec);
    let missing_path_spec = LensSpec {
        runtime: LensRuntime::StaticLookup {
            embeddings_file: PathBuf::new(),
            tokenizer: static_runtime.tokenizer.clone(),
            dim: 3,
        },
        ..static_runtime.spec.clone()
    };
    let missing_path_signal_kind = signal_kind_from_spec(&missing_path_spec);
    assert_eq!(empty_probe_error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(missing_hash_signal_kind, CapabilitySignalKind::Unknown);
    assert_eq!(missing_path_signal_kind, CapabilitySignalKind::Unknown);

    let readback = json!({
        "source_of_truth": {
            "commissioned_artifact_path": artifact.artifact_path,
            "commissioned_spec_path": commissioned_spec_path,
            "commissioned_card_path": card_path,
            "assay_cf_dir": assay_dir,
            "static_embeddings_file": static_runtime.embeddings_file,
            "static_tokenizer": static_runtime.tokenizer,
        },
        "before": {
            "commissioned_artifact_dir_exists": before_artifact_exists,
            "assay_rows": assay_before_rows,
        },
        "after": {
            "commissioned_artifact_file_exists": artifact.artifact_path.is_file(),
            "commissioned_artifact_sha256": file_digest(&artifact.artifact_path),
            "commissioned_spec_signal_kind": card_readback.signal_kind.as_str(),
            "commissioned_runtime": artifact.spec.runtime,
            "assay_rows": assay_after_rows,
            "grounded_signal": grounded_card.signal,
            "gate_decision": gate.decision,
            "gate_reason": gate.reason,
            "static_signal_kind": static_card.signal_kind.as_str(),
            "static_weights_sha256": hex32(&static_runtime.weights_sha256),
            "static_known_vector": static_runtime.vector,
        },
        "edge_cases": [
            {
                "name": "empty_probe_set",
                "before": {
                    "registered_commissioned_lens": registered_commissioned,
                    "probe_count": 0,
                    "card_file_exists": false
                },
                "after": {
                    "error_code": empty_probe_error.code,
                    "error_message": empty_probe_error.message,
                    "profile_card_created": false
                }
            },
            {
                "name": "missing_artifact_hash",
                "before": {
                    "runtime": missing_hash_spec.runtime,
                    "weights_sha256": hex32(&missing_hash_spec.weights_sha256),
                    "corpus_hash": hex32(&missing_hash_spec.corpus_hash)
                },
                "after": {
                    "signal_kind": missing_hash_signal_kind.as_str(),
                    "learned_encoder": missing_hash_signal_kind.is_learned_encoder()
                }
            },
            {
                "name": "missing_artifact_path",
                "before": {
                    "runtime": missing_path_spec.runtime,
                    "weights_sha256": hex32(&missing_path_spec.weights_sha256),
                    "corpus_hash": hex32(&missing_path_spec.corpus_hash)
                },
                "after": {
                    "signal_kind": missing_path_signal_kind.as_str(),
                    "learned_encoder": missing_path_signal_kind.is_learned_encoder()
                }
            }
        ]
    });
    let readback_path = root.join("issue936-readback.json");
    write_json(&readback_path, &readback);
    let persisted_readback: serde_json::Value = read_json(&readback_path).expect("read FSV JSON");

    println!("ISSUE936_FSV_ROOT={}", root.display());
    println!("ISSUE936_READBACK={}", readback_path.display());
    println!("ISSUE936_READBACK_SHA256={}", file_digest(&readback_path));
    println!(
        "ISSUE936_COMMISSIONED_SIGNAL_KIND={}",
        persisted_readback["after"]["commissioned_spec_signal_kind"]
    );
    println!(
        "ISSUE936_GATE_DECISION={}",
        persisted_readback["after"]["gate_decision"]
    );
    println!(
        "ISSUE936_STATIC_SIGNAL_KIND={}",
        persisted_readback["after"]["static_signal_kind"]
    );

    assert_eq!(
        persisted_readback["after"]["commissioned_spec_signal_kind"],
        "deterministic_content_feature"
    );
    assert_eq!(persisted_readback["after"]["gate_decision"], "park");
    assert_eq!(
        persisted_readback["after"]["static_signal_kind"],
        "learned_encoder"
    );
    assert_eq!(
        persisted_readback["edge_cases"][0]["after"]["error_code"],
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    assert_eq!(
        persisted_readback["edge_cases"][1]["after"]["signal_kind"],
        "unknown"
    );
    assert_eq!(
        persisted_readback["edge_cases"][2]["after"]["signal_kind"],
        "unknown"
    );
}

struct StaticRuntimeReadback {
    lens_id: calyx_core::LensId,
    embeddings_file: PathBuf,
    tokenizer: PathBuf,
    weights_sha256: [u8; 32],
    row_count: u32,
    vector: Vec<f32>,
    spec: LensSpec,
}

fn register_static_lookup(registry: &mut Registry, root: &Path) -> StaticRuntimeReadback {
    let static_dir = root.join("static-lookup");
    fs::create_dir_all(&static_dir).expect("create static dir");
    let embeddings_file = static_dir.join("embeddings.cslm");
    let tokenizer = static_dir.join("tokenizer.json");
    write_i8_matrix(
        &embeddings_file,
        &[[0, 0, 0], [3, 0, 0], [0, 4, 0], [0, 0, 5]],
        3,
    );
    write_tokenizer(&tokenizer);
    let lens = StaticLookupLens::from_files(StaticLookupFileSpec {
        name: "issue936-static-lookup".to_string(),
        embeddings_file: embeddings_file.clone(),
        tokenizer: tokenizer.clone(),
        dim: Some(3),
        norm_policy: NormPolicy::unit(),
        expected_weights_sha256: None,
    })
    .expect("load static lookup lens");
    let vector = match lens
        .measure(&Input::new(Modality::Text, b"alpha beta".to_vec()))
        .expect("measure known static lookup vector")
    {
        SlotVector::Dense { data, .. } => data,
        other => panic!("expected dense static lookup vector, got {other:?}"),
    };
    let spec = lens.lens_spec();
    let weights_sha256 = spec.weights_sha256;
    let row_count = lens.row_count();
    let contract = lens.contract().clone();
    let lens_id = registry
        .register_frozen_with_spec(lens, contract, spec.clone())
        .expect("register static lookup");
    StaticRuntimeReadback {
        lens_id,
        embeddings_file,
        tokenizer,
        weights_sha256,
        row_count,
        vector,
        spec,
    }
}

fn profile_probes() -> Vec<ProfileProbe> {
    vec![
        ProfileProbe::labeled(Input::new(Modality::Text, b"alpha words".to_vec()), "words"),
        ProfileProbe::labeled(Input::new(Modality::Text, b"beta phrase".to_vec()), "words"),
        ProfileProbe::labeled(
            Input::new(Modality::Text, b"12345 67890".to_vec()),
            "digits",
        ),
        ProfileProbe::labeled(
            Input::new(Modality::Text, b"98765 43210".to_vec()),
            "digits",
        ),
    ]
}

fn slot_for_lens(lens_id: calyx_core::LensId, name: &str) -> Slot {
    let slot_id = SlotId::new(0);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, name.to_string()),
        lens_id,
        shape: SlotShape::Dense(8),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some(name.to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: Default::default(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn assay_key() -> AssayCacheKey {
    AssayCacheKey::scoped(1, "issue936", vault_id(), AnchorKind::Reward)
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn write_i8_matrix(path: &Path, rows: &[[i8; 3]], dim: u32) {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MATRIX_MAGIC);
    bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&dim.to_le_bytes());
    bytes.push(DTYPE_I8);
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes.extend_from_slice(&1.0_f32.to_le_bytes());
    for row in rows {
        for value in row {
            bytes.push(*value as u8);
        }
    }
    fs::write(path, bytes).expect("write static matrix");
}

fn write_tokenizer(path: &Path) {
    let tokenizer = json!({
        "version": "1.0",
        "truncation": null,
        "padding": null,
        "added_tokens": [],
        "normalizer": null,
        "pre_tokenizer": {"type": "Whitespace"},
        "post_processor": null,
        "decoder": null,
        "model": {
            "type": "WordLevel",
            "vocab": {"[UNK]": 0, "alpha": 1, "beta": 2, "gamma": 3},
            "unk_token": "[UNK]"
        }
    });
    fs::write(
        path,
        serde_json::to_vec(&tokenizer).expect("tokenizer JSON"),
    )
    .expect("write tokenizer");
}

fn temp_root(label: &str) -> PathBuf {
    let base = calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", std::env::temp_dir);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    base.join(format!("{label}-{}-{nanos}", std::process::id()))
}

fn write_json(path: &Path, value: &impl serde::Serialize) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize JSON"),
    )
    .expect("write JSON");
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> serde_json::Result<T> {
    serde_json::from_slice(&fs::read(path).expect("read JSON"))
}

fn file_digest(path: &Path) -> String {
    let bytes = fs::read(path).expect("read digest input");
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    hex32(&digest)
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
