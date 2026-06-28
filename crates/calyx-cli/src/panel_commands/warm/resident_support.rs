use super::*;

const RESIDENT_CPU_LENS_REFUSED: &str = "CALYX_PANEL_RESIDENT_CPU_LENS_REFUSED";

pub(in crate::panel_commands) struct ResidentWarmOptions {
    pub(in crate::panel_commands) home: PathBuf,
    pub(in crate::panel_commands) template: String,
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
        log.append(&run_progress_record(
            &options.template,
            "resident_run_start",
        ))?;
    }
    require_gpu_content_lenses(&options.home, &options.template, progress_log.as_ref())?;
    let preflight = warm_preflight(
        &options.home,
        &options.template,
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
        &options.template,
        now_ms(),
        &shared_progress_log,
        &load_limit,
        load_parallelism,
    )?;
    let load_ms = load_started.elapsed().as_millis();
    let probe_started = Instant::now();
    let probes = probe_panel(&build, progress_log.as_ref(), &options.template)?;
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
        template_selector: options.template,
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
