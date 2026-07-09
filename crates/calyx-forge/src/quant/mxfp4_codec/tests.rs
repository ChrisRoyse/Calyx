use super::*;
use proptest::prelude::*;

fn unit_vec(dim: usize) -> Vec<f32> {
    vec![1.0 / (dim as f32).sqrt(); dim]
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut aa = 0.0;
    let mut bb = 0.0;
    for (left, right) in a.iter().zip(b.iter()) {
        dot += left * right;
        aa += left * left;
        bb += right * right;
    }
    dot / (aa.sqrt() * bb.sqrt())
}

#[test]
fn mxfp4_codec_without_assay_evidence_fails_closed() -> Result<()> {
    let codec = MxFp4Codec::new(128);
    let error = codec
        .encode(&unit_vec(128))
        .expect_err("MXFP4 without assay evidence must fail closed");
    assert!(matches!(error, ForgeError::QuantIntelligenceLoss { .. }));
    println!(
        "mxfp4_codec_no_evidence_fail_closed PASSED code={} error={error}",
        error.code()
    );
    Ok(())
}

#[test]
fn mxfp4_codec_explicit_assay_safe_sets_bits4fp() -> Result<()> {
    let codec = MxFp4Codec::new(128);
    let original = unit_vec(128);
    let safety = passing_safety();
    let qv = codec.encode_assay_checked("slot:assay-safe", &original, &safety)?;
    assert_eq!(qv.level, QuantLevel::Bits4Fp);
    assert_eq!(qv.scale, 0.0);
    assert_eq!(qv.seed_id, ZERO_SEED);
    let decoded = codec.decode(&qv)?;
    let cos = cosine(&original, &decoded);
    assert!(cos >= 0.95, "cosine={cos}");
    println!(
        "mxfp4_codec_assay_safe PASSED cosine={cos:.6} bytes={}",
        qv.bytes.len()
    );
    Ok(())
}

#[test]
fn mxfp4_codec_assay_evidence_controls_slot_admission() -> Result<()> {
    let safe = AssayQuantSafety {
        baseline_bits: 1.0,
        quantized_bits: 0.97,
        cosine: 0.995,
        far_delta: 0.005,
    };
    let unsafe_evidence = AssayQuantSafety {
        baseline_bits: 1.0,
        quantized_bits: 0.70,
        cosine: 0.995,
        far_delta: 0.005,
    };
    let mut codec = MxFp4Codec::new(128);
    assert!(codec.record_assay_safety("slot:safe", safe));
    assert!(!codec.record_assay_safety("slot:unsafe", unsafe_evidence));

    let safe_qv = codec.encode_for_slot("slot:safe", &unit_vec(128))?;
    let unsafe_error = codec
        .encode_for_slot("slot:unsafe", &unit_vec(128))
        .expect_err("unsafe slot must not fall back to MXFP8");
    let explicit_mxfp8 = codec.encode_mxfp8(&unit_vec(128))?;

    assert_eq!(safe_qv.level, QuantLevel::Bits4Fp);
    assert_eq!(explicit_mxfp8.level, QuantLevel::Bits8Fp);
    assert!(matches!(
        unsafe_error,
        ForgeError::QuantIntelligenceLoss { .. }
    ));
    println!(
        "mxfp4_assay_evidence PASSED safe={:?} unsafe_error={} explicit_mxfp8={:?}",
        safe_qv.level,
        unsafe_error.code(),
        explicit_mxfp8.level
    );
    Ok(())
}

#[test]
fn mxfp8_explicit_roundtrip_cosine() -> Result<()> {
    let codec = MxFp4Codec::new(128);
    let original = unit_vec(128);
    let qv = codec.encode_mxfp8(&original)?;
    assert_eq!(qv.level, QuantLevel::Bits8Fp);
    let decoded = codec.decode(&qv)?;
    let cos = cosine(&original, &decoded);
    assert!(cos >= 0.99, "cosine={cos}");
    println!(
        "mxfp8_explicit_roundtrip PASSED cosine={cos:.6} dim={} bytes={}",
        decoded.len(),
        qv.bytes.len()
    );
    Ok(())
}

#[test]
fn mxfp8_explicit_edges_fail_closed_and_large_dim() -> Result<()> {
    let codec = MxFp4Codec::new(1536);
    let vec = unit_vec(1536);
    let qv = codec.encode_mxfp8(&vec)?;
    assert_eq!(codec.decode(&qv)?.len(), 1536);

    assert_eq!(qv.level, QuantLevel::Bits8Fp);
    let decoded = codec.decode(&qv)?;
    let cosine = cosine(&vec, &decoded);
    assert!(cosine >= 0.99, "cosine={cosine}");

    let mut corrupt = qv.clone();
    corrupt.bytes.push(1);
    let corrupt_err = codec
        .decode(&corrupt)
        .expect_err("corrupt byte length must fail closed");
    println!(
        "mxfp8_explicit PASSED level={:?} bits={} bytes={} cosine={cosine:.6} corrupt={corrupt_err}",
        qv.level,
        qv.level.bits_per_channel(),
        qv.bytes.len()
    );
    assert!(matches!(corrupt_err, ForgeError::QuantError { .. }));
    Ok(())
}

#[test]
fn mxfp_dot_estimate_matches_decoded_dot() -> Result<()> {
    let codec = MxFp4Codec::new(65);
    let safety = passing_safety();
    let left: Vec<f32> = (0..65).map(|idx| (idx as f32 - 32.0) / 16.0).collect();
    let right: Vec<f32> = (0..65).map(|idx| (32.0 - idx as f32) / 21.0).collect();

    let left_fp4 = codec.encode_assay_checked("slot:left", &left, &safety)?;
    let right_fp4 = codec.encode_assay_checked("slot:right", &right, &safety)?;
    assert_dot_matches_decoded(&codec, &left_fp4, &right_fp4)?;

    let left_fp8 = codec.encode_mxfp8(&left)?;
    let right_fp8 = codec.encode_mxfp8(&right)?;
    assert_dot_matches_decoded(&codec, &left_fp8, &right_fp8)?;
    assert_dot_matches_decoded(&codec, &left_fp4, &right_fp8)?;

    println!(
        "MXFP_DOT_ESTIMATE PASSED fp4_bytes={} fp8_bytes={}",
        left_fp4.bytes.len(),
        left_fp8.bytes.len()
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn mxfp4_codec_roundtrip_preserves_sign(values in proptest::collection::vec(-1.0f32..1.0, 128)) {
        let codec = MxFp4Codec::new(128);
        let safety = passing_safety();
        let qv = codec.encode_assay_checked("slot:proptest-safe", &values, &safety)?;
        let decoded = codec.decode(&qv)?;
        for (actual, expected) in decoded.iter().zip(values.iter()) {
            if *expected > 0.0 {
                prop_assert!(*actual > 0.0);
            } else if *expected < 0.0 {
                prop_assert!(*actual < 0.0);
            } else {
                prop_assert_eq!(*actual, 0.0);
            }
        }
    }
}

fn passing_safety() -> AssayQuantSafety {
    AssayQuantSafety {
        baseline_bits: 1.0,
        quantized_bits: 0.97,
        cosine: 0.995,
        far_delta: 0.005,
    }
}

fn assert_dot_matches_decoded(
    codec: &MxFp4Codec,
    left: &QuantizedVec,
    right: &QuantizedVec,
) -> Result<()> {
    let actual = codec.dot_estimate(left, right)?;
    let decoded_left = codec.decode(left)?;
    let decoded_right = codec.decode(right)?;
    let expected: f32 = decoded_left
        .iter()
        .zip(decoded_right.iter())
        .map(|(lhs, rhs)| lhs * rhs)
        .sum();
    assert!(
        (actual - expected).abs() <= 1.0e-6 * expected.abs().max(1.0),
        "actual={actual} expected={expected}"
    );
    Ok(())
}
