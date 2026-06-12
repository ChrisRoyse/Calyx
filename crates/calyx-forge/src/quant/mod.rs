pub mod binary;
pub mod mxfp4_codec;
pub mod qjl;
pub mod rotation;
pub mod turboquant;

use std::fmt;

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::ForgeError;
use crate::Result;

pub use binary::{BinaryCodec, binary_prefilter, hamming_dot_estimate};
pub use mxfp4_codec::{AssayQuantSafety, MxFp4Codec};
pub use qjl::{QjlResidual, dot_estimate_unbiased, dot_qjl_correction, encode_qjl_residual};
pub use rotation::{
    CURRENT_SEED_VERSION, RotationSeed, apply_inverse_rotation, apply_rotation,
    apply_rotation_batch, new_seed, seed_id_hex,
};
pub use turboquant::TurboQuantCodec;

pub type SeedId = [u8; 32];

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum QuantLevel {
    F32,
    Bits8,
    Bits8Fp,
    Bits4Fp,
    Bits3p5,
    Bits2p5,
    Bits1,
}

impl QuantLevel {
    pub fn bits_per_channel(self) -> f32 {
        match self {
            Self::F32 => 32.0,
            Self::Bits8 => 8.0,
            Self::Bits8Fp => 8.0,
            Self::Bits4Fp => 4.0,
            Self::Bits3p5 => 3.5,
            Self::Bits2p5 => 2.5,
            Self::Bits1 => 1.0,
        }
    }

    pub fn is_lossy(self) -> bool {
        !matches!(self, Self::F32 | Self::Bits8)
    }
}

impl fmt::Display for QuantLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::F32 => "F32",
            Self::Bits8 => "Bits8",
            Self::Bits8Fp => "Bits8Fp",
            Self::Bits4Fp => "Bits4Fp",
            Self::Bits3p5 => "Bits3p5",
            Self::Bits2p5 => "Bits2p5",
            Self::Bits1 => "Bits1",
        };
        f.write_str(name)
    }
}

pub trait Quantizer: Send + Sync {
    fn encode(&self, vec: &[f32]) -> Result<QuantizedVec>;
    fn decode(&self, qv: &QuantizedVec) -> Result<Vec<f32>>;
    fn dot_estimate(&self, a: &QuantizedVec, b: &QuantizedVec) -> Result<f32>;
    fn level(&self) -> QuantLevel;
    fn dim(&self) -> usize;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QuantizedVec {
    pub level: QuantLevel,
    pub dim: usize,
    pub bytes: Vec<u8>,
    pub scale: f32,
    pub seed_id: SeedId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_seed(byte: u8) -> SeedId {
        [byte; 32]
    }

    fn sample_quantized_vec(bytes: Vec<u8>) -> QuantizedVec {
        QuantizedVec {
            level: QuantLevel::Bits3p5,
            dim: 4,
            bytes,
            scale: 0.125,
            seed_id: sample_seed(7),
        }
    }

    #[test]
    fn quant_level_bits_and_lossiness_are_declared() {
        assert_eq!(QuantLevel::Bits3p5.bits_per_channel(), 3.5);
        assert_eq!(QuantLevel::Bits8Fp.bits_per_channel(), 8.0);
        assert!(QuantLevel::Bits8Fp.is_lossy());
        assert!(QuantLevel::Bits1.is_lossy());
        println!(
            "QUANT_LEVEL_BITS PASSED Bits3p5={} Bits8Fp_lossy={} Bits1_lossy={}",
            QuantLevel::Bits3p5.bits_per_channel(),
            QuantLevel::Bits8Fp.is_lossy(),
            QuantLevel::Bits1.is_lossy()
        );
    }

    #[test]
    fn quantized_vec_serde_roundtrips() {
        let qv = sample_quantized_vec(vec![3, 1, 4, 1, 5]);
        let json = serde_json::to_string(&qv).expect("quantized vec json serialize");
        let restored: QuantizedVec =
            serde_json::from_str(&json).expect("quantized vec json deserialize");
        assert_eq!(qv, restored);
        println!("QUANTIZED_VEC_SERDE PASSED bytes={}", restored.bytes.len());
    }

    #[test]
    fn quant_level_all_variants_serde_roundtrip() {
        let levels = [
            QuantLevel::F32,
            QuantLevel::Bits8,
            QuantLevel::Bits8Fp,
            QuantLevel::Bits4Fp,
            QuantLevel::Bits3p5,
            QuantLevel::Bits2p5,
            QuantLevel::Bits1,
        ];
        for level in levels {
            let json = serde_json::to_string(&level).expect("quant level serialize");
            let restored: QuantLevel =
                serde_json::from_str(&json).expect("quant level deserialize");
            assert_eq!(level, restored);
        }
        println!("QUANT_LEVEL_SERDE PASSED variants={}", levels.len());
    }

    #[test]
    fn quant_edges_cover_f32_bits2p5_and_empty_bytes() {
        assert!(!QuantLevel::F32.is_lossy());
        assert_eq!(QuantLevel::Bits2p5.bits_per_channel(), 2.5);
        let empty = sample_quantized_vec(Vec::new());
        let json = serde_json::to_string(&empty).expect("empty bytes quantized vec serializes");
        assert!(json.contains("\"bytes\":[]"));
        println!(
            "QUANT_EDGES PASSED f32_lossy={} bits2p5={} empty_bytes={}",
            QuantLevel::F32.is_lossy(),
            QuantLevel::Bits2p5.bits_per_channel(),
            empty.bytes.len()
        );
    }

    #[test]
    fn quant_error_display_codes_are_fail_closed() {
        let quant = ForgeError::QuantError {
            op: "encode".to_string(),
            level: "Bits3p5".to_string(),
            detail: "non-finite rotated coefficient".to_string(),
            remediation: "Reject non-finite quantizer inputs before encoding".to_string(),
        };
        let seed = ForgeError::SeedVersionMismatch {
            expected: 1,
            got: 2,
        };
        println!("{quant}");
        println!("{seed}");
        assert!(quant.to_string().starts_with("CALYX_FORGE_QUANT_ERROR"));
        assert!(seed.to_string().contains("CALYX_FORGE_QUANT_SEED_VERSION"));
        println!("QUANT_ERROR_DISPLAY PASSED");
    }

    proptest! {
        #[test]
        fn quant_error_display_starts_with_code(
            op in ".{0,32}",
            level in ".{0,24}",
            detail in ".{0,96}",
            remediation in ".{0,96}",
        ) {
            let err = ForgeError::QuantError {
                op,
                level,
                detail,
                remediation,
            };
            prop_assert!(err.to_string().starts_with("CALYX_FORGE_QUANT_ERROR"));
        }
    }
}
