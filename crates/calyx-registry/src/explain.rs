use calyx_core::{LensId, Result};
use serde::{Deserialize, Serialize};

use crate::profile::{CapabilityCard, ProfileProbe, profile_lens};
use crate::{LensRuntime, Registry};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensExplanation {
    pub lens_id: LensId,
    pub corpus_hash: String,
    pub axis: Option<String>,
    pub runtime: Option<LensRuntime>,
    pub bits: String,
    pub redundancy: String,
    pub cost_ms_per_input: f32,
    pub vram_bytes: u64,
}

pub fn explain_lens(
    registry: &Registry,
    lens_id: LensId,
    probes: &[ProfileProbe],
) -> Result<LensExplanation> {
    let card = profile_lens(registry, lens_id, probes)?;
    explain_lens_from_card(registry, lens_id, &card)
}

pub fn explain_lens_from_card(
    registry: &Registry,
    lens_id: LensId,
    card: &CapabilityCard,
) -> Result<LensExplanation> {
    let spec = registry.lens_spec(lens_id);
    let corpus_hash = spec
        .map(|spec| hex32(&spec.corpus_hash))
        .or_else(|| {
            registry
                .frozen_contract(lens_id)
                .map(|contract| hex32(&contract.corpus_hash()))
        })
        .unwrap_or_else(|| "unknown".to_string());
    Ok(LensExplanation {
        lens_id,
        corpus_hash,
        axis: spec.and_then(|spec| spec.axis.clone()),
        runtime: spec.map(|spec| spec.runtime.clone()),
        bits: "provisional (Assay report not attached)".to_string(),
        redundancy: "provisional (Assay report not attached)".to_string(),
        cost_ms_per_input: card.cost.ms_per_input,
        vram_bytes: card.cost.vram_bytes,
    })
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
