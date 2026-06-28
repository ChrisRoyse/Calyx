use super::*;

pub(super) fn emit_progress(
    progress: &mut Option<&mut dyn FnMut(TemplateLensProgress) -> CliResult<()>>,
    event: TemplateLensProgress,
) -> CliResult<()> {
    if let Some(progress) = progress.as_deref_mut() {
        progress(event)?;
    }
    Ok(())
}

pub(super) fn lens_progress(
    phase: &'static str,
    idx: usize,
    total: usize,
    lens: &TemplateLensRef,
) -> TemplateLensProgress {
    TemplateLensProgress {
        phase,
        ordinal: idx + 1,
        total,
        slot_key: lens.slot_key.clone(),
        lens_name: lens.lens_name.clone(),
        lens_id: lens.lens_id.to_string(),
        runtime_lens_id: lens.runtime_lens_id.map(|id| id.to_string()),
        runtime: lens.runtime.clone(),
        modality: format!("{:?}", lens.modality),
        shape: format!("{:?}", lens.shape),
        placement: format!("{:?}", lens.placement),
        manifest: lens.manifest.clone(),
    }
}
