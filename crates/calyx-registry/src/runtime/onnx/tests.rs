use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{Input, Lens, Modality, SlotShape, SlotVector};
use proptest::prelude::*;

use super::custom::pool_output;
use super::fastembed_runtime::execution_providers;
use super::*;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

#[test]
fn execution_provider_policy_is_cuda_fail_loud() {
    let providers = execution_providers(OnnxProviderPolicy::CudaFailLoud);

    assert_eq!(providers.len(), 1);
    let provider = format!("{:?}", providers[0]);
    assert!(provider.contains("CUDA"));
    assert!(provider.contains("error_on_failure: true"));
}

#[test]
fn execution_provider_policy_can_be_explicit_cpu() {
    let providers = execution_providers(OnnxProviderPolicy::CpuExplicit);

    assert_eq!(providers.len(), 1);
    let provider = format!("{:?}", providers[0]);
    assert!(provider.contains("CPU"));
    assert!(!provider.contains("CUDA"));
}

#[test]
fn persisted_onnx_manifest_defaults_to_cuda_fail_loud() {
    let fixture = Fixture::new("manifest-provider", &[3.0, 4.0, 0.0]);
    let spec = OnnxLens::from_files(fixture.spec("custom-provider"))
        .unwrap()
        .lens_spec();

    let file_spec = OnnxFileSpec::from_lens_spec(&spec).unwrap();

    assert_eq!(file_spec.provider_policy, OnnxProviderPolicy::CudaFailLoud);
}

#[test]
fn custom_onnx_from_files_measures_unit_norm_vector() {
    let fixture = Fixture::new("unit-norm", &[3.0, 4.0, 0.0]);
    let lens = OnnxLens::from_files(
        fixture
            .spec("custom-unit")
            .with_expected_shape(SlotShape::Dense(3)),
    )
    .unwrap();

    assert_eq!(lens.shape(), SlotShape::Dense(3));
    assert_eq!(lens.runtime_name(), "onnx-custom");
    let vector = lens
        .measure(&Input::new(Modality::Text, b"hello calyx".to_vec()))
        .unwrap();

    let SlotVector::Dense { dim, data } = vector else {
        panic!("expected dense custom ONNX vector");
    };
    assert_eq!(dim, 3);
    let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1.0e-6);
    assert!((data[0] - 0.6).abs() < 1.0e-6);
    assert!((data[1] - 0.8).abs() < 1.0e-6);
}

#[test]
fn custom_onnx_lens_spec_round_trips_runtime_files() {
    let fixture = Fixture::new("spec-roundtrip", &[3.0, 4.0, 0.0]);
    let lens = OnnxLens::from_files(fixture.spec("custom-spec")).unwrap();
    let spec = lens.lens_spec();

    let reloaded = OnnxLens::from_lens_spec(&spec).unwrap();
    assert_eq!(reloaded.id(), lens.id());
    assert_eq!(reloaded.runtime_name(), "onnx-custom");
    let vector = reloaded
        .measure(&Input::new(Modality::Text, b"calyx".to_vec()))
        .unwrap();

    lens.contract()
        .verify_vector(reloaded.id(), &vector)
        .unwrap();
}

#[test]
fn custom_onnx_file_hash_controls_lens_id_and_frozen_violation() {
    let fixture = Fixture::new("hash", &[3.0, 4.0, 0.0]);
    let first = OnnxLens::from_files(fixture.spec("custom-hash")).unwrap();
    let second = OnnxLens::from_files(fixture.spec("custom-hash")).unwrap();
    assert_eq!(first.id(), second.id());

    let expected = first.contract().weights_sha256();
    fs::write(
        &fixture.config,
        r#"{"model_type":"calyx-test","pooling":"cls"}"#,
    )
    .unwrap();
    let changed = OnnxLens::from_files(fixture.spec("custom-hash")).unwrap();
    assert_ne!(first.id(), changed.id());

    let error = lens_error(OnnxLens::from_files(
        fixture
            .spec("custom-hash")
            .with_expected_weights_sha256(expected),
    ));
    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

#[test]
fn custom_onnx_missing_tokenizer_is_config_invalid() {
    let fixture = Fixture::new("missing-tokenizer", &[3.0, 4.0, 0.0]);
    fs::remove_file(&fixture.tokenizer).unwrap();

    let error = lens_error(OnnxLens::from_files(fixture.spec("custom-missing")));

    assert_eq!(error.code, "CALYX_LENS_CONFIG_INVALID");
}

#[test]
fn custom_onnx_declared_dim_mismatch_fails_closed() {
    let fixture = Fixture::new("dim-mismatch", &[3.0, 4.0, 0.0]);

    let error = lens_error(OnnxLens::from_files(
        fixture
            .spec("custom-dim")
            .with_expected_shape(SlotShape::Dense(4)),
    ));

    assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
}

#[test]
fn custom_onnx_non_finite_output_is_numerical_invariant() {
    let fixture = Fixture::new("nan", &[f32::NAN, 1.0, 0.0]);
    let lens = OnnxLens::from_files(fixture.spec("custom-nan")).unwrap();

    let error = lens
        .measure(&Input::new(Modality::Text, b"hello".to_vec()))
        .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_NUMERICAL_INVARIANT");
}

proptest! {
    #[test]
    fn pooling_is_deterministic(values in proptest::collection::vec(-10.0f32..10.0, 12)) {
        let shape = [1, 4, 3];
        let mask = [1, 1, 0, 1];
        for policy in [PoolingPolicy::Mean, PoolingPolicy::Cls, PoolingPolicy::LastToken] {
            let first = pool_output(&shape, &values, &mask, policy, 3).unwrap();
            for _ in 0..100 {
                prop_assert_eq!(pool_output(&shape, &values, &mask, policy, 3).unwrap(), first.clone());
            }
        }
    }
}

#[test]
#[ignore = "requires manual HF cache/network and downloads ONNX all-MiniLM"]
fn onnx_all_minilm_manual_fsv() {
    let lens = OnnxLens::all_minilm_l6_v2_cpu_explicit("onnx-manual-fsv").unwrap();
    let input = Input::new(Modality::Text, b"Calyx PH19 ONNX local probe".to_vec());
    let vector = lens.measure(&input).unwrap();

    if let SlotVector::Dense { dim, data } = vector {
        println!("ONNX_FSV_CACHE={}", lens.files().cache_dir.display());
        println!("ONNX_FSV_MODEL={}", lens.files().model_file.display());
        println!("ONNX_FSV_PROVIDER_POLICY={}", lens.provider_policy());
        println!("ONNX_FSV_DIM={dim}");
        println!("ONNX_FSV_FIRST3={:?}", &data[..3]);
        let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
        println!("ONNX_FSV_NORM={norm:.8}");
        assert!((norm - 1.0).abs() < 1.0e-3);
    } else {
        panic!("expected dense ONNX vector");
    }
}

#[test]
#[ignore = "requires manual CUDA/ONNX stack; validates fail-loud GPU policy"]
fn onnx_cuda_fail_loud_manual_fsv() {
    let input = Input::new(Modality::Text, b"Calyx PH19 CUDA fail-loud probe".to_vec());
    match OnnxLens::all_minilm_l6_v2("onnx-manual-cuda-fail-loud") {
        Ok(lens) => match lens.measure(&input) {
            Ok(vector) => {
                println!("ONNX_CUDA_RESULT=success");
                if let SlotVector::Dense { dim, data } = vector {
                    let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
                    println!("ONNX_CUDA_DIM={dim}");
                    println!("ONNX_CUDA_NORM={norm:.8}");
                    assert!((norm - 1.0).abs() < 1.0e-3);
                }
            }
            Err(error) => {
                println!("ONNX_CUDA_RESULT=fail_loud");
                println!("ONNX_CUDA_ERROR_CODE={}", error.code);
                println!("ONNX_CUDA_ERROR_MESSAGE={}", error.message);
                assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
            }
        },
        Err(error) => {
            println!("ONNX_CUDA_RESULT=fail_loud_init");
            println!("ONNX_CUDA_ERROR_CODE={}", error.code);
            println!("ONNX_CUDA_ERROR_MESSAGE={}", error.message);
            assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
        }
    }
}

#[test]
#[ignore = "requires manual HF cache/network and downloads ONNX all-MiniLM"]
fn onnx_dim_guard_manual_fsv() {
    let lens = OnnxLens::all_minilm_l6_v2_cpu_explicit("onnx-manual-dim-guard").unwrap();
    let error = lens
        .contract()
        .verify_vector(
            lens.id(),
            &SlotVector::Dense {
                dim: 3,
                data: vec![1.0, 0.0, 0.0],
            },
        )
        .unwrap_err();

    println!("ONNX_DIM_GUARD_ERROR={}", error.code);
    assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
}

#[test]
#[ignore = "requires explicit custom ONNX env paths in a manual verification run"]
fn custom_onnx_manual_fsv_from_files() {
    let model = std::env::var("CALYX_CUSTOM_ONNX_MODEL").unwrap();
    let tokenizer = std::env::var("CALYX_CUSTOM_ONNX_TOKENIZER").unwrap();
    let config = std::env::var("CALYX_CUSTOM_ONNX_CONFIG").unwrap();
    let lens = OnnxLens::from_files(
        OnnxFileSpec::text(
            "onnx-custom-manual-fsv",
            "Xenova/bge-small-en-v1.5",
            model,
            tokenizer,
            config,
            PoolingPolicy::Mean,
            NormPolicy::unit(),
        )
        .with_provider_policy(OnnxProviderPolicy::CpuExplicit),
    )
    .unwrap();
    let vector = lens
        .measure(&Input::new(
            Modality::Text,
            b"Calyx PH73 custom ONNX explicit file probe".to_vec(),
        ))
        .unwrap();
    let spec = lens.lens_spec();
    let reloaded = OnnxLens::from_lens_spec(&spec).unwrap();
    assert_eq!(lens.id(), reloaded.id());

    let SlotVector::Dense { dim, data } = vector else {
        panic!("expected dense custom ONNX vector");
    };
    let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
    println!("ONNX_CUSTOM_FSV_RUNTIME={}", lens.runtime_name());
    println!("ONNX_CUSTOM_FSV_MODEL_ID={}", lens.files().model_code);
    println!("ONNX_CUSTOM_FSV_LENS_ID={}", lens.id());
    println!("ONNX_CUSTOM_FSV_CORPUS_HASH={}", hex32(&spec.corpus_hash));
    println!(
        "ONNX_CUSTOM_FSV_WEIGHTS_SHA256={}",
        hex32(&spec.weights_sha256)
    );
    println!("ONNX_CUSTOM_FSV_DIM={dim}");
    println!("ONNX_CUSTOM_FSV_DTYPE=int8");
    println!("ONNX_CUSTOM_FSV_NORM={norm:.8}");
    println!("ONNX_CUSTOM_FSV_FIRST3={:?}", &data[..3]);
    println!(
        "ONNX_CUSTOM_FSV_SPEC_RELOAD_RUNTIME={}",
        reloaded.runtime_name()
    );
    assert_eq!(lens.runtime_name(), "onnx-custom");
    assert!((norm - 1.0).abs() < 1.0e-3);
}

#[test]
#[ignore = "manual PH73 edge FSV prints source-of-truth file states"]
fn custom_onnx_edges_manual_fsv() {
    let missing = Fixture::new("edge-missing-tokenizer", &[3.0, 4.0, 0.0]);
    println!(
        "EDGE_MISSING_TOKENIZER_BEFORE_EXISTS={}",
        missing.tokenizer.is_file()
    );
    fs::remove_file(&missing.tokenizer).unwrap();
    let missing_error = lens_error(OnnxLens::from_files(missing.spec("edge-missing")));
    println!(
        "EDGE_MISSING_TOKENIZER_AFTER_EXISTS={}",
        missing.tokenizer.is_file()
    );
    println!("EDGE_MISSING_TOKENIZER_ERROR={}", missing_error.code);
    assert_eq!(missing_error.code, "CALYX_LENS_CONFIG_INVALID");

    let dim = Fixture::new("edge-dim", &[3.0, 4.0, 0.0]);
    let dim_error = lens_error(OnnxLens::from_files(
        dim.spec("edge-dim")
            .with_expected_shape(SlotShape::Dense(4)),
    ));
    println!(
        "EDGE_DECLARED_DIM_ACTUAL=3 DECLARED=4 ERROR={}",
        dim_error.code
    );
    assert_eq!(dim_error.code, "CALYX_LENS_DIM_MISMATCH");

    let nan = Fixture::new("edge-nan", &[f32::NAN, 1.0, 0.0]);
    let nan_lens = OnnxLens::from_files(nan.spec("edge-nan")).unwrap();
    let nan_error = nan_lens
        .measure(&Input::new(Modality::Text, b"hello".to_vec()))
        .unwrap_err();
    println!("EDGE_NON_FINITE_OUTPUT_ERROR={}", nan_error.code);
    assert_eq!(nan_error.code, "CALYX_LENS_NUMERICAL_INVARIANT");

    let drift = Fixture::new("edge-hash", &[3.0, 4.0, 0.0]);
    let original = OnnxLens::from_files(drift.spec("edge-hash")).unwrap();
    let expected = original.contract().weights_sha256();
    fs::write(
        &drift.config,
        r#"{"model_type":"calyx-test","pooling":"cls"}"#,
    )
    .unwrap();
    let drift_error = lens_error(OnnxLens::from_files(
        drift
            .spec("edge-hash")
            .with_expected_weights_sha256(expected),
    ));
    println!(
        "EDGE_HASH_DRIFT_EXPECTED={} ERROR={}",
        hex32(&expected),
        drift_error.code
    );
    assert_eq!(drift_error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

struct Fixture {
    root: PathBuf,
    model: PathBuf,
    tokenizer: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new(name: &str, output: &[f32]) -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "calyx-custom-onnx-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let model = root.join("model.onnx");
        let tokenizer = root.join("tokenizer.json");
        let config = root.join("config.json");
        write_tokenizer(&tokenizer);
        fs::write(
            &config,
            r#"{"model_type":"calyx-test","hidden_size":3,"pooling":"mean"}"#,
        )
        .unwrap();
        write_model(&model, output);
        Self {
            root,
            model,
            tokenizer,
            config,
        }
    }

    fn spec(&self, name: &str) -> OnnxFileSpec {
        OnnxFileSpec::text(
            name,
            "calyx-test-custom-onnx",
            self.model.clone(),
            self.tokenizer.clone(),
            self.config.clone(),
            PoolingPolicy::Mean,
            NormPolicy::unit(),
        )
        .with_provider_policy(OnnxProviderPolicy::CpuExplicit)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_tokenizer(path: &Path) {
    fs::write(
        path,
        r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"hello":1,"calyx":2},"unk_token":"[UNK]"}}"#,
    )
    .unwrap();
}

fn write_model(path: &Path, output: &[f32]) {
    use ort::editor::{Graph, Model, ONNX_DOMAIN, Opset};
    use ort::memory::Allocator;
    use ort::session::Session;
    use ort::value::{Outlet, Shape, SymbolicDimensions, Tensor, TensorElementType, ValueType};

    let mut graph = Graph::new().unwrap();
    graph
        .set_inputs([Outlet::new(
            "input_ids",
            ValueType::Tensor {
                ty: TensorElementType::Int64,
                shape: Shape::new([1, -1]),
                dimension_symbols: SymbolicDimensions::empty(2),
            },
        )])
        .unwrap();
    graph
        .set_outputs([Outlet::new(
            "sentence_embedding",
            ValueType::Tensor {
                ty: TensorElementType::Float32,
                shape: Shape::new([1, output.len() as i64]),
                dimension_symbols: SymbolicDimensions::empty(2),
            },
        )])
        .unwrap();
    let mut tensor =
        Tensor::<f32>::new(&Allocator::default(), [1_i64, output.len() as i64]).unwrap();
    tensor.extract_tensor_mut().1.copy_from_slice(output);
    graph
        .add_initializer("sentence_embedding", tensor, false)
        .unwrap();
    let mut model = Model::new([Opset::new(ONNX_DOMAIN, 22).unwrap()]).unwrap();
    model.add_graph(graph).unwrap();
    let builder = Session::builder()
        .unwrap()
        .with_optimized_model_path(path)
        .unwrap();
    let session = model.into_session(&builder).unwrap();
    drop(session);
    assert!(
        path.is_file(),
        "expected ORT to materialize {}",
        path.display()
    );
}

fn lens_error(result: Result<OnnxLens>) -> calyx_core::CalyxError {
    match result {
        Ok(lens) => panic!("expected error, got lens {}", lens.id()),
        Err(error) => error,
    }
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
