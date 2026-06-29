use calyx_core::{CalyxError, Lens, Panel, QuantPolicy, Result, SlotShape, SlotState};

use crate::frozen::FrozenLensContract;
use crate::spec::{LensRuntime, LensSpec, default_recall_delta};
use crate::{AlgorithmicLens, Registry};

use super::{
    AlgorithmicPanelLens, PanelLensRuntime, PanelSlotSpec, PanelTemplate, instantiate_panel,
};

#[derive(Clone)]
pub struct MaterializedPanelTemplate {
    pub template_name: String,
    pub panel: Panel,
    pub registry: Registry,
    pub registered_lenses_added: usize,
    pub inactive_unmaterialized_slots: Vec<String>,
}

pub fn materialize_panel_template(
    template: &PanelTemplate,
    created_at: u64,
) -> Result<MaterializedPanelTemplate> {
    let mut instantiated = instantiate_panel(template, created_at);
    let mut registry = Registry::new();
    let mut inactive_unmaterialized_slots = Vec::new();

    for (slot, spec) in instantiated.panel.slots.iter_mut().zip(&template.slots) {
        let Some(lens) = materialize_algorithmic_content_lens(template, spec)? else {
            slot.state = SlotState::Parked;
            inactive_unmaterialized_slots.push(spec.name.clone());
            continue;
        };
        let contract = lens.contract().clone();
        let runtime = algorithmic_runtime(&spec.runtime)
            .expect("materialize_algorithmic_content_lens only returns algorithmic lenses");
        let lens_spec = spec_from_contract(spec, runtime, &contract);
        let lens_id = registry.register_frozen_with_spec(lens, contract, lens_spec)?;
        slot.lens_id = lens_id;
        slot.quant = QuantPolicy::turboquant_default();
    }

    ensure_active_slots_are_registered(&template.name, &instantiated.panel, &registry)?;
    Ok(MaterializedPanelTemplate {
        template_name: instantiated.template_name,
        panel: instantiated.panel,
        registered_lenses_added: registry.lens_snapshots().len(),
        registry,
        inactive_unmaterialized_slots,
    })
}

fn materialize_algorithmic_content_lens(
    template: &PanelTemplate,
    spec: &PanelSlotSpec,
) -> Result<Option<AlgorithmicLens>> {
    let PanelLensRuntime::Algorithmic { lens } = &spec.runtime else {
        return Ok(None);
    };
    if spec.retrieval_only || spec.excluded_from_dedup {
        return Ok(None);
    }
    let materialized = match lens {
        AlgorithmicPanelLens::ByteFeatures => {
            AlgorithmicLens::byte_features(&spec.name, spec.modality)
        }
        AlgorithmicPanelLens::AstStyle => AlgorithmicLens::ast_style(&spec.name, spec.modality),
        AlgorithmicPanelLens::SparseKeywords => {
            AlgorithmicLens::sparse_keywords(&spec.name, spec.modality, sparse_dim(spec.output)?)
        }
        AlgorithmicPanelLens::Scalar => AlgorithmicLens::scalar(&spec.name, spec.modality),
        AlgorithmicPanelLens::TemporalRecent
        | AlgorithmicPanelLens::TemporalPeriodic
        | AlgorithmicPanelLens::TemporalPositional => {
            return Err(CalyxError::lens_frozen_violation(format!(
                "panel {} content slot {} uses temporal sidecar runtime {:?}",
                template.name, spec.name, lens
            )));
        }
    };
    ensure_lens_matches_slot(template, spec, &materialized)?;
    Ok(Some(materialized))
}

fn algorithmic_runtime(runtime: &PanelLensRuntime) -> Option<LensRuntime> {
    let PanelLensRuntime::Algorithmic { lens } = runtime else {
        return None;
    };
    let kind = match lens {
        AlgorithmicPanelLens::ByteFeatures => "byte-features".to_string(),
        AlgorithmicPanelLens::AstStyle => "ast-style".to_string(),
        AlgorithmicPanelLens::SparseKeywords => "sparse-keywords".to_string(),
        AlgorithmicPanelLens::Scalar => "scalar".to_string(),
        AlgorithmicPanelLens::TemporalRecent => "temporal-recent".to_string(),
        AlgorithmicPanelLens::TemporalPeriodic => "temporal-periodic".to_string(),
        AlgorithmicPanelLens::TemporalPositional => "temporal-positional".to_string(),
    };
    Some(LensRuntime::Algorithmic { kind })
}

fn ensure_lens_matches_slot(
    template: &PanelTemplate,
    spec: &PanelSlotSpec,
    lens: &dyn Lens,
) -> Result<()> {
    if lens.shape() != spec.output {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "panel {} slot {} declares shape {:?}, but materialized lens {} emits {:?}",
            template.name,
            spec.name,
            spec.output,
            lens.id(),
            lens.shape()
        )));
    }
    if lens.modality() != spec.modality {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "panel {} slot {} declares modality {:?}, but materialized lens {} accepts {:?}",
            template.name,
            spec.name,
            spec.modality,
            lens.id(),
            lens.modality()
        )));
    }
    Ok(())
}

fn ensure_active_slots_are_registered(
    template: &str,
    panel: &Panel,
    registry: &Registry,
) -> Result<()> {
    let missing = panel
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active && !registry.contains(slot.lens_id))
        .map(|slot| {
            format!(
                "slot={} key={} lens={}",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id
            )
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(CalyxError {
        code: crate::CALYX_PANEL_LENS_MISSING,
        message: format!(
            "built-in panel {template} has active slots without materialized registry entries: [{}]",
            missing.join("; ")
        ),
        remediation: "materialize local built-in lenses or park slots that require an external registry source before persisting the vault panel",
    })
}

fn spec_from_contract(
    spec: &PanelSlotSpec,
    runtime: LensRuntime,
    contract: &FrozenLensContract,
) -> LensSpec {
    LensSpec {
        name: spec.name.clone(),
        runtime,
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some(spec.name.clone()),
        asymmetry: spec.asymmetry,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: default_recall_delta(),
        retrieval_only: spec.retrieval_only,
        excluded_from_dedup: spec.excluded_from_dedup,
    }
}

fn sparse_dim(shape: SlotShape) -> Result<u32> {
    match shape {
        SlotShape::Sparse(dim) => Ok(dim),
        other => Err(CalyxError::lens_dim_mismatch(format!(
            "sparse keyword panel lens requires sparse output, got {other:?}"
        ))),
    }
}
