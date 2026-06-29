use super::*;

const RESIDENT_CPU_LENS_REFUSED: &str = "CALYX_PANEL_RESIDENT_CPU_LENS_REFUSED";

pub(in crate::panel_commands) struct ResidentWarmOptions {
    pub(in crate::panel_commands) home: PathBuf,
    pub(in crate::panel_commands) template: Option<String>,
    pub(in crate::panel_commands) vault: Option<PathBuf>,
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
    pub(in crate::panel_commands) ready_out: Option<PathBuf>,
    pub(in crate::panel_commands) max_resident_vram_mib: u64,
    pub(in crate::panel_commands) declared_template_vram_mib: u64,
    pub(in crate::panel_commands) resident_overhead_multiplier: f32,
    pub(in crate::panel_commands) estimated_resident_vram_mib: u64,
    pub(in crate::panel_commands) max_load_secs: u64,
    pub(in crate::panel_commands) load_parallelism: usize,
    pub(in crate::panel_commands) load_ms: u128,
    pub(in crate::panel_commands) probe_ms: u128,
    pub(in crate::panel_commands) warmed_lens_count: usize,
    pub(in crate::panel_commands) content_lens_count: usize,
    pub(in crate::panel_commands) gpu_content_lens_count: usize,
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
    require_gpu_content_lenses(&options.home, &template, progress_log.as_ref())?;
    let preflight = warm_preflight(
        &options.home,
        &template,
        options.max_resident_vram_mib,
        options.resident_overhead_multiplier_milli,
        progress_log.as_ref(),
    )?;
    let load_parallelism = options
        .load_parallelism
        .unwrap_or_else(|| default_load_parallelism(preflight.lens_count));
    let load_limit = WarmLoadLimit::new(options.max_load_secs);
    let load_started = Instant::now();
    let build = build_warm_template_panel(
        &options.home,
        &template,
        now_ms(),
        &shared_progress_log,
        &load_limit,
        load_parallelism,
    )?;
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
        build,
        home: options.home,
        template_selector: template,
        ready_out: options.ready_out,
        max_resident_vram_mib: options.max_resident_vram_mib,
        declared_template_vram_mib: preflight.declared_template_vram_mib,
        resident_overhead_multiplier: multiplier_to_f32(options.resident_overhead_multiplier_milli),
        estimated_resident_vram_mib: preflight.estimated_resident_vram_mib,
        max_load_secs: options.max_load_secs,
        load_parallelism,
        load_ms,
        probe_ms,
        warmed_lens_count: probes.len(),
        content_lens_count,
        gpu_content_lens_count,
    })
}

fn load_vault_resident_warm_state(
    options: ResidentWarmOptions,
    vault: PathBuf,
) -> CliResult<ResidentWarmState> {
    let _worker_shutdown = MultimodalGpuWorkerShutdownGuard;
    let selector = format!("vault:{}", vault.display());
    let progress_log = options
        .progress_out
        .clone()
        .map(WarmProgressLog::create)
        .transpose()?;
    if let Some(log) = &progress_log {
        log.append(&run_progress_record(&selector, "resident_run_start"))?;
    }
    let load_started = Instant::now();
    let state = load_vault_panel_state(&vault)?;
    let mut panel = state.panel;
    if let Some(modality) = options.modality {
        panel.slots.retain(|slot| {
            slot.state != SlotState::Active
                || slot.modality == modality
                || slot.slot_key.key().starts_with("E")
        });
    }
    require_gpu_content_slots(&selector, &panel.slots)?;
    let build = SavedTemplatePanelBuild {
        template_id: format!("vault:{}", vault.display()),
        template_name: vault
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("vault")
            .to_string(),
        content_lens_count: panel
            .slots
            .iter()
            .filter(|slot| {
                slot.state == SlotState::Active && !slot.retrieval_only && !slot.excluded_from_dedup
            })
            .count(),
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
        source_of_truth: vault_source_of_truth(&vault),
        template_source: format!("vault:{}", vault.display()),
        build,
        home: options.home,
        template_selector: selector,
        ready_out: options.ready_out,
        max_resident_vram_mib: options.max_resident_vram_mib,
        declared_template_vram_mib: 0,
        resident_overhead_multiplier: multiplier_to_f32(options.resident_overhead_multiplier_milli),
        estimated_resident_vram_mib: 0,
        max_load_secs: options.max_load_secs,
        load_parallelism: 1,
        load_ms,
        probe_ms,
        warmed_lens_count: probes.len(),
        content_lens_count,
        gpu_content_lens_count,
    })
}

fn vault_source_of_truth(vault: &Path) -> String {
    format!("vault MANIFEST panel_ref registry_ref:{}", vault.display())
}

fn require_gpu_content_slots(selector: &str, slots: &[Slot]) -> CliResult {
    let cpu_lenses = slots
        .iter()
        .filter(|slot| {
            slot.state == SlotState::Active
                && !slot.retrieval_only
                && !slot.excluded_from_dedup
                && slot.resource.placement != Placement::Gpu
        })
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
    Err(CliError::from(CalyxError {
        code: RESIDENT_CPU_LENS_REFUSED,
        message: format!(
            "resident vault {selector} refuses {} CPU/non-GPU content lenses: {}",
            cpu_lenses.len(),
            cpu_lenses.join(", ")
        ),
        remediation: "pass --modality to select a GPU-only modality or replace every content lens with a GPU resident runtime",
    }))
}

fn require_gpu_content_lenses(
    home: &Path,
    selector: &str,
    progress_log: Option<&WarmProgressLog>,
) -> CliResult {
    let store = template_store::TemplateStore::open(home);
    let template = store.load(selector)?;
    template.validate()?;
    let cpu_lenses = template
        .lenses
        .iter()
        .filter(|lens| lens.counts_toward_a35 && lens.placement != Placement::Gpu)
        .map(|lens| {
            format!(
                "{}:{}:{:?}:{}",
                lens.slot_key, lens.lens_id, lens.placement, lens.manifest
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
        record.lens_count = Some(template.lenses.len());
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
