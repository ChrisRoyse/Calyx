use calyx_core::{Modality, Result};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};

use super::StructuredBinding;
use super::{AlgorithmicEncoder, LEGAL_DOCUMENT_STRUCTURE_POLICY, SPARSE_KEYWORDS_RAW_TF_POLICY};

pub(crate) fn algorithmic_contract(
    name: &str,
    modality: Modality,
    encoder: AlgorithmicEncoder,
) -> FrozenLensContract {
    if encoder == AlgorithmicEncoder::ByteFeatures {
        return FrozenLensContract::algorithmic_byte_features(name, modality);
    }
    let encoder_text = match encoder {
        AlgorithmicEncoder::LegalDocumentStructure => format!(
            "{encoder:?}:{}:{LEGAL_DOCUMENT_STRUCTURE_POLICY}",
            encoder.dim()
        ),
        AlgorithmicEncoder::SparseKeywordsRawTf { .. } => format!(
            "{encoder:?}:{}:{SPARSE_KEYWORDS_RAW_TF_POLICY}",
            encoder.dim()
        ),
        _ => format!("{encoder:?}:{}", encoder.dim()),
    };
    FrozenLensContract::new(
        name,
        sha256_digest(&[b"algorithmic-runtime-v2", encoder_text.as_bytes()]),
        sha256_digest(&[b"algorithmic-data-oblivious"]),
        encoder.shape(),
        modality,
        LensDType::F32,
        NormPolicy::None,
    )
}

pub(crate) fn structured_contract(
    name: &str,
    binding: &StructuredBinding,
) -> Result<FrozenLensContract> {
    let canonical = binding.canonical_bytes()?;
    Ok(FrozenLensContract::new(
        name,
        sha256_digest(&[b"structured-algorithmic-runtime-v1", &canonical]),
        sha256_digest(&[
            b"structured-record-schema-v1",
            binding.schema_id().as_bytes(),
        ]),
        binding.shape(),
        Modality::Structured,
        LensDType::F32,
        NormPolicy::None,
    ))
}
