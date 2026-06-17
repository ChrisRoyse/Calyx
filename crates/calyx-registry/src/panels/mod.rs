mod defaults;

use std::collections::BTreeMap;

use calyx_core::{
    Asymmetry, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState,
    content_address,
};
use serde::{Deserialize, Serialize};

pub use defaults::{
    bio_default, civic_default, code_default, legal_default, media_default, medical_default,
    text_default,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelSlotSpec {
    pub name: String,
    pub runtime: PanelLensRuntime,
    pub output: SlotShape,
    pub modality: Modality,
    pub retrieval_only: bool,
    pub excluded_from_dedup: bool,
    pub required: bool,
    pub asymmetry: Asymmetry,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelLensRuntime {
    Registry { name: String },
    TeiHttp { endpoint: String },
    Algorithmic { lens: AlgorithmicPanelLens },
    ExternalCmd { name: String },
    Placeholder { name: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlgorithmicPanelLens {
    ByteFeatures,
    AstStyle,
    SparseKeywords,
    TemporalRecent,
    TemporalPeriodic,
    TemporalPositional,
    Scalar,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelTemplate {
    pub name: String,
    pub slots: Vec<PanelSlotSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstantiatedPanel {
    pub template_name: String,
    pub panel: Panel,
    pub slot_specs: Vec<PanelSlotSpec>,
}

pub fn instantiate_panel(template: &PanelTemplate, created_at: u64) -> InstantiatedPanel {
    let slots = template
        .slots
        .iter()
        .enumerate()
        .map(|(idx, spec)| {
            let slot_id = SlotId::new(idx as u16);
            Slot {
                slot_id,
                slot_key: SlotKey::new(slot_id, spec.name.clone()),
                lens_id: slot_lens_id(&template.name, spec),
                shape: spec.output,
                modality: spec.modality,
                asymmetry: spec.asymmetry,
                quant: QuantPolicy::None,
                resource: Default::default(),
                axis: Some(spec.name.clone()),
                retrieval_only: spec.retrieval_only,
                excluded_from_dedup: spec.excluded_from_dedup,
                bits_about: BTreeMap::new(),
                state: SlotState::Active,
                added_at_panel_version: (idx + 1) as u32,
            }
        })
        .collect::<Vec<_>>();
    InstantiatedPanel {
        template_name: template.name.clone(),
        panel: Panel {
            version: slots.len() as u32,
            slots,
            created_at,
            kernel_ref: None,
            guard_ref: None,
        },
        slot_specs: template.slots.clone(),
    }
}

impl PanelTemplate {
    pub fn temporal_specs(&self) -> impl Iterator<Item = &PanelSlotSpec> {
        self.slots
            .iter()
            .filter(|slot| slot.retrieval_only || slot.excluded_from_dedup)
    }
}

impl PanelSlotSpec {
    pub fn content(
        name: impl Into<String>,
        runtime: PanelLensRuntime,
        output: SlotShape,
        modality: Modality,
    ) -> Self {
        Self {
            name: name.into(),
            runtime,
            output,
            modality,
            retrieval_only: false,
            excluded_from_dedup: false,
            required: true,
            asymmetry: Asymmetry::None,
        }
    }

    pub fn registry(
        name: impl Into<String>,
        registry_name: impl Into<String>,
        output: SlotShape,
        modality: Modality,
    ) -> Self {
        Self::content(
            name,
            PanelLensRuntime::Registry {
                name: registry_name.into(),
            },
            output,
            modality,
        )
    }

    pub fn temporal(
        name: impl Into<String>,
        lens: AlgorithmicPanelLens,
        output: SlotShape,
    ) -> Self {
        Self {
            name: name.into(),
            runtime: PanelLensRuntime::Algorithmic { lens },
            output,
            modality: Modality::Structured,
            retrieval_only: true,
            excluded_from_dedup: true,
            required: false,
            asymmetry: Asymmetry::None,
        }
    }

    pub fn with_asymmetry(mut self, asymmetry: Asymmetry) -> Self {
        self.asymmetry = asymmetry;
        self
    }
}

pub(super) fn slot_lens_id(template: &str, spec: &PanelSlotSpec) -> LensId {
    let spec_text = format!(
        "{template}:{}:{:?}:{:?}:{:?}:{}:{}",
        spec.name,
        spec.runtime,
        spec.output,
        spec.modality,
        spec.retrieval_only,
        spec.excluded_from_dedup
    );
    LensId::from_bytes(content_address([spec_text.as_bytes()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_default_instantiates_expected_temporal_tail() {
        let template = text_default();
        let panel = instantiate_panel(&template, 10);

        assert_eq!(template.slots.len(), 8);
        assert_eq!(panel.panel.version, 8);
        assert_eq!(panel.panel.slots[5].slot_key.key(), "E2_recency");
        assert!(template.slots[5..].iter().all(|slot| slot.retrieval_only));
        assert!(
            panel.panel.slots[5..]
                .iter()
                .all(|slot| slot.retrieval_only)
        );
        assert!(
            template.slots[5..]
                .iter()
                .all(|slot| slot.excluded_from_dedup)
        );
        assert!(
            panel.panel.slots[5..]
                .iter()
                .all(|slot| slot.excluded_from_dedup)
        );
    }

    #[test]
    fn all_default_panels_include_temporal_slots_last() {
        for template in [
            code_default(),
            civic_default(),
            media_default(),
            legal_default(),
            medical_default(),
            bio_default(),
        ] {
            let names = template
                .slots
                .iter()
                .map(|slot| slot.name.as_str())
                .collect::<Vec<_>>();
            assert!(names.ends_with(&["E2_recency", "E3_periodic", "E4_positional"]));
            assert_eq!(template.temporal_specs().count(), 3);
        }
        assert!(code_default().slots.len() >= 15);
        assert!(civic_default().slots.len() >= 24);
        assert!(media_default().slots.len() >= 10);
        assert!(legal_default().slots.len() >= 8);
        assert!(medical_default().slots.len() >= 6);
        assert!(bio_default().slots.len() >= 7);
    }

    #[test]
    fn media_default_matches_pinned_wavlm_speaker_dim() {
        let speaker = media_default()
            .slots
            .into_iter()
            .find(|slot| slot.name == "speaker_wavlm")
            .expect("media default should include WavLM speaker slot");

        assert_eq!(speaker.output, SlotShape::Dense(512));
        assert_eq!(speaker.modality, Modality::Audio);
    }

    #[test]
    fn media_default_matches_pinned_style_lens_contract() {
        let style = media_default()
            .slots
            .into_iter()
            .find(|slot| slot.name == "style_register")
            .expect("media default should include style register slot");

        assert_eq!(style.output, SlotShape::Dense(768));
        assert_eq!(style.modality, Modality::Text);
    }

    #[test]
    fn code_default_declares_ast_and_sparse_lexical_lenses() {
        let slots = code_default().slots;
        let ast = slots.iter().find(|slot| slot.name == "ast").unwrap();
        let lexical = slots
            .iter()
            .find(|slot| slot.name == "lexical_sparse")
            .unwrap();

        assert_eq!(ast.output, SlotShape::Dense(8));
        assert_eq!(ast.modality, Modality::Code);
        assert!(matches!(
            ast.runtime,
            PanelLensRuntime::Algorithmic {
                lens: AlgorithmicPanelLens::AstStyle
            }
        ));
        assert_eq!(lexical.output, SlotShape::Sparse(30_522));
        assert_eq!(lexical.modality, Modality::Code);
        assert!(matches!(
            lexical.runtime,
            PanelLensRuntime::Algorithmic {
                lens: AlgorithmicPanelLens::SparseKeywords
            }
        ));
    }
}
