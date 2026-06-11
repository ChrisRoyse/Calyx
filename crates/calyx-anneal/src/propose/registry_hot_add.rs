use calyx_core::{Asymmetry, CalyxError, Constellation, Modality, Panel, QuantPolicy, Result, Ts};
use calyx_registry::{AlgorithmicLens, Registry, SlotSpec, SwapController};

use super::{
    CALYX_REGISTRY_HOT_ADD_FAIL, CandidateLens, HotAddAction, HotAddPlan, HotAddReceipt,
    LensHotAdder,
};
use crate::{ArtifactKey, ArtifactPtr};

pub struct RegistryHotAdder<'a> {
    registry: &'a mut Registry,
}

impl<'a> RegistryHotAdder<'a> {
    pub fn new(registry: &'a mut Registry) -> Self {
        Self { registry }
    }
}

impl LensHotAdder for RegistryHotAdder<'_> {
    fn plan_hot_add(
        &mut self,
        panel: &Panel,
        candidate: &CandidateLens,
        _corpus: &[Constellation],
    ) -> Result<HotAddPlan> {
        let candidate_hash = hash_candidate(candidate)?;
        let panel_hash = hash_panel(panel)?;
        Ok(HotAddPlan {
            artifact_key: ArtifactKey::ConfigCache(panel_hash),
            prior_ptr: ArtifactPtr::ConfigCacheKeyHash(panel_hash),
            candidate_ptr: ArtifactPtr::ConfigCacheKeyHash(hash_many([
                b"lens-proposal-candidate".as_slice(),
                &candidate_hash,
                &panel_hash,
            ])),
            candidate_action: HotAddAction::stable(),
            incumbent_action: HotAddAction::stable(),
            description: format!(
                "lens_proposal hot_add candidate_hash={}",
                hex32(candidate_hash)
            ),
        })
    }

    fn apply_hot_add(
        &mut self,
        controller: &mut SwapController,
        candidate: &CandidateLens,
        _corpus: &[Constellation],
        now: Ts,
    ) -> Result<HotAddReceipt> {
        let (lens, spec) = candidate_slot_spec(candidate)?;
        let contract = lens.contract().clone();
        if !self.registry.contains(spec.lens_id) {
            self.registry.register_frozen(lens, contract)?;
        }
        let outcome = controller.add_lens(self.registry, spec, [], now)?;
        Ok(HotAddReceipt {
            lens_id: outcome.slot.lens_id,
            panel_version: outcome.panel_version,
            slot_count: controller.panel().slots.len(),
        })
    }
}

fn candidate_slot_spec(candidate: &CandidateLens) -> Result<(AlgorithmicLens, SlotSpec)> {
    let (name, lens, modality, axis) = match candidate {
        CandidateLens::Algorithmic { kind, params } => {
            let name = format!("anneal-{}-{}", algorithmic_key(*kind), params.seed);
            let (lens, modality, axis) = match kind {
                super::AlgorithmicKind::Tfidf => (
                    AlgorithmicLens::byte_features(&name, Modality::Text),
                    Modality::Text,
                    Some("tfidf".to_string()),
                ),
                super::AlgorithmicKind::TimeLag => (
                    AlgorithmicLens::scalar(&name, Modality::Structured),
                    Modality::Structured,
                    Some("created_at".to_string()),
                ),
                super::AlgorithmicKind::FrequencyBand => (
                    AlgorithmicLens::scalar(&name, Modality::Structured),
                    Modality::Structured,
                    Some("periodic".to_string()),
                ),
                super::AlgorithmicKind::Pca => (
                    AlgorithmicLens::scalar(&name, Modality::Structured),
                    Modality::Structured,
                    Some("pca".to_string()),
                ),
            };
            (name, lens, modality, axis)
        }
        CandidateLens::Commission { .. } => {
            return Err(CalyxError {
                code: CALYX_REGISTRY_HOT_ADD_FAIL,
                message: "commissioned candidate has no frozen runtime artifact to hot-add yet"
                    .to_string(),
                remediation: "commission and freeze the lens artifact before registry hot-add",
            });
        }
    };
    let contract = lens.contract();
    let spec = SlotSpec {
        key: format!("anneal_{name}"),
        lens_id: contract.lens_id(),
        shape: contract.shape(),
        modality,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        axis,
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    Ok((lens, spec))
}

fn algorithmic_key(kind: super::AlgorithmicKind) -> &'static str {
    match kind {
        super::AlgorithmicKind::Pca => "pca",
        super::AlgorithmicKind::TimeLag => "time_lag",
        super::AlgorithmicKind::FrequencyBand => "frequency_band",
        super::AlgorithmicKind::Tfidf => "tfidf",
    }
}

fn hash_candidate(candidate: &CandidateLens) -> Result<[u8; 32]> {
    serde_json::to_vec(candidate)
        .map(|bytes| blake3::hash(&bytes).into())
        .map_err(|error| CalyxError {
            code: CALYX_REGISTRY_HOT_ADD_FAIL,
            message: format!("serialize candidate for hot-add hash failed: {error}"),
            remediation: "repair candidate serialization before registry hot-add",
        })
}

fn hash_panel(panel: &Panel) -> Result<[u8; 32]> {
    serde_json::to_vec(panel)
        .map(|bytes| blake3::hash(&bytes).into())
        .map_err(|error| CalyxError {
            code: CALYX_REGISTRY_HOT_ADD_FAIL,
            message: format!("serialize panel for hot-add hash failed: {error}"),
            remediation: "repair panel serialization before registry hot-add",
        })
}

fn hash_many<const N: usize>(parts: [&[u8]; N]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn hex32(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
