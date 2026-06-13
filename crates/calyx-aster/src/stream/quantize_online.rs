//! On-the-fly slot quantization for the streaming ingest pipeline.
//!
//! Each streamed slot vector is quantized through TurboQuant using a rotation
//! seed that is *content-addressed* per `LensId + CxId` — `blake3(lens_id ||
//! cx_id)` — and never random (A25). The seed is therefore reproducible across
//! restarts: the same `(LensId, CxId, level, raw)` always yields byte-identical
//! quantized output, which is the property the FSV re-run/`xxd`-diff gate checks.

use calyx_core::{CalyxError, CxId, LensId};
use calyx_forge::ForgeError;
use calyx_forge::quant::{QuantLevel, QuantizedVec, Quantizer, TurboQuantCodec, new_seed};

/// Module-local error code: a non-finite coefficient reached the on-the-fly
/// quantizer. Fail-closed *before* quantization; the event is never written.
pub const CALYX_FORGE_INPUT_NAN: &str = "CALYX_FORGE_INPUT_NAN";

const INPUT_NAN_REMEDIATION: &str =
    "reject NaN/Inf slot vectors before on-the-fly quantization; re-emit finite coefficients";

const FORGE_FAULT_REMEDIATION: &str = "fail-closed on Forge quantization fault: reject the input or re-encode with the current rotation seed";

/// Builds a `CALYX_FORGE_INPUT_NAN` error.
pub(crate) fn input_nan_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_FORGE_INPUT_NAN,
        message: message.into(),
        remediation: INPUT_NAN_REMEDIATION,
    }
}

/// Maps a Forge fault into a Calyx error, preserving the stable Forge code.
pub(crate) fn forge_to_calyx(err: ForgeError) -> CalyxError {
    CalyxError {
        code: err.code(),
        message: err.to_string(),
        remediation: FORGE_FAULT_REMEDIATION,
    }
}

/// Configuration for on-the-fly slot quantization.
#[derive(Clone, Debug, PartialEq)]
pub struct QuantizeOnlineConfig {
    /// Lens identity mixed into the content-addressed rotation seed (A25).
    pub lens_id: LensId,
    /// TurboQuant level (must be `Bits3p5` or `Bits2p5`).
    pub level: QuantLevel,
}

impl QuantizeOnlineConfig {
    /// Creates a config for `lens_id` at `level`.
    pub fn new(lens_id: LensId, level: QuantLevel) -> Self {
        Self { lens_id, level }
    }
}

/// Content-addressed rotation-seed entropy: `blake3(lens_id || cx_id)`.
///
/// Deterministic across restarts; never random (A25).
pub fn rotation_seed_entropy(lens_id: LensId, cx_id: CxId) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(lens_id.as_bytes());
    hasher.update(cx_id.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Quantizes one slot vector on the fly.
///
/// Fails closed with `CALYX_FORGE_INPUT_NAN` if any coefficient is non-finite —
/// before deriving a seed or touching Forge. Otherwise derives the
/// content-addressed seed for `(lens_id, cx_id)` and TurboQuant-encodes `raw`.
pub fn quantize_slot_online(
    raw: &[f32],
    config: &QuantizeOnlineConfig,
    cx_id: CxId,
) -> Result<QuantizedVec, CalyxError> {
    if let Some(idx) = raw.iter().position(|value| !value.is_finite()) {
        return Err(input_nan_error(format!(
            "non-finite slot coefficient at index {idx}"
        )));
    }
    if raw.is_empty() {
        return Err(input_nan_error("slot vector is empty; nothing to quantize"));
    }
    let entropy = rotation_seed_entropy(config.lens_id, cx_id);
    let seed = new_seed(raw.len(), &entropy);
    let codec = TurboQuantCodec::new(seed, config.level).map_err(forge_to_calyx)?;
    codec.encode(raw).map_err(forge_to_calyx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn cx(byte: u8) -> CxId {
        CxId::from_bytes([byte; 16])
    }

    fn lens(byte: u8) -> LensId {
        LensId::from_bytes([byte; 16])
    }

    fn config() -> QuantizeOnlineConfig {
        QuantizeOnlineConfig::new(lens(0xAB), QuantLevel::Bits3p5)
    }

    #[test]
    fn seed_entropy_is_content_addressed_and_deterministic() {
        let a = rotation_seed_entropy(lens(1), cx(2));
        let b = rotation_seed_entropy(lens(1), cx(2));
        assert_eq!(a, b, "same (lens, cx) -> identical entropy (A25)");
        assert_ne!(
            a,
            rotation_seed_entropy(lens(9), cx(2)),
            "different lens -> different entropy"
        );
        assert_ne!(
            a,
            rotation_seed_entropy(lens(1), cx(9)),
            "different cx -> different entropy"
        );
    }

    #[test]
    fn fixed_input_quantizes_to_bit_identical_output_on_rerun() {
        let cfg = config();
        let raw = vec![1.0_f32; 128];
        let first = quantize_slot_online(&raw, &cfg, cx(7)).expect("first encode");
        let second = quantize_slot_online(&raw, &cfg, cx(7)).expect("second encode");
        assert_eq!(first.dim, 128);
        assert_eq!(
            first.bytes, second.bytes,
            "same seed -> bit-identical (A25)"
        );
        assert_eq!(first.seed_id, second.seed_id);
    }

    #[test]
    fn different_cx_yields_different_seed_id() {
        let cfg = config();
        let raw = vec![0.5_f32; 64];
        let a = quantize_slot_online(&raw, &cfg, cx(1)).expect("encode a");
        let b = quantize_slot_online(&raw, &cfg, cx(2)).expect("encode b");
        assert_ne!(a.seed_id, b.seed_id, "cx is mixed into the seed");
    }

    #[test]
    fn nan_input_fails_closed_before_quantization() {
        let cfg = config();
        let mut raw = vec![1.0_f32; 16];
        raw[5] = f32::NAN;
        let err = quantize_slot_online(&raw, &cfg, cx(3)).expect_err("NaN must fail closed");
        assert_eq!(err.code, CALYX_FORGE_INPUT_NAN);
    }

    #[test]
    fn infinite_input_fails_closed() {
        let cfg = config();
        let mut raw = vec![1.0_f32; 16];
        raw[0] = f32::INFINITY;
        let err = quantize_slot_online(&raw, &cfg, cx(3)).expect_err("Inf must fail closed");
        assert_eq!(err.code, CALYX_FORGE_INPUT_NAN);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn finite_inputs_round_to_same_dim_and_are_deterministic(
            len in 64usize..=512,
            seed_byte in any::<u8>(),
        ) {
            let cfg = config();
            let raw: Vec<f32> = (0..len)
                .map(|i| ((i as f32) * 0.013 - 1.7).sin())
                .collect();
            let cx_id = cx(seed_byte);
            let first = quantize_slot_online(&raw, &cfg, cx_id).expect("encode");
            prop_assert_eq!(first.dim, len);
            let second = quantize_slot_online(&raw, &cfg, cx_id).expect("re-encode");
            prop_assert_eq!(&first.bytes, &second.bytes);

            // Decoded values must all be finite.
            let entropy = rotation_seed_entropy(cfg.lens_id, cx_id);
            let codec = TurboQuantCodec::new(new_seed(len, &entropy), cfg.level).expect("codec");
            let decoded = codec.decode(&first).expect("decode");
            prop_assert_eq!(decoded.len(), len);
            prop_assert!(decoded.iter().all(|v| v.is_finite()));
        }
    }
}
