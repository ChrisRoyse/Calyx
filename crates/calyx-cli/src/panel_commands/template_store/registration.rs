use calyx_registry::Registry;

use crate::error::CliResult;
use crate::lens_commands::support::{prepare_manifest_runtime, register_prepared_manifest_runtime};

use super::progress::{emit_progress, lens_progress};
use super::{
    SavedPanelTemplate, TEMPLATE_INVALID, TemplateLensProgress, id_for_loaded, template_error,
};

#[cfg(test)]
pub(in crate::panel_commands) fn register_template_lenses(
    registry: &mut Registry,
    template: &mut SavedPanelTemplate,
) -> CliResult<usize> {
    register_template_lenses_with_progress(registry, template, None)
}

pub(in crate::panel_commands) fn register_template_lenses_with_progress(
    registry: &mut Registry,
    template: &mut SavedPanelTemplate,
    progress: Option<&mut dyn FnMut(TemplateLensProgress) -> CliResult<()>>,
) -> CliResult<usize> {
    // Registry/template registration is an in-memory transaction. A late lens
    // must not leave earlier lenses registered or runtime IDs partially filled
    // when the caller chooses to inspect/reuse these values after an error.
    let mut staged_registry = registry.clone();
    let mut staged_template = template.clone();
    let added =
        register_template_lenses_staged(&mut staged_registry, &mut staged_template, progress)?;
    *registry = staged_registry;
    *template = staged_template;
    Ok(added)
}

fn register_template_lenses_staged(
    registry: &mut Registry,
    template: &mut SavedPanelTemplate,
    progress: Option<&mut dyn FnMut(TemplateLensProgress) -> CliResult<()>>,
) -> CliResult<usize> {
    let mut progress = progress;
    let mut added = 0;
    let total = template.lenses.len();
    let template_id = id_for_loaded(template)?;
    for (idx, lens) in template.lenses.iter_mut().enumerate() {
        emit_progress(&mut progress, lens_progress("load_start", idx, total, lens))?;
        let spec = lens.verified_materialization_spec(&template_id)?;
        let prepared = prepare_manifest_runtime(spec)
            .map_err(|error| lens.materialization_error(&template_id, "runtime_prepare", error))?;
        let expected_contract = lens.expected_runtime_contract().ok_or_else(|| {
            template_error(
                TEMPLATE_INVALID,
                format!(
                    "template {template_id} lens {} is missing its frozen runtime contract",
                    lens.lens_name
                ),
                "explicitly refresh the template from verified commissioned artifacts",
            )
        })?;
        if &prepared.contract != expected_contract {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!(
                    "template {template_id} lens {} runtime contract conflict: prepared={} expected={} spec_blake3={} manifest_blake3={}",
                    lens.lens_name,
                    prepared.contract.lens_id(),
                    expected_contract.lens_id(),
                    lens.immutable_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.spec_blake3.as_str())
                        .unwrap_or("missing"),
                    lens.immutable_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.manifest_blake3.as_str())
                        .unwrap_or("missing")
                ),
                "recommission the lens and explicitly save a new template version; never reinterpret the existing object",
            ));
        }
        let runtime_spec_lens_id = prepared.spec.lens_id();
        let runtime_lens_id = prepared.contract.lens_id();
        if let Some(existing) = registry.find_lens_by_spec_id(runtime_spec_lens_id) {
            if registry.lens_spec(existing) != Some(&prepared.spec) {
                return Err(template_error(
                    TEMPLATE_INVALID,
                    format!(
                        "registry lens {existing} does not match manifest {}",
                        lens.manifest
                    ),
                    "recommission the lens so the registry snapshot and manifest are identical",
                ));
            }
            if let Some(expected) = lens.runtime_lens_id
                && existing != expected
            {
                return Err(template_error(
                    TEMPLATE_INVALID,
                    format!("runtime resolved {existing}, expected {expected}"),
                    "recommission the lens so runtime and manifest contracts agree",
                ));
            }
            lens.runtime_lens_id = Some(existing);
            emit_progress(
                &mut progress,
                lens_progress("existing_matched", idx, total, lens),
            )?;
            continue;
        }
        if let Some(expected) = lens.runtime_lens_id
            && runtime_lens_id != expected
        {
            return Err(template_error(
                TEMPLATE_INVALID,
                format!("runtime registered {runtime_lens_id}, expected {expected}"),
                "recommission the lens so runtime and manifest contracts agree",
            ));
        }
        emit_progress(
            &mut progress,
            lens_progress("runtime_register_start", idx, total, lens),
        )?;
        let registered = register_prepared_manifest_runtime(registry, prepared)?;
        lens.runtime_lens_id = Some(registered);
        emit_progress(
            &mut progress,
            lens_progress("runtime_register_ok", idx, total, lens),
        )?;
        added += 1;
    }
    Ok(added)
}
