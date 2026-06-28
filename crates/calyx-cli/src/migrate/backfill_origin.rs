use calyx_core::{Constellation, SlotId};
use calyx_registry::{PanelLensRuntime, PanelSlotSpec};

pub(crate) const ORIGIN_LEARNED_TEI: &str = "learned_tei";
pub(crate) const ORIGIN_ALGORITHMIC: &str = "algorithmic";
pub(crate) const ORIGIN_OFFLINE_DETERMINISTIC: &str = "offline_deterministic";

pub(crate) fn slot_origin_key(slot: SlotId) -> String {
    format!("backfill_slot_{}_origin", slot.get())
}

pub(crate) fn slot_runtime_key(slot: SlotId) -> String {
    format!("backfill_slot_{}_runtime", slot.get())
}

pub(crate) fn runtime_kind(spec: &PanelSlotSpec) -> &'static str {
    match &spec.runtime {
        PanelLensRuntime::TeiHttp { .. } => "tei_http",
        PanelLensRuntime::Registry { .. } => "registry",
        PanelLensRuntime::Algorithmic { .. } => "algorithmic",
        PanelLensRuntime::ExternalCmd { .. } => "external_cmd",
        PanelLensRuntime::Placeholder { .. } => "placeholder",
    }
}

pub(crate) fn is_model_like_runtime(spec: &PanelSlotSpec) -> bool {
    matches!(
        spec.runtime,
        PanelLensRuntime::TeiHttp { .. }
            | PanelLensRuntime::Registry { .. }
            | PanelLensRuntime::ExternalCmd { .. }
            | PanelLensRuntime::Placeholder { .. }
    )
}

#[derive(Default)]
pub(crate) struct BackfillProvenanceAudit {
    pub missing: Vec<String>,
    pub invalid: Vec<String>,
    pub learned_tei_rows: usize,
    pub offline_model_rows: Vec<String>,
}

pub(crate) fn inspect_backfill_provenance(
    cx: &Constellation,
    slot: SlotId,
    spec: &PanelSlotSpec,
    row_ref: String,
    audit: &mut BackfillProvenanceAudit,
) {
    let Some(origin) = cx.metadata.get(&slot_origin_key(slot)) else {
        audit.missing.push(row_ref);
        return;
    };
    if let Some(runtime) = cx.metadata.get(&slot_runtime_key(slot)) {
        if runtime != runtime_kind(spec) {
            audit.invalid.push(format!(
                "{row_ref}:runtime={runtime}:expected={}",
                runtime_kind(spec)
            ));
        }
    } else {
        audit.missing.push(format!("{row_ref}:runtime"));
    }
    if !matches!(
        origin.as_str(),
        ORIGIN_LEARNED_TEI | ORIGIN_ALGORITHMIC | ORIGIN_OFFLINE_DETERMINISTIC
    ) {
        audit.invalid.push(format!("{row_ref}:origin={origin}"));
        return;
    }
    if is_model_like_runtime(spec) {
        match origin.as_str() {
            ORIGIN_LEARNED_TEI => audit.learned_tei_rows += 1,
            ORIGIN_OFFLINE_DETERMINISTIC => audit.offline_model_rows.push(row_ref),
            _ => audit.invalid.push(format!("{row_ref}:origin={origin}")),
        }
    }
}

pub(crate) fn learned_backfill_gate(
    require_backfill: bool,
    audit: &BackfillProvenanceAudit,
) -> String {
    if !require_backfill {
        return "NOT_REQUESTED".to_string();
    }
    if audit.missing.is_empty() && audit.invalid.is_empty() && audit.offline_model_rows.is_empty() {
        "PASS".to_string()
    } else {
        "FAIL".to_string()
    }
}
