//! Durable panel-template catalog/object storage and runtime registration.

mod progress;
mod readback;
mod registration;
mod resident_registration;
mod storage_io;
mod store;
mod types;

#[cfg(test)]
mod tests;

#[cfg(test)]
use calyx_registry::Registry;

pub(super) use super::template_cards::ensemble_card_from_capability_cards;
pub(super) use super::template_model::{
    CATALOG_VERSION, MIN_CONTENT_LENSES, OBJECT_VERSION, PanelTemplateCatalog,
    PanelTemplateIndexEntry, PanelTemplateVersionRef, SavedPanelTemplate, TEMPLATE_INVALID,
    TEMPLATE_NOT_FOUND, TemplateDraft, TemplateEnsembleCard, TemplateLensRef,
    default_time_controls, id_for_loaded, lens_ref_from_catalog, object_bytes, template_error,
};
#[cfg(test)]
pub(super) use registration::register_template_lenses;
pub(super) use registration::register_template_lenses_with_progress;
pub(super) use resident_registration::ResidentSwapAttestation;
#[allow(unused_imports)]
pub(super) use types::{
    TemplateLensProgress, TemplateSave, TemplateStore, TemplateSummary, TemplateSwap,
};
