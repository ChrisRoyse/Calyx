use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ForgeError;

pub type Result<T> = std::result::Result<T, ForgeError>;

/// Backend operations implemented by the Stage 2 `Backend` trait.
pub const FORGE_SHIPPED_BACKEND_OPS: &[&str] = &[
    "gemm",
    "cosine",
    "dot",
    "l2",
    "normalize",
    "topk",
    "device_info",
];

/// PRD-listed Forge operations that are intentionally not part of the Stage 2 trait yet.
pub const FORGE_DEFERRED_BACKEND_OPS: &[&str] = &[
    "knn",
    "histogram_nmi",
    "spmm_sparse_ops",
    "bilinear_cross_term",
    "graph_ops",
    "colbert_maxsim",
];

/// Exact CUDA `topk` is currently guaranteed only for global `k <= 1024`.
pub const CUDA_EXACT_TOPK_MAX_K: usize = 1024;

pub trait Backend: Send + Sync {
    fn gemm(
        &self,
        a: &[f32],
        b: &[f32],
        m: usize,
        k: usize,
        n: usize,
        out: &mut [f32],
    ) -> Result<()>;
    fn cosine(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()>;
    fn dot(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()>;
    fn l2(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()>;
    fn normalize(&self, vecs: &mut [f32], dim: usize) -> Result<()>;
    fn topk(&self, scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>>;
    fn device_info(&self) -> DeviceInfo;
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Cpu,
    Cuda,
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cpu => f.write_str("cpu"),
            Self::Cuda => f.write_str("cuda"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BestConfig {
    pub backend: BackendKind,
    pub tile_m: usize,
    pub tile_n: usize,
    pub tile_k: usize,
    pub extra: HashMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceInfo {
    pub kind: BackendKind,
    pub name: String,
    pub avx512: bool,
    pub vram_mib: Option<u64>,
}

impl Default for DeviceInfo {
    fn default() -> Self {
        Self {
            kind: BackendKind::Cpu,
            name: "cpu".to_string(),
            avx512: false,
            vram_mib: None,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use proptest::prelude::*;

    fn all_error_variants(op: String, detail: String, remediation: String) -> Vec<ForgeError> {
        vec![
            ForgeError::NumericalInvariant {
                op: op.clone(),
                detail: detail.clone(),
                remediation: remediation.clone(),
            },
            ForgeError::DeviceUnavailable {
                device: op.clone(),
                detail,
                remediation: remediation.clone(),
            },
            ForgeError::ShapeMismatch {
                expected: vec![1, 2, 3],
                got: vec![1, 3],
                remediation: remediation.clone(),
            },
            ForgeError::Unimplemented { op, remediation },
            ForgeError::QuantError {
                op: "quantize".to_string(),
                level: "Bits3p5".to_string(),
                detail: "invalid quantized payload".to_string(),
                remediation: "Reject malformed quantized vectors".to_string(),
            },
            ForgeError::QuantIntelligenceLoss {
                slot: "slot-text".to_string(),
                detail: "guard FAR regression".to_string(),
                remediation: "Use a gentler quantization level".to_string(),
            },
            ForgeError::CacheError {
                op: "load".to_string(),
                path: "/tmp/calyx-autotune.json".to_string(),
                detail: "malformed JSON".to_string(),
                remediation: "Discard the corrupt cache or rerun autotune".to_string(),
            },
            ForgeError::SeedVersionMismatch {
                expected: 1,
                got: 2,
            },
        ]
    }

    #[test]
    fn backend_contract_lists_shipped_and_deferred_ops() -> Result<()> {
        assert_eq!(
            FORGE_SHIPPED_BACKEND_OPS,
            [
                "gemm",
                "cosine",
                "dot",
                "l2",
                "normalize",
                "topk",
                "device_info"
            ]
        );
        assert_eq!(
            FORGE_DEFERRED_BACKEND_OPS,
            [
                "knn",
                "histogram_nmi",
                "spmm_sparse_ops",
                "bilinear_cross_term",
                "graph_ops",
                "colbert_maxsim"
            ]
        );
        assert_eq!(CUDA_EXACT_TOPK_MAX_K, 1024);

        let payload = serde_json::json!({
            "issue": 338,
            "crate": "calyx-forge",
            "trait": "Backend",
            "shipped_ops": FORGE_SHIPPED_BACKEND_OPS,
            "deferred_ops": FORGE_DEFERRED_BACKEND_OPS,
            "cuda_exact_topk_max_k": CUDA_EXACT_TOPK_MAX_K,
            "deferred_ops_issue": "https://github.com/ChrisRoyse/Calyx/issues/338"
        });
        if let Ok(root) = std::env::var("CALYX_FSV_ROOT") {
            let root = std::path::PathBuf::from(root);
            std::fs::create_dir_all(&root).expect("create CALYX_FSV_ROOT");
            let path = root.join("forge-backend-contract-readback.json");
            let bytes = serde_json::to_vec_pretty(&payload).expect("serialize backend contract");
            std::fs::write(&path, &bytes).expect("write backend contract readback");
            let readback = std::fs::read_to_string(&path).expect("read backend contract readback");
            let restored: serde_json::Value =
                serde_json::from_str(&readback).expect("parse backend contract readback");
            assert_eq!(restored, payload);
            println!(
                "FORGE_BACKEND_CONTRACT_READBACK path={} bytes={}",
                path.display(),
                readback.len()
            );
        }
        Ok(())
    }

    #[test]
    fn device_info_default_roundtrips_and_backend_displays() -> Result<()> {
        let info = DeviceInfo::default();
        let json = serde_json::to_string(&info).map_err(|err| ForgeError::Unimplemented {
            op: "serde_json::to_string".to_string(),
            remediation: err.to_string(),
        })?;
        let restored: DeviceInfo =
            serde_json::from_str(&json).map_err(|err| ForgeError::Unimplemented {
                op: "serde_json::from_str".to_string(),
                remediation: err.to_string(),
            })?;

        assert_eq!(info, restored);
        assert_eq!(BackendKind::Cpu.to_string(), "cpu");
        Ok(())
    }

    #[test]
    fn best_config_serializes_lowercase_backend_and_roundtrips() -> Result<()> {
        let config = BestConfig {
            backend: BackendKind::Cpu,
            tile_m: 64,
            tile_n: 32,
            tile_k: 16,
            extra: HashMap::from([("packing".to_string(), "row-major".to_string())]),
        };

        let json = serde_json::to_string(&config).map_err(|err| ForgeError::Unimplemented {
            op: "serde_json::to_string".to_string(),
            remediation: err.to_string(),
        })?;
        assert!(json.contains("\"backend\":\"cpu\""));

        let restored: BestConfig =
            serde_json::from_str(&json).map_err(|err| ForgeError::Unimplemented {
                op: "serde_json::from_str".to_string(),
                remediation: err.to_string(),
            })?;
        assert_eq!(config, restored);
        Ok(())
    }

    proptest! {
        #[test]
        fn forge_error_display_starts_with_catalog_prefix(
            op in ".{0,32}",
            detail in ".{0,96}",
            remediation in ".{0,96}"
        ) {
            for err in all_error_variants(op.clone(), detail.clone(), remediation.clone()) {
                let rendered = err.to_string();
                prop_assert!(
                    rendered.starts_with("CALYX_FORGE_")
                        || rendered.starts_with("CALYX_QUANT_INTELLIGENCE_LOSS")
                );
            }
        }
    }

    #[test]
    fn display_handles_documented_edge_cases() {
        let long_detail = "n".repeat(512);
        let errors = [
            ForgeError::ShapeMismatch {
                expected: vec![],
                got: vec![],
                remediation: "check input shape metadata".to_string(),
            },
            ForgeError::NumericalInvariant {
                op: "normalize".to_string(),
                detail: long_detail,
                remediation: "reject NaN/Inf before compute".to_string(),
            },
            ForgeError::DeviceUnavailable {
                device: "cuda:0".to_string(),
                detail: "driver init failed".to_string(),
                remediation: "read nvidia-smi\ncheck CUDA_VISIBLE_DEVICES".to_string(),
            },
            ForgeError::QuantIntelligenceLoss {
                slot: "kernel".to_string(),
                detail: "kernel recall regressed".to_string(),
                remediation: "use a gentler quant level".to_string(),
            },
        ];

        for err in errors {
            let rendered = err.to_string();
            assert!(
                rendered.starts_with("CALYX_FORGE_")
                    || rendered.starts_with("CALYX_QUANT_INTELLIGENCE_LOSS")
            );
        }
    }

    #[test]
    fn fail_closed_error_codes_are_literal_first_tokens() {
        let numerical = ForgeError::NumericalInvariant {
            op: "cosine".to_string(),
            detail: "NaN score".to_string(),
            remediation: "fail closed and inspect input vectors".to_string(),
        };
        let device = ForgeError::DeviceUnavailable {
            device: "cuda:0".to_string(),
            detail: "no compatible device".to_string(),
            remediation: "surface device error instead of silent fallback".to_string(),
        };

        println!("{numerical}");
        println!("{device}");
        assert!(
            numerical
                .to_string()
                .starts_with("CALYX_FORGE_NUMERICAL_INVARIANT")
        );
        assert!(
            device
                .to_string()
                .starts_with("CALYX_FORGE_DEVICE_UNAVAILABLE")
        );
        let loss = ForgeError::QuantIntelligenceLoss {
            slot: "slot-text".to_string(),
            detail: "bits delta below bound".to_string(),
            remediation: "try Bits3p5 instead of Bits2p5".to_string(),
        };
        println!("{loss}");
        assert!(
            loss.to_string()
                .starts_with("CALYX_QUANT_INTELLIGENCE_LOSS")
        );
    }
}
