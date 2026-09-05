//! Issue #1489 regression net: the session-free static contract derivation
//! (`derive_runtime_contract_from_spec`) must be byte-identical to the
//! contract the warm path (`load_runtime_lens_from_spec`) constructs, for
//! every `LensRuntime` kind in the registry.
//!
//! Cheap runtimes are round-tripped through the real constructor. Model-backed
//! runtimes (candle, ONNX, fastembed, qwen3) pin the exact corpus-hash formula
//! their constructors use, so a change to either side breaks this net; the
//! full real-model round-trip runs in
//! `tests/issue1489_contract_parity_fsv.rs` on GPU hosts.

use super::*;
use crate::runtime::candle::{self, CandleDevicePolicy, CandlePoolingPolicy, CandlePrecision};
use crate::runtime::common::hash_files;

pub(super) fn fixture_dir(name: &str) -> PathBuf {
    let dir = temp_vault_dir(name).join("fixtures");
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn spec_with(
    name: &str,
    runtime: LensRuntime,
    output: SlotShape,
    weights_sha256: [u8; 32],
    norm_policy: NormPolicy,
) -> LensSpec {
    LensSpec {
        name: name.to_string(),
        runtime,
        output,
        modality: Modality::Text,
        weights_sha256,
        corpus_hash: [0; 32],
        norm_policy,
        max_batch: None,
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}

fn assert_contract_eq(kind: &str, derived: &FrozenLensContract, expected: &FrozenLensContract) {
    let diffs = contract_field_diffs(kind, expected, derived);
    assert!(
        diffs.is_empty(),
        "static contract derivation diverges for {kind}: {diffs:?}"
    );
}

/// Cheap runtimes: build the actual runtime lens and compare its contract
/// against the static derivation, field by field.
#[test]
fn static_derivation_round_trips_constructible_runtime_kinds() {
    let dir = fixture_dir("parity-cheap");

    // static_lookup fixture: f32 matrix with 2 rows x 4 dims + word-level tokenizer.
    let embeddings = dir.join("potion.cxlkup");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"CXLKUP1\0");
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&4u32.to_le_bytes());
    bytes.push(3); // f32
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes.extend_from_slice(&1.0f32.to_le_bytes());
    for value in [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(&embeddings, bytes).unwrap();
    let tokenizer = dir.join("tokenizer.json");
    fs::write(
        &tokenizer,
        br#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"alpha":1,"beta":2},"unk_token":"[UNK]"}}"#,
    )
    .unwrap();
    let lookup_weights = hash_files(&[embeddings.clone(), tokenizer.clone()]).unwrap();

    let specs = vec![
        spec_with(
            "parity-algorithmic-scalar",
            LensRuntime::Algorithmic {
                kind: "scalar".to_string(),
            },
            // The scalar encoder has a fixed dimension of 1; declaring any other
            // shape is (correctly) rejected by the runtime-kind reconstructor.
            SlotShape::Dense(1),
            [0; 32],
            NormPolicy::None,
        ),
        spec_with(
            "parity-algorithmic-sparse",
            LensRuntime::Algorithmic {
                kind: "sparse_keywords".to_string(),
            },
            SlotShape::Sparse(1024),
            [0; 32],
            NormPolicy::None,
        ),
        spec_with(
            "parity-tei",
            LensRuntime::TeiHttp {
                endpoint: "http://127.0.0.1:9".to_string(),
            },
            SlotShape::Dense(768),
            [0; 32],
            NormPolicy::None,
        ),
        spec_with(
            "parity-external-cmd",
            LensRuntime::ExternalCmd {
                cmd: "calyx-fixture-embedder".to_string(),
                args: vec!["--dim".to_string(), "16".to_string()],
            },
            SlotShape::Dense(16),
            [0; 32],
            NormPolicy::None,
        ),
        spec_with(
            "parity-static-lookup",
            LensRuntime::StaticLookup {
                embeddings_file: embeddings.clone(),
                tokenizer: tokenizer.clone(),
                dim: 4,
            },
            SlotShape::Dense(4),
            lookup_weights,
            NormPolicy::unit(),
        ),
    ];

    for spec in specs {
        let derived = derive_runtime_contract_from_spec(&spec).unwrap();
        let (_, runtime_contract) = load_runtime_lens_from_spec(&spec).unwrap();
        println!(
            "parity {}: derived_lens_id={} runtime_lens_id={}",
            spec.name,
            derived.lens_id(),
            runtime_contract.lens_id()
        );
        assert_contract_eq(&spec.name, &derived, &runtime_contract);
    }
}

/// Issue #1489 direction A->B: SPLADE-style sparse custom ONNX lenses must use
/// the sparse corpus-hash formula from `runtime::onnx::custom`, not the dense
/// one the static derivation used before the fix.
#[test]
fn onnx_sparse_static_contract_matches_runtime_splade_formula() {
    let dir = fixture_dir("parity-onnx-sparse");
    let model = dir.join("model.onnx");
    let tokenizer = dir.join("tokenizer.json");
    let config = dir.join("config.json");
    fs::write(&model, b"onnx fixture bytes").unwrap();
    fs::write(&tokenizer, br#"{"tokenizer":"fixture"}"#).unwrap();
    fs::write(
        &config,
        br#"{"pooling":"mean","max_position_embeddings":512}"#,
    )
    .unwrap();
    let model_id = "prithivida/Splade_PP_en_v1-onnx";
    let weights = [0x33; 32];
    let spec = spec_with(
        "parity-onnx-splade",
        LensRuntime::Onnx {
            model_id: model_id.to_string(),
            files: vec![model, tokenizer, config],
        },
        SlotShape::Sparse(30_522),
        weights,
        NormPolicy::Finite,
    );

    let derived = derive_runtime_contract_from_spec(&spec).unwrap();
    let sparse_corpus = sha256_digest(&[
        b"onnx-custom-splade-v1",
        model_id.as_bytes(),
        b"sparse-positive-f32",
    ]);
    let expected = FrozenLensContract::new(
        spec.name.clone(),
        weights,
        sparse_corpus,
        SlotShape::Sparse(30_522),
        Modality::Text,
        LensDType::F32,
        NormPolicy::Finite,
    );
    println!(
        "onnx-splade parity: derived_lens_id={} expected_lens_id={}",
        derived.lens_id(),
        expected.lens_id()
    );
    assert_contract_eq("onnx-splade", &derived, &expected);

    // The pre-fix static derivation used the dense formula; make sure that
    // regression cannot silently return.
    let pre_fix_dense_corpus = sha256_digest(&[
        b"onnx-custom-v1",
        model_id.as_bytes(),
        b"mean",
        format!("{:?}", NormPolicy::Finite).as_bytes(),
    ]);
    assert_ne!(derived.corpus_hash(), pre_fix_dense_corpus);
}

#[test]
fn onnx_dense_static_contract_matches_runtime_formula() {
    let dir = fixture_dir("parity-onnx-dense");
    let model = dir.join("model.onnx");
    let tokenizer = dir.join("tokenizer.json");
    let config = dir.join("config.json");
    fs::write(&model, b"onnx fixture bytes").unwrap();
    fs::write(&tokenizer, br#"{"tokenizer":"fixture"}"#).unwrap();
    fs::write(
        &config,
        br#"{"pooling":"cls","max_position_embeddings":512}"#,
    )
    .unwrap();
    let model_id = "intfloat/e5-base-v2-onnx";
    let norm = NormPolicy::unit();
    let spec = spec_with(
        "parity-onnx-dense",
        LensRuntime::Onnx {
            model_id: model_id.to_string(),
            files: vec![model, tokenizer, config],
        },
        SlotShape::Dense(768),
        [0x44; 32],
        norm,
    );

    let derived = derive_runtime_contract_from_spec(&spec).unwrap();
    let expected_corpus = sha256_digest(&[
        b"onnx-custom-v1",
        model_id.as_bytes(),
        b"cls",
        format!("{norm:?}").as_bytes(),
    ]);
    assert_eq!(derived.corpus_hash(), expected_corpus);
    assert_eq!(derived.shape(), SlotShape::Dense(768));
}

/// Issue #1489 direction B->A: candle F16/BF16 lenses replay non-finite rows
/// at F32 on the warm path (CudaFailLoud), so the frozen corpus hash encodes
/// "f32" — not the model precision the static derivation encoded before the
/// fix.
#[test]
fn candle_half_precision_static_contract_matches_warm_path_finite_replay() {
    let dir = fixture_dir("parity-candle");
    let weights_file = dir.join("model.safetensors");
    let tokenizer = dir.join("tokenizer.json");
    let config = dir.join("config.json");
    fs::write(&weights_file, b"safetensors fixture bytes").unwrap();
    fs::write(&tokenizer, br#"{"tokenizer":"fixture"}"#).unwrap();
    fs::write(&config, br#"{"hidden_size":384}"#).unwrap();
    let model_id = "sentence-transformers/all-MiniLM-L6-v2";

    for (dtype, expected_replay) in [("f16", "f32"), ("bf16", "f32"), ("f32", "none")] {
        let norm = NormPolicy::unit();
        let spec = spec_with(
            &format!("parity-candle-{dtype}"),
            LensRuntime::CandleLocal {
                model_id: model_id.to_string(),
                files: vec![weights_file.clone(), tokenizer.clone(), config.clone()],
                dtype: dtype.to_string(),
                pooling: "mean".to_string(),
            },
            SlotShape::Dense(384),
            [0x55; 32],
            norm,
        );
        let derived = derive_runtime_contract_from_spec(&spec).unwrap();
        let norm_text = format!("{norm:?}");
        let expected_corpus = sha256_digest(&[
            b"candle-local-bert-v2",
            model_id.as_bytes(),
            b"512",
            dtype.as_bytes(),
            b"mean",
            norm_text.as_bytes(),
            expected_replay.as_bytes(),
        ]);
        println!(
            "candle parity dtype={dtype}: expected_replay={expected_replay} derived_lens_id={}",
            derived.lens_id()
        );
        assert_eq!(
            derived.corpus_hash(),
            expected_corpus,
            "candle {dtype} corpus hash must encode finite replay {expected_replay}"
        );
        assert_eq!(derived.shape(), SlotShape::Dense(384));
        assert_eq!(derived.weights_sha256(), [0x55; 32]);

        // Shared-formula cross-check: the warm path computes the replay
        // precision from the same helper the static derivation now uses.
        let replay = candle::contract_finite_replay_precision(
            candle::LENS_SPEC_DEVICE_POLICY,
            CandlePrecision::parse(dtype).unwrap(),
        );
        assert_eq!(
            derived.corpus_hash(),
            candle::contract_corpus_hash(
                model_id,
                512,
                CandlePrecision::parse(dtype).unwrap(),
                CandlePoolingPolicy::Mean,
                norm,
                replay,
            )
        );

        if dtype != "f32" {
            // The pre-fix static derivation hashed the model precision as the
            // replay text; that identity must never come back.
            let pre_fix_corpus = sha256_digest(&[
                b"candle-local-bert-v2",
                model_id.as_bytes(),
                b"512",
                dtype.as_bytes(),
                b"mean",
                norm_text.as_bytes(),
                dtype.as_bytes(),
            ]);
            assert_ne!(derived.corpus_hash(), pre_fix_corpus);
        }
    }
}

#[test]
fn candle_cpu_policy_does_not_use_finite_replay() {
    // The warm path always rehydrates candle lenses with CudaFailLoud; the
    // static derivation must track that policy, not CPU.
    assert_eq!(
        candle::contract_finite_replay_precision(
            CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
            CandlePrecision::F16,
        ),
        Some(CandlePrecision::F32)
    );
    assert_eq!(
        candle::contract_finite_replay_precision(
            CandleDevicePolicy::CpuExplicit,
            CandlePrecision::F16,
        ),
        None
    );
    assert_eq!(
        candle::contract_finite_replay_precision(
            CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
            CandlePrecision::F32,
        ),
        None
    );
}
