use super::preflight::{build_loaded_warm_template_panel, warm_preflight_loaded};
use super::resident_vram::vault_resident_vram_preflight;
use super::*;
use crate::path_identity::vault_template_source;

const RESIDENT_CPU_LENS_REFUSED: &str = "CALYX_PANEL_RESIDENT_CPU_LENS_REFUSED";

pub(in crate::panel_commands) struct ResidentWarmOptions {
    pub(in crate::panel_commands) home: PathBuf,
    pub(in crate::panel_commands) template: Option<String>,
    pub(in crate::panel_commands) vault: Option<PathBuf>,
    pub(in crate::panel_commands) slots: Vec<SlotId>,
    pub(in crate::panel_commands) modality: Option<Modality>,
    pub(in crate::panel_commands) ready_out: Option<PathBuf>,
    pub(in crate::panel_commands) max_resident_vram_mib: u64,
    pub(in crate::panel_commands) resident_overhead_multiplier_milli: u64,
    pub(in crate::panel_commands) max_load_secs: u64,
    pub(in crate::panel_commands) load_parallelism: Option<usize>,
    pub(in crate::panel_commands) progress_out: Option<PathBuf>,
}

pub(in crate::panel_commands) struct ResidentWarmState {
    pub(in crate::panel_commands) build: SavedTemplatePanelBuild,
    pub(in crate::panel_commands) home: PathBuf,
    pub(in crate::panel_commands) template_selector: String,
    pub(in crate::panel_commands) template_source: String,
    pub(in crate::panel_commands) source_of_truth: String,
    pub(in crate::panel_commands) slot_scope: Vec<SlotId>,
    pub(in crate::panel_commands) ready_out: Option<PathBuf>,
    pub(in crate::panel_commands) max_resident_vram_mib: u64,
    pub(in crate::panel_commands) declared_template_vram_mib: u64,
    pub(in crate::panel_commands) resident_overhead_multiplier_milli: u64,
    pub(in crate::panel_commands) estimated_resident_vram_mib: u64,
    pub(in crate::panel_commands) max_load_secs: u64,
    pub(in crate::panel_commands) load_parallelism: usize,
    pub(in crate::panel_commands) load_ms: u128,
    pub(in crate::panel_commands) probe_ms: u128,
    pub(in crate::panel_commands) warmed_lens_count: usize,
    pub(in crate::panel_commands) content_lens_count: usize,
    pub(in crate::panel_commands) gpu_content_lens_count: usize,
    pub(in crate::panel_commands) cpu_excluded_slots: Vec<String>,
}

pub(in crate::panel_commands) fn load_resident_warm_state(
    options: ResidentWarmOptions,
) -> CliResult<ResidentWarmState> {
    if options.template.is_some() == options.vault.is_some() {
        return Err(CliError::usage(
            "resident warm state requires exactly one of template or vault",
        ));
    }
    if let Some(vault) = options.vault.clone() {
        return load_vault_resident_warm_state(options, vault);
    }
    let template = options
        .template
        .clone()
        .ok_or_else(|| CliError::usage("resident warm state missing template"))?;
    let _worker_shutdown = MultimodalGpuWorkerShutdownGuard;
    let progress_log = options
        .progress_out
        .clone()
        .map(WarmProgressLog::create)
        .transpose()?;
    let shared_progress_log = progress_log
        .as_ref()
        .map(|log| Arc::new(Mutex::new(log.clone())));
    if let Some(log) = &progress_log {
        log.append(&run_progress_record(&template, "resident_run_start"))?;
    }
    // Load and validate the immutable source exactly once. Selection must be
    // resolved against that full identity before any runtime is prepared.
    let store = template_store::TemplateStore::open(&options.home);
    let saved_template = store.load(&template)?;
    saved_template.validate()?;
    validate_template_slot_capacity(&template, &saved_template)?;
    let template_id = template_store::id_for_loaded(&saved_template)?;
    let mut selected_panel = saved_template.to_target_panel(now_ms());
    let slot_scope = normalized_slot_scope(&template, options.slots)?;
    apply_resident_slot_scope(
        &template,
        &mut selected_panel,
        &slot_scope,
        options.modality,
    )?;
    apply_resident_modality_scope(&mut selected_panel, options.modality);
    let selected_slots = selected_content_slots(&template, &selected_panel)?;
    require_gpu_content_panel(&template, &selected_panel, progress_log.as_ref())?;
    if let Some(log) = &progress_log
        && (!slot_scope.is_empty()
            || options.modality.is_some()
            || selected_slots.len() != saved_template.lenses.len())
    {
        let mut record = run_progress_record(&template, "resident_slot_scope_selected");
        record.lens_count = Some(selected_slots.len());
        log.append(&record)?;
    }
    let preflight = warm_preflight_loaded(
        &template,
        &saved_template,
        Some(&selected_slots),
        options.max_resident_vram_mib,
        options.resident_overhead_multiplier_milli,
        progress_log.as_ref(),
    )?;
    let load_parallelism = options
        .load_parallelism
        .unwrap_or_else(|| default_load_parallelism(preflight.lens_count));
    let load_limit = WarmLoadLimit::new(options.max_load_secs);
    let load_started = Instant::now();
    let mut build = build_loaded_warm_template_panel(
        saved_template,
        template_id,
        Some(&selected_slots),
        &template,
        now_ms(),
        &shared_progress_log,
        &load_limit,
        load_parallelism,
    )?;
    apply_resident_slot_scope(&template, &mut build.panel, &slot_scope, options.modality)?;
    apply_resident_modality_scope(&mut build.panel, options.modality);
    let load_ms = load_started.elapsed().as_millis();
    let probe_started = Instant::now();
    let probes = probe_panel(&build, progress_log.as_ref(), &template)?;
    let probe_ms = probe_started.elapsed().as_millis();
    let content_lens_count = content_slots(&build).count();
    let gpu_content_lens_count = content_slots(&build)
        .filter(|slot| slot.resource.placement == Placement::Gpu)
        .count();
    Ok(ResidentWarmState {
        source_of_truth: source_of_truth(&options.home, &build.template_id),
        template_source: format!("saved:{}:{}", build.template_name, build.template_id),
        slot_scope,
        build,
        home: options.home,
        template_selector: template,
        ready_out: options.ready_out,
        max_resident_vram_mib: options.max_resident_vram_mib,
        declared_template_vram_mib: preflight.declared_template_vram_mib,
        resident_overhead_multiplier_milli: options.resident_overhead_multiplier_milli,
        estimated_resident_vram_mib: preflight.estimated_resident_vram_mib,
        max_load_secs: options.max_load_secs,
        load_parallelism,
        load_ms,
        probe_ms,
        warmed_lens_count: probes.len(),
        content_lens_count,
        gpu_content_lens_count,
        cpu_excluded_slots: Vec::new(),
    })
}

fn load_vault_resident_warm_state(
    options: ResidentWarmOptions,
    vault: PathBuf,
) -> CliResult<ResidentWarmState> {
    let _worker_shutdown = MultimodalGpuWorkerShutdownGuard;
    let selector = vault_template_source(&vault)?;
    let progress_log = options
        .progress_out
        .clone()
        .map(WarmProgressLog::create)
        .transpose()?;
    if let Some(log) = &progress_log {
        log.append(&run_progress_record(&selector, "resident_run_start"))?;
    }
    let slot_scope = normalized_slot_scope(&selector, options.slots)?;
    let load_started = Instant::now();
    let state = load_vault_panel_state(&vault)?;
    let mut panel = state.panel;
    apply_resident_slot_scope(&selector, &mut panel, &slot_scope, options.modality)?;
    apply_resident_modality_scope(&mut panel, options.modality);
    selected_content_slots(&selector, &panel)?;
    if let Some(log) = &progress_log
        && !slot_scope.is_empty()
    {
        let mut record = run_progress_record(&selector, "resident_slot_scope_selected");
        record.lens_count = Some(slot_scope.len());
        log.append(&record)?;
    }
    let cpu_excluded_slots =
        exclude_cpu_content_slots(&selector, &mut panel, progress_log.as_ref())?;
    let preflight = vault_resident_vram_preflight(
        &selector,
        &panel,
        &state.registry,
        options.max_resident_vram_mib,
        options.resident_overhead_multiplier_milli,
        progress_log.as_ref(),
    )?;
    let build = SavedTemplatePanelBuild {
        template_id: format!("vault:{}", vault.display()),
        template_name: vault
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("vault")
            .to_string(),
        panel,
        registry: state.registry,
        a37_gate_eligible: false,
        a37_status: "vault_source".to_string(),
        registered_lenses_added: 0,
    };
    let load_ms = load_started.elapsed().as_millis();
    let probe_started = Instant::now();
    let probes = probe_panel(&build, progress_log.as_ref(), &selector)?;
    let probe_ms = probe_started.elapsed().as_millis();
    let content_lens_count = content_slots(&build).count();
    let gpu_content_lens_count = content_slots(&build)
        .filter(|slot| slot.resource.placement == Placement::Gpu)
        .count();
    Ok(ResidentWarmState {
        source_of_truth: vault_source_of_truth(&selector),
        template_source: selector.clone(),
        slot_scope,
        build,
        home: options.home,
        template_selector: selector,
        ready_out: options.ready_out,
        max_resident_vram_mib: options.max_resident_vram_mib,
        declared_template_vram_mib: preflight.declared_template_vram_mib,
        resident_overhead_multiplier_milli: options.resident_overhead_multiplier_milli,
        estimated_resident_vram_mib: preflight.estimated_resident_vram_mib,
        max_load_secs: options.max_load_secs,
        load_parallelism: 1,
        load_ms,
        probe_ms,
        warmed_lens_count: probes.len(),
        content_lens_count,
        gpu_content_lens_count,
        cpu_excluded_slots,
    })
}

fn vault_source_of_truth(vault_source: &str) -> String {
    format!("vault MANIFEST panel_ref registry_ref:{vault_source}")
}

fn normalized_slot_scope(selector: &str, slots: Vec<SlotId>) -> CliResult<Vec<SlotId>> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::with_capacity(slots.len());
    for slot_id in slots {
        if !seen.insert(slot_id) {
            return Err(resident_slot_scope_error(
                selector,
                format!("duplicate --slot {}", slot_id.get()),
            ));
        }
        normalized.push(slot_id);
    }
    Ok(normalized)
}

fn apply_resident_slot_scope(
    selector: &str,
    panel: &mut Panel,
    slot_scope: &[SlotId],
    modality: Option<Modality>,
) -> CliResult {
    if slot_scope.is_empty() {
        return Ok(());
    }
    let requested = slot_scope.iter().copied().collect::<BTreeSet<_>>();
    let mut scoped_lenses = Vec::with_capacity(slot_scope.len());
    for slot_id in slot_scope {
        let slot = panel
            .slots
            .iter()
            .find(|candidate| candidate.slot_id == *slot_id)
            .ok_or_else(|| {
                resident_slot_scope_error(
                    selector,
                    format!("--slot {} is not present", slot_id.get()),
                )
            })?;
        if slot.state != SlotState::Active {
            return Err(resident_slot_scope_error(
                selector,
                format!(
                    "--slot {} is {:?}, expected Active",
                    slot_id.get(),
                    slot.state
                ),
            ));
        }
        if slot.retrieval_only || slot.excluded_from_dedup {
            return Err(resident_slot_scope_error(
                selector,
                format!(
                    "--slot {} is not a content lens retrieval_only={} excluded_from_dedup={}",
                    slot_id.get(),
                    slot.retrieval_only,
                    slot.excluded_from_dedup
                ),
            ));
        }
        if let Some(modality) = modality
            && slot.modality != modality
        {
            return Err(resident_slot_scope_error(
                selector,
                format!(
                    "--slot {} modality {:?} does not match --modality {:?}",
                    slot_id.get(),
                    slot.modality,
                    modality
                ),
            ));
        }
        if slot.resource.placement != Placement::Gpu {
            scoped_lenses.push(format!(
                "slot={} key={} lens={} placement={:?}",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id,
                slot.resource.placement
            ));
        }
    }
    if !scoped_lenses.is_empty() {
        return Err(CliError::from(CalyxError {
            code: RESIDENT_CPU_LENS_REFUSED,
            message: format!(
                "resident source {selector} refuses {} selected CPU/non-GPU content lenses: {}",
                scoped_lenses.len(),
                scoped_lenses.join(", ")
            ),
            remediation: "choose only GPU resident slots or replace the selected content lenses with GPU resident runtimes",
        }));
    }
    panel.slots.retain(|slot| requested.contains(&slot.slot_id));
    Ok(())
}

fn resident_slot_scope_error(selector: &str, detail: String) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_PANEL_RESIDENT_SLOT_SCOPE_INVALID",
        message: format!("resident source {selector} has invalid slot scope: {detail}"),
        remediation: "pass --slot only for active GPU content slots present in the selected source panel",
    })
}

fn apply_resident_modality_scope(panel: &mut Panel, modality: Option<Modality>) {
    let Some(modality) = modality else {
        return;
    };
    panel
        .slots
        .retain(|slot| !is_active_content_slot(slot) || slot.modality == modality);
}

fn selected_content_slots(selector: &str, panel: &Panel) -> CliResult<BTreeSet<SlotId>> {
    let selected = panel
        .slots
        .iter()
        .filter(|slot| is_active_content_slot(slot))
        .map(|slot| slot.slot_id)
        .collect::<BTreeSet<_>>();
    if selected.is_empty() {
        return Err(resident_slot_scope_error(
            selector,
            "selection contains no active content lens".to_string(),
        ));
    }
    Ok(selected)
}

fn validate_template_slot_capacity(
    selector: &str,
    template: &template_store::SavedPanelTemplate,
) -> CliResult {
    let total_slots = template
        .lenses
        .len()
        .checked_add(template.time_controls.len())
        .ok_or_else(|| {
            resident_slot_scope_error(selector, "source slot count overflowed usize".to_string())
        })?;
    let max_slots = usize::from(u16::MAX) + 1;
    if total_slots > max_slots {
        return Err(resident_slot_scope_error(
            selector,
            format!(
                "source contains {total_slots} slots, exceeding the SlotId capacity of {max_slots}"
            ),
        ));
    }
    Ok(())
}

fn is_active_content_slot(slot: &Slot) -> bool {
    slot.state == SlotState::Active && !slot.retrieval_only && !slot.excluded_from_dedup
}

/// Exclude active CPU/non-GPU content slots from the resident's warm roster
/// (#1490). The resident is GPU-only by design (#1066); search and ingest
/// measure CPU-placed slots locally in-process, so serving a vault must not
/// deadlock on them. The exclusion is LOUD — structured stderr line, progress
/// record, and the ready payload lists every excluded slot — and a panel with
/// no GPU content slot at all is still refused (a CPU-only panel needs no
/// resident).
fn exclude_cpu_content_slots(
    selector: &str,
    panel: &mut Panel,
    progress_log: Option<&WarmProgressLog>,
) -> CliResult<Vec<String>> {
    let is_active_content = |slot: &Slot| {
        slot.state == SlotState::Active && !slot.retrieval_only && !slot.excluded_from_dedup
    };
    let excluded = panel
        .slots
        .iter()
        .filter(|slot| is_active_content(slot) && slot.resource.placement != Placement::Gpu)
        .map(|slot| {
            format!(
                "slot={} key={} lens={} placement={:?}",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id,
                slot.resource.placement
            )
        })
        .collect::<Vec<_>>();
    if excluded.is_empty() {
        return Ok(excluded);
    }
    let gpu_remaining = panel
        .slots
        .iter()
        .filter(|slot| is_active_content(slot) && slot.resource.placement == Placement::Gpu)
        .count();
    if gpu_remaining == 0 {
        return Err(CliError::from(CalyxError {
            code: RESIDENT_CPU_LENS_REFUSED,
            message: format!(
                "resident vault {selector} has no GPU content lenses to serve; all {} content lenses are CPU/non-GPU placed: {}",
                excluded.len(),
                excluded.join(", ")
            ),
            remediation: "a CPU-only panel needs no resident: run search without --resident-addr, or replace the content lenses with GPU resident runtimes",
        }));
    }
    panel
        .slots
        .retain(|slot| !(is_active_content(slot) && slot.resource.placement != Placement::Gpu));
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=resident_cpu_lens_excluded selector={selector} gpu_content_lenses={gpu_remaining} cpu_excluded={} slots=[{}]",
        excluded.len(),
        excluded.join(", ")
    );
    if let Some(log) = progress_log {
        let mut record = run_progress_record(selector, "resident_cpu_lens_excluded");
        record.lens_count = Some(excluded.len());
        record.remediation = Some(
            "CPU-placed content lenses are excluded from the GPU resident and measured in-process at ingest/search time (#1490)"
                .to_string(),
        );
        log.append(&record)?;
    }
    Ok(excluded)
}

fn require_gpu_content_panel(
    selector: &str,
    panel: &Panel,
    progress_log: Option<&WarmProgressLog>,
) -> CliResult {
    let cpu_lenses = panel
        .slots
        .iter()
        .filter(|slot| is_active_content_slot(slot) && slot.resource.placement != Placement::Gpu)
        .map(|slot| {
            format!(
                "slot={} key={} lens={} placement={:?}",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id,
                slot.resource.placement
            )
        })
        .collect::<Vec<_>>();
    if cpu_lenses.is_empty() {
        return Ok(());
    }
    let message = format!(
        "resident panel {selector} refuses {} CPU/non-GPU content lenses: {}",
        cpu_lenses.len(),
        cpu_lenses.join(", ")
    );
    if let Some(log) = progress_log {
        let mut record = run_progress_record(selector, "resident_gpu_placement_error");
        record.lens_count = Some(
            panel
                .slots
                .iter()
                .filter(|slot| is_active_content_slot(slot))
                .count(),
        );
        record.error_code = Some(RESIDENT_CPU_LENS_REFUSED.to_string());
        record.error_message = Some(message.clone());
        record.remediation = Some(
            "replace every content lens with a GPU resident runtime before starting the service"
                .to_string(),
        );
        log.append(&record)?;
    }
    Err(CliError::from(CalyxError {
        code: RESIDENT_CPU_LENS_REFUSED,
        message,
        remediation: "replace every content lens with a GPU resident runtime before starting the service",
    }))
}

#[cfg(test)]
mod tests;
