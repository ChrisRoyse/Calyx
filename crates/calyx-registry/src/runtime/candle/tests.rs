use super::*;
use crate::runtime::onnx::OnnxLens;
use candle_core::Device;
use candle_transformers::models::bert::Config;
use serde_json::json;
use std::fs;
use std::path::Path;

#[test]
fn mean_pool_uses_attention_mask() {
    let tokens = vec![vec![1.0, 3.0], vec![5.0, 9.0]];

    let pooled = mean_pool(&tokens, &[1, 0], 2).unwrap();

    assert_eq!(pooled, vec![1.0, 3.0]);
}

#[test]
fn mean_pool_rejects_wrong_dim() {
    let error = mean_pool(&[vec![1.0]], &[1], 2).unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
}

#[test]
fn cls_pool_uses_first_unmasked_token() {
    let tokens = vec![vec![1.0, 3.0], vec![5.0, 9.0]];

    let pooled = pool_tokens(&tokens, &[0, 1], 2, CandlePoolingPolicy::Cls).unwrap();

    assert_eq!(pooled, vec![5.0, 9.0]);
}

#[test]
fn candle_precision_and_pooling_parse_manifest_values() {
    assert_eq!(
        CandlePrecision::parse("fp16").unwrap(),
        CandlePrecision::F16
    );
    assert_eq!(
        CandlePrecision::parse("bf16").unwrap(),
        CandlePrecision::BF16
    );
    assert_eq!(
        CandlePoolingPolicy::parse("first-token").unwrap(),
        CandlePoolingPolicy::Cls
    );

    let error = CandlePrecision::parse("mxfp8").unwrap_err();
    assert_eq!(error.code, "CALYX_LENS_CONFIG_INVALID");
}

#[test]
fn candle_oom_error_maps_to_vram_oom() {
    let error = candle_error_message("CUDA out of memory during candle allocation".to_string());

    assert_eq!(error.code, "CALYX_VRAM_OOM");
}

#[test]
fn candle_model_files_preserve_manifest_contract_paths() {
    let root = Path::new("/tmp/calyx-candle-contract");
    let weights = root.join("model.safetensors");
    let tokenizer = root.join("tokenizer.json");
    let config = root.join("config.json");
    let tokenizer_config = root.join("tokenizer_config.json");
    let files = CandleModelFiles {
        cache_dir: root.to_path_buf(),
        model_id: "fixture/model".to_string(),
        config: config.clone(),
        tokenizer: tokenizer.clone(),
        weights: weights.clone(),
        contract_paths: vec![
            weights.clone(),
            tokenizer.clone(),
            config.clone(),
            tokenizer_config.clone(),
        ],
    };

    assert_eq!(
        files.artifact_paths(),
        vec![weights, tokenizer, config, tokenizer_config]
    );
}

#[test]
fn half_cuda_config_raises_layer_norm_epsilon() {
    let mut half_cuda = Config {
        layer_norm_eps: 1.0e-12,
        ..Default::default()
    };
    stabilize_half_cuda_config(
        &mut half_cuda,
        CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
        CandlePrecision::F16,
    );
    assert_eq!(half_cuda.layer_norm_eps, HALF_CUDA_MIN_LAYER_NORM_EPS);

    let mut f32_cuda = Config {
        layer_norm_eps: 1.0e-12,
        ..Default::default()
    };
    stabilize_half_cuda_config(
        &mut f32_cuda,
        CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
        CandlePrecision::F32,
    );
    assert_eq!(f32_cuda.layer_norm_eps, 1.0e-12);

    let mut bf16_cpu = Config {
        layer_norm_eps: 1.0e-12,
        ..Default::default()
    };
    stabilize_half_cuda_config(
        &mut bf16_cpu,
        CandleDevicePolicy::CpuExplicit,
        CandlePrecision::BF16,
    );
    assert_eq!(bf16_cpu.layer_norm_eps, 1.0e-12);
}

#[test]
fn half_cuda_requests_f32_finite_replay() {
    assert!(needs_f32_finite_replay(
        CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
        CandlePrecision::F16
    ));
    assert!(needs_f32_finite_replay(
        CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
        CandlePrecision::BF16
    ));
    assert!(!needs_f32_finite_replay(
        CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
        CandlePrecision::F32
    ));
    assert!(!needs_f32_finite_replay(
        CandleDevicePolicy::CpuExplicit,
        CandlePrecision::F16
    ));
}

#[test]
fn candle_device_policy_reports_cpu_and_cuda_truth() {
    assert_eq!(
        CandleDevicePolicy::CpuExplicit.as_str(),
        "cpu_explicit,no_cuda"
    );
    assert!(matches!(
        candle_device(CandleDevicePolicy::CpuExplicit).unwrap(),
        Device::Cpu
    ));
    let cuda_feature = cfg!(feature = "candle-cuda");
    let cuda_result = candle_device(CandleDevicePolicy::CudaFailLoud { ordinal: 0 });
    let cuda_error = if cuda_feature {
        assert!(cuda_result.is_ok() || cuda_result.as_ref().unwrap_err().message.contains("CUDA"));
        cuda_result.err()
    } else {
        let error = cuda_result.expect_err("cuda feature is not compiled by default");
        assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
        assert!(error.message.contains("without feature `candle-cuda`"));
        Some(error)
    };

    if let Some(root) = std::env::var_os("CALYX_FSV_ROOT") {
        write_device_policy_readback(Path::new(&root), cuda_feature, cuda_error);
    }
}

fn write_device_policy_readback(root: &Path, cuda_feature: bool, cuda_error: Option<CalyxError>) {
    fs::create_dir_all(root).unwrap();
    let readback = json!({
        "default_policy": CandleDevicePolicy::CpuExplicit.as_str(),
        "cuda_policy": CandleDevicePolicy::CudaFailLoud { ordinal: 0 }.as_str(),
        "candle_cuda_feature_compiled": cuda_feature,
        "cuda_fail_loud_error_code": cuda_error.as_ref().map(|error| error.code),
        "cuda_fail_loud_error_message": cuda_error.as_ref().map(|error| error.message.as_str()),
    });
    fs::write(
        root.join("candle-device-policy-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires manual HF cache/network and downloads all-MiniLM weights"]
fn candle_all_minilm_manual_fsv() {
    let lens = CandleLens::all_minilm_l6_v2("candle-manual-fsv").unwrap();
    println!("CANDLE_FSV_DEVICE_POLICY={}", lens.device_policy().as_str());
    let input = Input::new(Modality::Text, b"Calyx PH19 candle local probe".to_vec());
    let vector = lens.measure(&input).unwrap();

    if let SlotVector::Dense { dim, data } = vector {
        println!("CANDLE_FSV_CACHE={}", lens.files().cache_dir.display());
        println!("CANDLE_FSV_WEIGHTS={}", lens.files().weights.display());
        println!("CANDLE_FSV_DIM={dim}");
        println!("CANDLE_FSV_FIRST3={:?}", &data[..3]);
        let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
        println!("CANDLE_FSV_NORM={norm:.8}");
        assert!((norm - 1.0).abs() < 1.0e-3);
    } else {
        panic!("expected dense candle vector");
    }
}

#[test]
#[ignore = "requires manual HF cache/network and downloads all-MiniLM weights"]
fn candle_dim_guard_manual_fsv() {
    let lens = CandleLens::all_minilm_l6_v2("candle-manual-dim-guard").unwrap();
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

    println!("CANDLE_DIM_GUARD_ERROR={}", error.code);
    assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");

    let empty = lens
        .measure(&Input::new(Modality::Text, Vec::new()))
        .unwrap();
    if let SlotVector::Dense { dim, data } = empty {
        let norm = data.iter().map(|v| v * v).sum::<f32>().sqrt();
        println!("CANDLE_EMPTY_DIM={dim}");
        println!("CANDLE_EMPTY_NORM={norm:.8}");
        println!("CANDLE_EMPTY_FIRST3={:?}", &data[..3]);
        assert!((norm - 1.0).abs() < 1.0e-3);
    } else {
        panic!("expected dense empty candle vector");
    }

    let invalid = lens
        .measure(&Input::new(Modality::Text, vec![0xff]))
        .unwrap_err();
    println!("CANDLE_INVALID_UTF8_ERROR={}", invalid.code);
    assert_eq!(invalid.code, "CALYX_LENS_DIM_MISMATCH");

    let wrong_modality = lens
        .measure(&Input::new(Modality::Image, b"pixels".to_vec()))
        .unwrap_err();
    println!("CANDLE_WRONG_MODALITY_ERROR={}", wrong_modality.code);
    assert_eq!(wrong_modality.code, "CALYX_LENS_DIM_MISMATCH");
}

#[test]
#[ignore = "requires manual CUDA, candle-cuda feature, and CALYX_CANDLE_FSV_MANIFEST"]
fn candle_fp16_cuda_manifest_manual_fsv() {
    let manifest = std::env::var("CALYX_CANDLE_FSV_MANIFEST")
        .expect("CALYX_CANDLE_FSV_MANIFEST points to candle-fp16 manifest");
    let expected_precision = std::env::var("CALYX_CANDLE_FSV_EXPECT_DTYPE")
        .map(|raw| CandlePrecision::parse(&raw).expect("CALYX_CANDLE_FSV_EXPECT_DTYPE is valid"))
        .unwrap_or(CandlePrecision::F16);
    let spec = crate::lens_spec_from_manifest_path(&manifest).unwrap();
    let lens = CandleLens::from_lens_spec(&spec).unwrap();
    assert_eq!(lens.precision(), expected_precision);
    assert!(matches!(
        lens.device_policy(),
        CandleDevicePolicy::CudaFailLoud { ordinal: 0 }
    ));

    let input = Input::new(
        Modality::Text,
        b"Calyx PH73 candle fp16 CUDA determinism probe".to_vec(),
    );
    let first = dense(lens.measure(&input).unwrap());
    let second = dense(lens.measure(&input).unwrap());
    let cosine = cosine(&first.1, &second.1);

    println!("CANDLE_FP16_FSV_MANIFEST={manifest}");
    println!("CANDLE_FP16_FSV_LENS_ID={}", lens.id());
    println!("CANDLE_FP16_FSV_DTYPE={}", lens.precision().as_str());
    println!("CANDLE_FP16_FSV_POOLING={}", lens.pooling().as_str());
    println!(
        "CANDLE_FP16_FSV_FINITE_REPLAY_DTYPE={}",
        lens.finite_replay_precision()
            .map(CandlePrecision::as_str)
            .unwrap_or("none")
    );
    println!(
        "CANDLE_FP16_FSV_DEVICE_POLICY={}",
        lens.device_policy().as_str()
    );
    println!("CANDLE_FP16_FSV_DIM={}", first.0);
    println!("CANDLE_FP16_FSV_FIRST4={:?}", &first.1[..4]);
    println!("CANDLE_FP16_FSV_REPLAY_COSINE={cosine:.8}");
    println!("CANDLE_FP16_FSV_NORM={:.8}", norm(&first.1));

    assert_eq!(first.0, second.0);
    assert!(cosine >= 0.9999);
    assert!((norm(&first.1) - 1.0).abs() < 1.0e-3);

    if let Ok(raw) = std::env::var("CALYX_CANDLE_FSV_HOLD_SECS") {
        let secs: u64 = raw.parse().expect("CALYX_CANDLE_FSV_HOLD_SECS is u64");
        println!("CANDLE_FP16_FSV_HOLD_SECS={secs}");
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
}

#[test]
#[ignore = "requires manual CUDA plus candle and ONNX sibling manifests"]
fn candle_fp16_vs_onnx_sibling_parity_manual_fsv() {
    let candle_manifest = std::env::var("CALYX_CANDLE_FSV_MANIFEST")
        .expect("CALYX_CANDLE_FSV_MANIFEST points to candle-fp16 manifest");
    let onnx_manifest = std::env::var("CALYX_CANDLE_FSV_ONNX_MANIFEST")
        .expect("CALYX_CANDLE_FSV_ONNX_MANIFEST points to onnx-int8 sibling manifest");
    let min_cosine: f32 = std::env::var("CALYX_CANDLE_FSV_PARITY_MIN_COSINE")
        .ok()
        .map(|raw| raw.parse().expect("parity threshold is f32"))
        .unwrap_or(0.70);

    let candle_spec = crate::lens_spec_from_manifest_path(&candle_manifest).unwrap();
    let onnx_spec = crate::lens_spec_from_manifest_path(&onnx_manifest).unwrap();
    let candle = CandleLens::from_lens_spec(&candle_spec).unwrap();
    let onnx = OnnxLens::from_lens_spec(&onnx_spec).unwrap();
    let probes = [
        "Calyx stores association-native measurements.",
        "A frozen lens should replay deterministically.",
        "The quick brown fox jumps over the lazy dog.",
    ];

    let mut cosines = Vec::with_capacity(probes.len());
    for probe in probes {
        let input = Input::new(Modality::Text, probe.as_bytes().to_vec());
        let candle_vector = dense(candle.measure(&input).unwrap()).1;
        let onnx_vector = dense(onnx.measure(&input).unwrap()).1;
        let score = cosine(&candle_vector, &onnx_vector);
        println!("CANDLE_ONNX_PARITY probe={probe:?} cosine={score:.8}");
        cosines.push(score);
    }

    let min_observed = cosines.iter().copied().fold(f32::INFINITY, f32::min);
    let mean = cosines.iter().sum::<f32>() / cosines.len() as f32;
    println!("CANDLE_ONNX_PARITY_CANDLE_MANIFEST={candle_manifest}");
    println!("CANDLE_ONNX_PARITY_ONNX_MANIFEST={onnx_manifest}");
    println!(
        "CANDLE_ONNX_PARITY_FINITE_REPLAY_DTYPE={}",
        candle
            .finite_replay_precision()
            .map(CandlePrecision::as_str)
            .unwrap_or("none")
    );
    println!("CANDLE_ONNX_PARITY_MIN={min_observed:.8}");
    println!("CANDLE_ONNX_PARITY_MEAN={mean:.8}");
    println!("CANDLE_ONNX_PARITY_THRESHOLD={min_cosine:.8}");
    assert!(min_observed >= min_cosine);
}

fn dense(vector: SlotVector) -> (u32, Vec<f32>) {
    let SlotVector::Dense { dim, data } = vector else {
        panic!("expected dense candle vector");
    };
    (dim, data)
}

fn norm(data: &[f32]) -> f32 {
    data.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let dot = left.iter().zip(right).map(|(l, r)| l * r).sum::<f32>();
    dot / (norm(left) * norm(right))
}
