use std::env;
use std::path::PathBuf;

use calyx_core::{LensId, Result};

use super::axis::MultimodalAxis;
use super::lens::{MultimodalAdapterLens, MultimodalAdapterSpec};
use crate::frozen::FrozenLensContract;
use crate::lens::Registry;
use crate::spec::LensSpec;

#[derive(Clone, Debug, PartialEq)]
pub struct MultimodalLensPackEntry {
    pub lens_id: LensId,
    pub spec: LensSpec,
    pub contract: FrozenLensContract,
}

pub fn default_multimodal_lens_specs() -> Vec<MultimodalAdapterSpec> {
    vec![
        spec(
            "image-siglip2-b16-adapter",
            MultimodalAxis::Image,
            "google/siglip2-base-patch16-224",
            768,
            "apache-2.0",
        ),
        spec(
            "audio-clap-htsat-adapter",
            MultimodalAxis::Audio,
            "laion/clap-htsat-unfused",
            512,
            "apache-2.0",
        ),
    ]
}

pub fn register_multimodal_lens_pack(
    registry: &mut Registry,
    specs: &[MultimodalAdapterSpec],
) -> Result<Vec<MultimodalLensPackEntry>> {
    let mut entries = Vec::with_capacity(specs.len());
    for spec in specs {
        let lens = MultimodalAdapterLens::from_adapter_spec(spec.clone())?;
        let contract = lens.contract();
        let lens_spec = lens.lens_spec();
        let lens_id =
            registry.register_frozen_with_spec(lens, contract.clone(), lens_spec.clone())?;
        entries.push(MultimodalLensPackEntry {
            lens_id,
            spec: lens_spec,
            contract,
        });
    }
    Ok(entries)
}

fn spec(
    name: &str,
    axis: MultimodalAxis,
    model_id: &str,
    dim: u32,
    license: &str,
) -> MultimodalAdapterSpec {
    let adapter_config = default_adapter_config_path(name);
    MultimodalAdapterSpec {
        name: name.to_string(),
        axis,
        model_id: model_id.to_string(),
        dim,
        license: Some(license.to_string()),
        allow_non_commercial: false,
        adapter_config: adapter_config.clone(),
        files: Vec::new(),
    }
}

fn default_adapter_config_path(name: &str) -> Option<PathBuf> {
    env::var_os("CALYX_HOME").map(PathBuf::from).map(|home| {
        home.join("lenses")
            .join(name)
            .join("onnx-int8")
            .join("adapter.json")
    })
}
