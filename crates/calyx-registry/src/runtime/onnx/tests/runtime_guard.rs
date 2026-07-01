use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use calyx_core::Lens;
use serde_json::{Value, json};

use super::super::*;

static ORT_ENV_LOCK: Mutex<()> = Mutex::new(());
const ORT_DYLIB_PATH: &str = "ORT_DYLIB_PATH";
const CALYX_ORT_CAPI: &str = "CALYX_ORT_CAPI";

#[test]
fn custom_onnx_missing_runtime_fails_closed_fast_before_model_fixtures() {
    let _lock = ORT_ENV_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!(
        "calyx-custom-onnx-runtime-guard-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let missing_ort = root.join("missing-onnxruntime.dll");
    let ort_dir = root.join("ort-dir");
    fs::create_dir_all(&ort_dir).unwrap();
    let cases = [
        ("unset", None),
        ("missing_file", Some(missing_ort.as_path())),
        ("directory", Some(ort_dir.as_path())),
    ];
    let mut report = Vec::new();
    for (name, ort_path) in cases {
        let _env = EnvGuard::set_path(ORT_DYLIB_PATH, ort_path);
        let spec = missing_runtime_spec(&root, name);
        let before = model_file_state(&spec);
        let error = lens_error(OnnxLens::from_files(spec.clone()));
        let after = model_file_state(&spec);
        assert_eq!(error.code, "CALYX_LENS_UNREACHABLE", "{name}");
        assert_eq!(before, after, "{name} should not touch model files");
        report.push(json!({
            "case": name,
            "ort_env": std::env::var_os(ORT_DYLIB_PATH).map(|value| value.to_string_lossy().to_string()),
            "before": before,
            "after": after,
            "error_code": error.code,
            "error_message": error.message,
        }));
    }
    maybe_write_fsv_json(
        "custom-onnx-runtime-guard.json",
        &json!({
            "source_of_truth": "ORT_DYLIB_PATH environment plus filesystem existence of model/tokenizer/config paths before and after OnnxLens::from_files",
            "cases": report,
        }),
    );
}

#[test]
fn dynamic_ort_uses_calyx_ort_capi_as_the_runtime_source_of_truth() {
    let _lock = ORT_ENV_LOCK.lock().unwrap();
    let root = std::env::temp_dir().join(format!("calyx-dynamic-ort-capi-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let capi = root.join("onnxruntime").join("capi");
    fs::create_dir_all(&capi).unwrap();
    let ort = capi.join("onnxruntime.dll");
    fs::write(&ort, b"real path contract").unwrap();

    let _ort_env = EnvGuard::set_path(ORT_DYLIB_PATH, None);
    let _capi_env = EnvGuard::set_path(CALYX_ORT_CAPI, Some(&capi));
    let before = json!({
        "ort_env": std::env::var_os(ORT_DYLIB_PATH).map(|value| value.to_string_lossy().to_string()),
        "capi_env": std::env::var_os(CALYX_ORT_CAPI).map(|value| value.to_string_lossy().to_string()),
        "ort_file_exists": ort.exists(),
    });

    let resolved =
        super::super::dynamic_ort::ensure_dynamic_ort(OnnxProviderPolicy::CpuExplicit).unwrap();

    let after = json!({
        "resolved": resolved.display().to_string(),
        "ort_env": std::env::var_os(ORT_DYLIB_PATH).map(|value| value.to_string_lossy().to_string()),
        "capi_env": std::env::var_os(CALYX_ORT_CAPI).map(|value| value.to_string_lossy().to_string()),
        "ort_file_exists": ort.exists(),
    });
    assert_eq!(resolved, ort);
    assert_eq!(std::env::var_os(ORT_DYLIB_PATH), Some(ort.clone().into()));
    assert!(ort.is_file());

    maybe_write_fsv_json(
        "dynamic-ort-capi-source-of-truth.json",
        &json!({
            "source_of_truth": "CALYX_ORT_CAPI filesystem directory plus ORT_DYLIB_PATH process environment after resolve_ort_dylib_path",
            "before": before,
            "after": after,
        }),
    );
}

struct EnvGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvGuard {
    fn set_path(key: &'static str, path: Option<&Path>) -> Self {
        let old = std::env::var_os(key);
        unsafe {
            match path {
                Some(path) => std::env::set_var(key, path),
                None => std::env::remove_var(key),
            }
        }
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.old {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn missing_runtime_spec(root: &Path, name: &str) -> OnnxFileSpec {
    let root = root.join(name);
    OnnxFileSpec::text(
        format!("custom-runtime-guard-{name}"),
        "calyx-test-custom-onnx",
        root.join("model.onnx"),
        root.join("tokenizer.json"),
        root.join("config.json"),
        PoolingPolicy::Mean,
        NormPolicy::unit(),
    )
    .with_provider_policy(OnnxProviderPolicy::CpuExplicit)
}

fn model_file_state(spec: &OnnxFileSpec) -> Value {
    json!({
        "model": {
            "path": spec.model_file.display().to_string(),
            "exists": spec.model_file.exists(),
        },
        "tokenizer": {
            "path": spec.tokenizer.display().to_string(),
            "exists": spec.tokenizer.exists(),
        },
        "config": {
            "path": spec.config.display().to_string(),
            "exists": spec.config.exists(),
        },
    })
}

fn maybe_write_fsv_json(name: &str, value: &Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).expect("create custom ONNX FSV root");
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).expect("custom ONNX FSV json"),
    )
    .expect("write custom ONNX FSV json");
}

fn lens_error(result: Result<OnnxLens>) -> calyx_core::CalyxError {
    match result {
        Ok(lens) => panic!("expected error, got lens {}", lens.id()),
        Err(error) => error,
    }
}
