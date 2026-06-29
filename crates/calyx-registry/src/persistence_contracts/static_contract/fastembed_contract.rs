use std::path::PathBuf;
use std::str::FromStr;

use calyx_core::{CalyxError, Modality, Result, SlotShape};
use fastembed::{
    Bgem3Embedding, Bgem3Model, RerankerModel, SparseModel, SparseTextEmbedding, TextRerank,
};

use super::{ensure_file, lens_config_invalid};
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::{FastembedBgem3Output, LensSpec};

pub(super) fn fastembed_sparse_contract(
    spec: &LensSpec,
    model_id: &str,
    files: &[PathBuf],
) -> Result<FrozenLensContract> {
    let model = sparse_model_from_name(model_id)?;
    let info = SparseTextEmbedding::get_model_info(&model);
    let shape = match model {
        SparseModel::SPLADEPPV1 => SlotShape::Sparse(30_522),
        SparseModel::BGEM3 => SlotShape::Sparse(250_002),
    };
    fastembed_contract(
        spec,
        files,
        shape,
        NormPolicy::Finite,
        &[b"fastembed-sparse-v1", info.model_code.as_bytes()],
    )
}

pub(super) fn fastembed_bgem3_contract(
    spec: &LensSpec,
    model_id: &str,
    files: &[PathBuf],
    output: FastembedBgem3Output,
) -> Result<FrozenLensContract> {
    let model = bgem3_model_from_name(model_id)?;
    let info = Bgem3Embedding::get_model_info(&model);
    let (shape, norm, token): (SlotShape, NormPolicy, &[u8]) = match output {
        FastembedBgem3Output::Dense => (SlotShape::Dense(1024), NormPolicy::unit(), b"dense"),
        FastembedBgem3Output::Sparse => (SlotShape::Sparse(250_002), NormPolicy::Finite, b"sparse"),
        FastembedBgem3Output::Colbert => (
            SlotShape::Multi { token_dim: 1024 },
            NormPolicy::Finite,
            b"colbert",
        ),
    };
    fastembed_contract(
        spec,
        files,
        shape,
        norm,
        &[b"fastembed-bgem3-v1", info.model_code.as_bytes(), token],
    )
}

pub(super) fn fastembed_reranker_contract(
    spec: &LensSpec,
    model_id: &str,
    files: &[PathBuf],
) -> Result<FrozenLensContract> {
    let model = reranker_model_from_name(model_id)?;
    let info = TextRerank::get_model_info(&model);
    fastembed_contract(
        spec,
        files,
        SlotShape::Dense(1),
        NormPolicy::Finite,
        &[b"fastembed-reranker-v1", info.model_code.as_bytes()],
    )
}

fn fastembed_contract(
    spec: &LensSpec,
    files: &[PathBuf],
    shape: SlotShape,
    norm: NormPolicy,
    corpus_parts: &[&[u8]],
) -> Result<FrozenLensContract> {
    if files.is_empty() {
        return Err(lens_config_invalid(format!(
            "fastembed lens {} has no persisted contract files",
            spec.name
        )));
    }
    for path in files {
        ensure_file("fastembed contract artifact", path)?;
    }
    Ok(FrozenLensContract::new(
        spec.name.clone(),
        spec.weights_sha256,
        sha256_digest(corpus_parts),
        shape,
        Modality::Text,
        LensDType::F32,
        norm,
    ))
}

fn sparse_model_from_name(raw: &str) -> Result<SparseModel> {
    SparseModel::from_str(raw.trim()).or_else(|_| match normalized(raw).as_str() {
        "baai/bge-m3" | "bge-m3" => Ok(SparseModel::BGEM3),
        "qdrant/splade_pp_en_v1" | "prithivida/splade_pp_en_v1" | "splade_pp_en_v1" => {
            Ok(SparseModel::SPLADEPPV1)
        }
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported fastembed sparse model {other}"
        ))),
    })
}

fn bgem3_model_from_name(raw: &str) -> Result<Bgem3Model> {
    Bgem3Model::from_str(raw.trim()).or_else(|_| match normalized(raw).as_str() {
        "baai/bge-m3" | "bge-m3" | "gpahal/bge-m3-onnx-int8" => Ok(Bgem3Model::BGEM3Q),
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported BGE-M3 fastembed model {other}"
        ))),
    })
}

fn reranker_model_from_name(raw: &str) -> Result<RerankerModel> {
    RerankerModel::from_str(raw.trim()).or_else(|_| match normalized(raw).as_str() {
        "baai/bge-reranker-v2-m3" | "bge-reranker-v2-m3" | "rozgo/bge-reranker-v2-m3" => {
            Ok(RerankerModel::BGERerankerV2M3)
        }
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported fastembed reranker model {other}"
        ))),
    })
}

fn normalized(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}
