use std::str::FromStr;

use calyx_core::{CalyxError, Result, SlotShape};
use fastembed::{Bgem3Model, RerankerModel, SparseModel};

use crate::frozen::NormPolicy;
use crate::spec::FastembedBgem3Output;

pub(super) const BGE_M3_DENSE_DIM: u32 = 1024;
pub(super) const BGE_M3_SPARSE_DIM: u32 = 250_002;
const SPLADE_DIM: u32 = 30_522;

pub(super) fn sparse_dim(model: &SparseModel) -> u32 {
    match model {
        SparseModel::SPLADEPPV1 => SPLADE_DIM,
        SparseModel::BGEM3 => BGE_M3_SPARSE_DIM,
    }
}

pub(super) fn bgem3_shape(output: FastembedBgem3Output) -> SlotShape {
    match output {
        FastembedBgem3Output::Dense => SlotShape::Dense(BGE_M3_DENSE_DIM),
        FastembedBgem3Output::Sparse => SlotShape::Sparse(BGE_M3_SPARSE_DIM),
        FastembedBgem3Output::Colbert => SlotShape::Multi {
            token_dim: BGE_M3_DENSE_DIM,
        },
    }
}

pub(super) fn bgem3_norm(output: FastembedBgem3Output) -> NormPolicy {
    match output {
        FastembedBgem3Output::Dense => NormPolicy::unit(),
        FastembedBgem3Output::Sparse | FastembedBgem3Output::Colbert => NormPolicy::Finite,
    }
}

pub(super) fn bgem3_corpus_token(output: FastembedBgem3Output) -> &'static [u8] {
    match output {
        FastembedBgem3Output::Dense => b"dense",
        FastembedBgem3Output::Sparse => b"sparse",
        FastembedBgem3Output::Colbert => b"colbert",
    }
}

pub(super) fn bgem3_runtime_name(output: FastembedBgem3Output) -> &'static str {
    match output {
        FastembedBgem3Output::Dense => "fastembed-bgem3-dense",
        FastembedBgem3Output::Sparse => "fastembed-bgem3-sparse",
        FastembedBgem3Output::Colbert => "fastembed-bgem3-colbert",
    }
}

pub(super) fn sparse_model_from_name(raw: &str) -> Result<SparseModel> {
    if let Ok(model) = SparseModel::from_str(raw.trim()) {
        return Ok(model);
    }
    match normalized(raw).as_str() {
        "baai/bge-m3" | "bge-m3" => Ok(SparseModel::BGEM3),
        "qdrant/splade_pp_en_v1" | "prithivida/splade_pp_en_v1" | "splade_pp_en_v1" => {
            Ok(SparseModel::SPLADEPPV1)
        }
        "naver/splade-v3" | "splade-v3" => Err(CalyxError::lens_unreachable(
            "SPLADE-v3 requires a dedicated ONNX SPLADE-v3 runtime; fastembed 5.16 exposes SPLADE++ v1 only",
        )),
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported fastembed sparse model {other}"
        ))),
    }
}

pub(super) fn bgem3_model_from_name(raw: &str) -> Result<Bgem3Model> {
    if let Ok(model) = Bgem3Model::from_str(raw.trim()) {
        return Ok(model);
    }
    match normalized(raw).as_str() {
        "baai/bge-m3" | "bge-m3" | "gpahal/bge-m3-onnx-int8" => Ok(Bgem3Model::BGEM3Q),
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported BGE-M3 fastembed model {other}"
        ))),
    }
}

pub(super) fn reranker_model_from_name(raw: &str) -> Result<RerankerModel> {
    if let Ok(model) = RerankerModel::from_str(raw.trim()) {
        return Ok(model);
    }
    match normalized(raw).as_str() {
        "baai/bge-reranker-v2-m3" | "bge-reranker-v2-m3" | "rozgo/bge-reranker-v2-m3" => {
            Ok(RerankerModel::BGERerankerV2M3)
        }
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported fastembed reranker model {other}"
        ))),
    }
}

fn normalized(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}
