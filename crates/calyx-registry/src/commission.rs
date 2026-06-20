use std::path::{Path, PathBuf};

use calyx_core::{Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::spec::{LensRuntime, LensSpec};
use crate::{Registry, ensure_input_modality};

mod algorithmic_manifest;
mod manifest;
mod manifest_metadata;
mod manifest_runtime;
#[cfg(test)]
mod tests;

pub use manifest::{
    LensForgeFile, LensForgeManifest, LensForgeShape, lens_spec_from_manifest,
    lens_spec_from_manifest_path, lens_spec_from_manifest_with_license_override,
};
pub use manifest_metadata::{
    lens_spec_metadata_from_manifest, lens_spec_metadata_from_manifest_path,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommissionRequest {
    pub name: String,
    pub base_model: String,
    pub corpus: Vec<Vec<u8>>,
    pub output_dim: u32,
    pub modality: Modality,
    pub axis: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommissionedLensArtifact {
    pub lens_id: LensId,
    pub artifact_path: PathBuf,
    pub weights_sha256: [u8; 32],
    pub corpus_hash: [u8; 32],
    pub contract: FrozenLensContract,
    pub spec: LensSpec,
}

#[derive(Clone, Debug)]
pub struct CommissionedLens {
    artifact: CommissionedLensArtifact,
}

pub fn commission_lens(
    request: &CommissionRequest,
    artifact_dir: &Path,
) -> Result<CommissionedLensArtifact> {
    std::fs::create_dir_all(artifact_dir).map_err(|err| {
        calyx_core::CalyxError::lens_unreachable(format!(
            "create artifact dir {} failed: {err}",
            artifact_dir.display()
        ))
    })?;
    let corpus_parts = request.corpus.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let axis = request.axis.clone().unwrap_or_else(|| request.name.clone());
    let corpus_hash = sha256_digest(&corpus_parts);
    let weights_sha256 = sha256_digest(&[
        b"commissioned-lens-v1",
        request.base_model.as_bytes(),
        &corpus_hash,
    ]);
    let contract = FrozenLensContract::new(
        request.name.clone(),
        weights_sha256,
        corpus_hash,
        SlotShape::Dense(request.output_dim),
        request.modality,
        LensDType::F32,
        NormPolicy::None,
    );
    let lens_id = contract.lens_id();
    let artifact_path = artifact_dir.join(format!("{lens_id}.commissioned.json"));
    let artifact_bytes = serde_json::to_vec(&(
        &request.name,
        &request.base_model,
        request.output_dim,
        &weights_sha256,
        &corpus_hash,
    ))
    .map_err(|err| {
        calyx_core::CalyxError::lens_unreachable(format!("encode artifact failed: {err}"))
    })?;
    std::fs::write(&artifact_path, artifact_bytes).map_err(|err| {
        calyx_core::CalyxError::lens_unreachable(format!(
            "write artifact {} failed: {err}",
            artifact_path.display()
        ))
    })?;
    let spec = LensSpec {
        name: request.name.clone(),
        runtime: LensRuntime::Algorithmic {
            kind: format!("commissioned:{}", request.base_model),
        },
        output: SlotShape::Dense(request.output_dim),
        modality: request.modality,
        weights_sha256,
        corpus_hash,
        norm_policy: NormPolicy::None,
        max_batch: None,
        axis: Some(axis),
        asymmetry: calyx_core::Asymmetry::None,
        quant_default: calyx_core::QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    Ok(CommissionedLensArtifact {
        lens_id,
        artifact_path,
        weights_sha256,
        corpus_hash,
        contract,
        spec,
    })
}

pub fn register_commissioned(
    registry: &mut Registry,
    artifact: CommissionedLensArtifact,
) -> Result<LensId> {
    let lens = CommissionedLens {
        artifact: artifact.clone(),
    };
    registry.register_frozen_with_spec(lens, artifact.contract.clone(), artifact.spec)
}

impl Lens for CommissionedLens {
    fn id(&self) -> LensId {
        self.artifact.lens_id
    }

    fn shape(&self) -> SlotShape {
        self.artifact.contract.shape()
    }

    fn modality(&self) -> Modality {
        self.artifact.contract.modality()
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        ensure_input_modality(self, input)?;
        let dim = match self.shape() {
            SlotShape::Dense(dim) => dim,
            _ => 0,
        };
        let seed = sha256_digest(&[
            &self.artifact.weights_sha256,
            input.bytes.as_slice(),
            self.artifact.lens_id.as_bytes(),
        ]);
        let data = (0..dim)
            .map(|idx| f32::from(seed[idx as usize % seed.len()]) / 255.0)
            .collect::<Vec<_>>();
        Ok(SlotVector::Dense { dim, data })
    }
}
