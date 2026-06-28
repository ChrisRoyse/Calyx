use std::collections::{BTreeSet, VecDeque};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use calyx_core::{CalyxError, Input, Modality, Placement, Slot, SlotShape, SlotState, SlotVector};
use calyx_registry::{
    LensRuntime, Registry, lens_spec_from_manifest_path, shutdown_multimodal_gpu_workers,
};
use serde::Serialize;

use super::{
    SavedTemplatePanelBuild,
    template_store::{self, TemplateLensProgress},
};
use crate::cmd::vault::now_ms;
use crate::error::{CliError, CliResult};
use crate::lens_commands::support::{
    PreparedRuntimeLens, prepare_manifest_runtime, register_prepared_manifest_runtime,
    runtime_name, slot_norm, slot_prefix, validate_vector_contract,
};
use crate::output::print_json;

const SCHEMA: &str = "calyx-panel-warm-v1";
const PROGRESS_SCHEMA: &str = "calyx-panel-warm-progress-v1";
const DEFAULT_MAX_RESIDENT_VRAM_MIB: u64 = 22 * 1024;
const DEFAULT_RESIDENT_OVERHEAD_MULTIPLIER_MILLI: u64 = 2100;
const DEFAULT_MAX_LOAD_SECS: u64 = 60;
const DEFAULT_LOAD_PARALLELISM: usize = 8;
const WARM_VRAM_BUDGET: &str = "CALYX_PANEL_WARM_VRAM_BUDGET";
const WARM_TIMEOUT: &str = "CALYX_PANEL_WARM_TIMEOUT";

#[derive(Debug, Default)]
struct Flags {
    home: Option<PathBuf>,
    template: Option<String>,
    hold_secs: u64,
    out: Option<PathBuf>,
    progress_out: Option<PathBuf>,
    max_resident_vram_mib: Option<u64>,
    resident_overhead_multiplier_milli: Option<u64>,
    max_load_secs: Option<u64>,
    load_parallelism: Option<usize>,
}

#[derive(Serialize)]
struct WarmReport {
    schema: &'static str,
    source_of_truth: String,
    home: PathBuf,
    template_selector: String,
    template_source: String,
    process_id: u32,
    hold_secs: u64,
    registry_resident_while_holding: bool,
    registry_residency_scope: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress_log: Option<PathBuf>,
    max_resident_vram_mib: u64,
    declared_template_vram_mib: u64,
    resident_overhead_multiplier: f32,
    estimated_resident_vram_mib: u64,
    max_load_secs: u64,
    load_parallelism: usize,
    load_ms: u128,
    probe_ms: u128,
    slot_count: usize,
    content_lens_count: usize,
    registry_lens_count: usize,
    registered_lenses_added: usize,
    gpu_content_lens_count: usize,
    cpu_content_lens_count: usize,
    warmed_lens_count: usize,
    a37_gate_eligible: bool,
    a37_status: String,
    probes: Vec<WarmProbeReport>,
}

#[derive(Serialize)]
struct WarmProgressRecord {
    schema: &'static str,
    timestamp_ms: u64,
    process_id: u32,
    template_selector: String,
    phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ordinal: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slot: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lens_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_lens_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lens_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spec_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    elapsed_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lens_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_lens_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    declared_template_vram_mib: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    estimated_resident_vram_mib: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_resident_vram_mib: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resident_overhead_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    load_parallelism: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vector_kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vector_len: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    norm: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_values: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remediation: Option<String>,
}

#[derive(Clone)]
struct WarmProgressLog {
    path: PathBuf,
}

type SharedProgressLog = Option<Arc<Mutex<WarmProgressLog>>>;

impl WarmProgressLog {
    fn create(path: PathBuf) -> CliResult<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, b"")?;
        Ok(Self { path })
    }

    fn append(&self, record: &WarmProgressRecord) -> CliResult {
        let line = serde_json::to_string(record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_data()?;
        eprintln!("{line}");
        Ok(())
    }
}

#[derive(Serialize)]
struct WarmProbeReport {
    slot: u16,
    key: String,
    lens_id: String,
    spec_name: String,
    runtime: &'static str,
    runtime_detail: String,
    modality: Modality,
    shape: SlotShape,
    placement: Placement,
    vector_kind: &'static str,
    vector_len: usize,
    norm: f32,
    first_values: Vec<f32>,
    elapsed_ms: u128,
}

struct WarmPreflight {
    lens_count: usize,
    declared_template_vram_mib: u64,
    estimated_resident_vram_mib: u64,
}

struct WarmLoadLimit {
    started: Instant,
    max_load_secs: u64,
}

struct WarmLoadWait<'a> {
    selector: &'a str,
    phase: &'static str,
    completed: usize,
    total: usize,
    load_parallelism: usize,
    progress_log: &'a SharedProgressLog,
}

impl WarmLoadLimit {
    fn new(max_load_secs: u64) -> Self {
        Self {
            started: Instant::now(),
            max_load_secs,
        }
    }

    fn recv<T>(&self, rx: &mpsc::Receiver<T>, wait: WarmLoadWait<'_>) -> CliResult<T> {
        if self.max_load_secs == 0 {
            return rx.recv().map_err(|_| {
                CliError::from(CalyxError::lens_unreachable(format!(
                    "panel warm worker channel closed during {phase}; completed={completed}/{total}",
                    phase = wait.phase,
                    completed = wait.completed,
                    total = wait.total,
                )))
            });
        }
        match self.remaining() {
            Some(remaining) if !remaining.is_zero() => rx.recv_timeout(remaining).map_err(|err| {
                match err {
                    mpsc::RecvTimeoutError::Timeout => self.timeout_error(&wait),
                    mpsc::RecvTimeoutError::Disconnected => {
                        CliError::from(CalyxError::lens_unreachable(format!(
                            "panel warm worker channel closed during {phase}; completed={completed}/{total}",
                            phase = wait.phase,
                            completed = wait.completed,
                            total = wait.total,
                        )))
                    }
                }
            }),
            _ => Err(self.timeout_error(&wait)),
        }
    }

    fn remaining(&self) -> Option<Duration> {
        if self.max_load_secs == 0 {
            return None;
        }
        Duration::from_secs(self.max_load_secs).checked_sub(self.started.elapsed())
    }

    fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    fn timeout_error(&self, wait: &WarmLoadWait<'_>) -> CliError {
        let elapsed_ms = self.elapsed_ms();
        let message = format!(
            "panel warm readiness exceeded {max}s during {phase}; completed={completed}/{total}; \
             load_parallelism={load_parallelism}; elapsed_ms={elapsed_ms}; all configured lenses \
             must prepare and complete warmup inference inside the global readiness deadline",
            max = self.max_load_secs,
            phase = wait.phase,
            completed = wait.completed,
            total = wait.total,
            load_parallelism = wait.load_parallelism,
        );
        let mut record = run_progress_record(wait.selector, "load_timeout");
        record.elapsed_ms = Some(elapsed_ms);
        record.lens_count = Some(wait.total);
        record.load_parallelism = Some(wait.load_parallelism);
        record.error_code = Some(WARM_TIMEOUT.to_string());
        record.error_message = Some(message.clone());
        record.remediation = Some(
            "increase bounded load parallelism only if VRAM remains under cap, optimize or replace \
             slow lenses, or start the resident warm service once"
                .to_string(),
        );
        let _ = append_shared_progress(wait.progress_log, &record);
        CliError::from(CalyxError {
            code: WARM_TIMEOUT,
            message,
            remediation: "increase bounded load parallelism only if VRAM remains under cap, optimize or replace slow lenses, or start the resident warm service once",
        })
    }
}

pub(super) fn run(args: &[String]) -> CliResult {
    let _worker_shutdown = MultimodalGpuWorkerShutdownGuard;
    let flags = Flags::parse(args)?;
    let max_resident_vram_mib = flags
        .max_resident_vram_mib
        .unwrap_or(DEFAULT_MAX_RESIDENT_VRAM_MIB);
    let resident_overhead_multiplier_milli = flags
        .resident_overhead_multiplier_milli
        .unwrap_or(DEFAULT_RESIDENT_OVERHEAD_MULTIPLIER_MILLI);
    let max_load_secs = flags.max_load_secs.unwrap_or(DEFAULT_MAX_LOAD_SECS);
    let home = match flags.home {
        Some(home) => home,
        None => calyx_home()?,
    };
    let template = flags
        .template
        .ok_or_else(|| CliError::usage("calyx panel warm requires --template <name-or-id>"))?;
    let progress_path = flags
        .progress_out
        .clone()
        .or_else(|| flags.out.as_deref().map(derive_progress_path));
    let progress_log = progress_path.map(WarmProgressLog::create).transpose()?;
    let shared_progress_log = progress_log
        .as_ref()
        .map(|log| Arc::new(Mutex::new(log.clone())));
    if let Some(log) = &progress_log {
        log.append(&run_progress_record(&template, "run_start"))?;
    }
    let preflight = warm_preflight(
        &home,
        &template,
        max_resident_vram_mib,
        resident_overhead_multiplier_milli,
        progress_log.as_ref(),
    )?;
    let load_parallelism = flags
        .load_parallelism
        .unwrap_or_else(|| default_load_parallelism(preflight.lens_count));
    let load_limit = WarmLoadLimit::new(max_load_secs);

    let load_started = Instant::now();
    let build = build_warm_template_panel(
        &home,
        &template,
        now_ms(),
        &shared_progress_log,
        &load_limit,
        load_parallelism,
    )?;
    let load_ms = load_started.elapsed().as_millis();
    if let Some(log) = &progress_log {
        log.append(&run_progress_record(&template, "load_ok"))?;
    }

    let probe_started = Instant::now();
    let probes = probe_panel(&build, progress_log.as_ref(), &template)?;
    let probe_ms = probe_started.elapsed().as_millis();
    if let Some(log) = &progress_log {
        log.append(&run_progress_record(&template, "probe_ok"))?;
    }

    let gpu_content_lens_count = content_slots(&build)
        .filter(|slot| slot.resource.placement == Placement::Gpu)
        .count();
    let content_lens_count = content_slots(&build).count();
    let report = WarmReport {
        schema: SCHEMA,
        source_of_truth: source_of_truth(&home, &build.template_id),
        home,
        template_selector: template.clone(),
        template_source: format!("saved:{}:{}", build.template_name, build.template_id),
        process_id: std::process::id(),
        hold_secs: flags.hold_secs,
        registry_resident_while_holding: flags.hold_secs > 0,
        registry_residency_scope: "cli_process_only",
        progress_log: progress_log.as_ref().map(|log| log.path.clone()),
        max_resident_vram_mib,
        declared_template_vram_mib: preflight.declared_template_vram_mib,
        resident_overhead_multiplier: multiplier_to_f32(resident_overhead_multiplier_milli),
        estimated_resident_vram_mib: preflight.estimated_resident_vram_mib,
        max_load_secs,
        load_parallelism,
        load_ms,
        probe_ms,
        slot_count: build.panel.slots.len(),
        content_lens_count,
        registry_lens_count: build.registry.lens_snapshots().len(),
        registered_lenses_added: build.registered_lenses_added,
        gpu_content_lens_count,
        cpu_content_lens_count: content_lens_count.saturating_sub(gpu_content_lens_count),
        warmed_lens_count: probes.len(),
        a37_gate_eligible: build.a37_gate_eligible,
        a37_status: build.a37_status.clone(),
        probes,
    };

    if let Some(path) = flags.out {
        write_report(&path, &report)?;
    }
    print_json(&report)?;

    if flags.hold_secs > 0 {
        if let Some(log) = &progress_log {
            log.append(&run_progress_record(&template, "hold_start"))?;
        }
        thread::sleep(Duration::from_secs(flags.hold_secs));
        if let Some(log) = &progress_log {
            log.append(&run_progress_record(&template, "hold_ok"))?;
        }
    }
    Ok(())
}

struct MultimodalGpuWorkerShutdownGuard;

impl Drop for MultimodalGpuWorkerShutdownGuard {
    fn drop(&mut self) {
        shutdown_multimodal_gpu_workers();
    }
}

fn warm_preflight(
    home: &Path,
    selector: &str,
    max_resident_vram_mib: u64,
    resident_overhead_multiplier_milli: u64,
    progress_log: Option<&WarmProgressLog>,
) -> CliResult<WarmPreflight> {
    let store = template_store::TemplateStore::open(home);
    let template = store.load(selector)?;
    template.validate()?;
    let declared_bytes = template
        .lenses
        .iter()
        .map(|lens| lens.cost.vram_bytes)
        .fold(0_u64, u64::saturating_add);
    let declared_template_vram_mib = bytes_to_mib_ceil(declared_bytes);
    let estimated_resident_vram_mib =
        estimate_resident_vram_mib(declared_bytes, resident_overhead_multiplier_milli);
    let semantic_lenses = semantic_lens_count(&template.lenses);
    if let Some(log) = progress_log {
        let mut record = run_progress_record(selector, "vram_preflight");
        record.lens_count = Some(template.lenses.len());
        record.semantic_lens_count = Some(semantic_lenses);
        record.declared_template_vram_mib = Some(declared_template_vram_mib);
        record.estimated_resident_vram_mib = Some(estimated_resident_vram_mib);
        record.max_resident_vram_mib = Some(max_resident_vram_mib);
        record.resident_overhead_multiplier =
            Some(multiplier_to_f32(resident_overhead_multiplier_milli));
        log.append(&record)?;
    }
    if estimated_resident_vram_mib > max_resident_vram_mib {
        let message = format!(
            "panel warm refuses template {selector}: declared_vram_mib={declared_template_vram_mib} \
             resident_overhead_multiplier={} estimated_resident_vram_mib={estimated_resident_vram_mib} \
             max_resident_vram_mib={max_resident_vram_mib} lens_count={} semantic_lens_count={semantic_lenses}; \
             this would exceed the Blackwell RTX 5090 22GiB resident target before inference",
            multiplier_to_f32(resident_overhead_multiplier_milli),
            template.lenses.len(),
        );
        if let Some(log) = progress_log {
            let mut record = run_progress_record(selector, "vram_preflight_error");
            record.lens_count = Some(template.lenses.len());
            record.semantic_lens_count = Some(semantic_lenses);
            record.declared_template_vram_mib = Some(declared_template_vram_mib);
            record.estimated_resident_vram_mib = Some(estimated_resident_vram_mib);
            record.max_resident_vram_mib = Some(max_resident_vram_mib);
            record.resident_overhead_multiplier =
                Some(multiplier_to_f32(resident_overhead_multiplier_milli));
            record.error_code = Some(WARM_VRAM_BUDGET.to_string());
            record.error_message = Some(message.clone());
            record.remediation = Some(
                "prune duplicate semantic embedders to 2-3 compact GPU/INT8 choices, replace \
                 non-Blackwell-friendly lenses, or raise the explicit --max-resident-vram-mib"
                    .to_string(),
            );
            log.append(&record)?;
        }
        return Err(CliError::from(CalyxError {
            code: WARM_VRAM_BUDGET,
            message,
            remediation: "prune duplicate semantic embedders to 2-3 compact GPU/INT8 choices, replace non-Blackwell-friendly lenses, or raise the explicit --max-resident-vram-mib",
        }));
    }
    Ok(WarmPreflight {
        lens_count: template.lenses.len(),
        declared_template_vram_mib,
        estimated_resident_vram_mib,
    })
}

fn default_load_parallelism(lens_count: usize) -> usize {
    lens_count.clamp(1, DEFAULT_LOAD_PARALLELISM)
}

fn semantic_lens_count(lenses: &[template_store::TemplateLensRef]) -> usize {
    lenses
        .iter()
        .filter(|lens| {
            lens.modality == Modality::Text
                && (lens.slot_key.contains("semantic")
                    || lens.lens_name.contains("semantic")
                    || lens.lens_name.contains("bge")
                    || lens.lens_name.contains("embedding")
                    || lens.lens_name.contains("colbert"))
        })
        .count()
}

fn build_warm_template_panel(
    home: &Path,
    selector: &str,
    now_ms: u64,
    progress_log: &SharedProgressLog,
    load_limit: &WarmLoadLimit,
    load_parallelism: usize,
) -> CliResult<SavedTemplatePanelBuild> {
    let store = template_store::TemplateStore::open(home);
    let mut template = store.load(selector)?;
    template.validate()?;
    let a37 = template.a37_admission();
    let template_id = template_store::id_for_loaded(&template)?;
    let mut registry = Registry::new();
    let registered_lenses_added = register_and_prime_warm_lenses_parallel(
        &mut registry,
        &mut template,
        progress_log,
        selector,
        load_limit,
        load_parallelism,
    )?;
    let panel = template.to_target_panel(now_ms);
    let content_lens_count = a37.content_lens_count.max(
        panel
            .slots
            .iter()
            .filter(|slot| is_content_slot(slot))
            .count(),
    );
    Ok(SavedTemplatePanelBuild {
        template_id,
        template_name: template.name,
        panel,
        registry,
        content_lens_count,
        a37_gate_eligible: a37.gate_eligible,
        a37_status: a37.status,
        registered_lenses_added,
    })
}

#[derive(Clone)]
struct WarmLensTask {
    template_idx: usize,
    position: usize,
    total: usize,
    lens: template_store::TemplateLensRef,
}

struct WarmPreparedLens {
    task: WarmLensTask,
    prepared: PreparedRuntimeLens,
    prepare_ms: u128,
}

fn register_and_prime_warm_lenses_parallel(
    registry: &mut Registry,
    template: &mut template_store::SavedPanelTemplate,
    progress_log: &SharedProgressLog,
    selector: &str,
    load_limit: &WarmLoadLimit,
    load_parallelism: usize,
) -> CliResult<usize> {
    let tasks = warm_lens_tasks(template);
    let prepared =
        prepare_warm_lenses_parallel(tasks, selector, load_limit, load_parallelism, progress_log)?;
    register_prepared_warm_lenses(registry, template, prepared, selector, progress_log)
}

fn warm_lens_tasks(template: &template_store::SavedPanelTemplate) -> Vec<WarmLensTask> {
    let total = template.lenses.len();
    let mut tasks = template
        .lenses
        .iter()
        .cloned()
        .enumerate()
        .map(|(template_idx, lens)| WarmLensTask {
            template_idx,
            position: template_idx + 1,
            total,
            lens,
        })
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        warm_prepare_weight(&right.lens)
            .cmp(&warm_prepare_weight(&left.lens))
            .then_with(|| left.lens.slot_key.cmp(&right.lens.slot_key))
    });
    tasks
}

fn warm_prepare_weight(lens: &template_store::TemplateLensRef) -> (u8, u64) {
    let placement_rank = if lens.placement == Placement::Gpu {
        1
    } else {
        0
    };
    (placement_rank, lens.cost.vram_bytes)
}

fn prepare_warm_lenses_parallel(
    tasks: Vec<WarmLensTask>,
    selector: &str,
    load_limit: &WarmLoadLimit,
    load_parallelism: usize,
    progress_log: &SharedProgressLog,
) -> CliResult<Vec<WarmPreparedLens>> {
    let total = tasks.len();
    if total == 0 {
        return Ok(Vec::new());
    }
    let worker_count = load_parallelism.min(total).max(1);
    let mut start = run_progress_record(selector, "parallel_prepare_start");
    start.lens_count = Some(total);
    start.load_parallelism = Some(worker_count);
    append_shared_progress(progress_log, &start)?;

    let queue = Arc::new(Mutex::new(VecDeque::from(tasks)));
    let (tx, rx) = mpsc::channel();
    for _ in 0..worker_count {
        let queue = queue.clone();
        let tx = tx.clone();
        let selector = selector.to_string();
        let progress_log = progress_log.clone();
        thread::spawn(move || {
            loop {
                let task = match queue.lock() {
                    Ok(mut guard) => guard.pop_front(),
                    Err(_) => {
                        let _ = tx.send(Err(CliError::from(CalyxError::lens_unreachable(
                            "panel warm prepare queue mutex was poisoned",
                        ))));
                        return;
                    }
                };
                let Some(task) = task else {
                    return;
                };
                if tx
                    .send(prepare_warm_lens(task, &selector, &progress_log))
                    .is_err()
                {
                    return;
                }
            }
        });
    }
    drop(tx);

    let mut prepared = Vec::with_capacity(total);
    while prepared.len() < total {
        let item = load_limit.recv(
            &rx,
            WarmLoadWait {
                selector,
                phase: "parallel_prepare_prime",
                completed: prepared.len(),
                total,
                load_parallelism: worker_count,
                progress_log,
            },
        )??;
        prepared.push(item);
    }
    prepared.sort_by_key(|item| item.task.template_idx);

    let mut ok = run_progress_record(selector, "parallel_prepare_ok");
    ok.elapsed_ms = Some(load_limit.elapsed_ms());
    ok.lens_count = Some(total);
    ok.load_parallelism = Some(worker_count);
    append_shared_progress(progress_log, &ok)?;
    Ok(prepared)
}

fn prepare_warm_lens(
    task: WarmLensTask,
    selector: &str,
    progress_log: &SharedProgressLog,
) -> CliResult<WarmPreparedLens> {
    append_shared_progress(
        progress_log,
        &task_progress_record(selector, "prepare_start", &task),
    )?;
    let started = Instant::now();
    let result = (|| {
        let spec = lens_spec_from_manifest_path(Path::new(&task.lens.manifest))?;
        let spec_lens_id = spec.lens_id();
        if spec_lens_id != task.lens.lens_id {
            return Err(template_store::template_error(
                template_store::TEMPLATE_INVALID,
                format!(
                    "manifest {} no longer resolves to {}",
                    task.lens.manifest, task.lens.lens_id
                ),
                "rebuild the template from the current frozen lens manifest",
            ));
        }
        let mut start = task_progress_record(selector, "runtime_prepare_start", &task);
        start.runtime = Some(runtime_name(&spec.runtime).to_string());
        start.runtime_detail = Some(runtime_detail(&spec.runtime));
        append_shared_progress(progress_log, &start)?;
        prepare_manifest_runtime(spec).map_err(CliError::from)
    })();
    match result {
        Ok(prepared) => {
            let mut record = task_progress_record(selector, "runtime_prepare_ok", &task);
            record.runtime = Some(runtime_name(&prepared.spec.runtime).to_string());
            record.runtime_detail = Some(runtime_detail(&prepared.spec.runtime));
            record.elapsed_ms = Some(started.elapsed().as_millis());
            append_shared_progress(progress_log, &record)?;
            prime_prepared_warm_lens(&task, &prepared, selector, progress_log)?;
            Ok(WarmPreparedLens {
                task,
                prepared,
                prepare_ms: started.elapsed().as_millis(),
            })
        }
        Err(error) => {
            let mut record = task_progress_record(selector, "runtime_prepare_error", &task);
            record.elapsed_ms = Some(started.elapsed().as_millis());
            record.error_code = Some(error.code().to_string());
            record.error_message = Some(error.message().to_string());
            record.remediation = Some(error.remediation().to_string());
            append_shared_progress(progress_log, &record)?;
            Err(error)
        }
    }
}

fn prime_prepared_warm_lens(
    task: &WarmLensTask,
    prepared: &PreparedRuntimeLens,
    selector: &str,
    progress_log: &SharedProgressLog,
) -> CliResult {
    let lens = &task.lens;
    let runtime_lens_id = prepared.lens.id();
    let spec = &prepared.spec;
    append_shared_progress(
        progress_log,
        &prime_progress_record(
            selector,
            "prime_start",
            task.position,
            task.total,
            lens,
            runtime_lens_id,
            &spec.runtime,
        ),
    )?;
    let input = Input::new(lens.modality, probe_bytes(lens.modality)?);
    let started = Instant::now();
    let vector = match prepared.lens.measure(&input) {
        Ok(vector) => vector,
        Err(error) => {
            append_shared_progress(
                progress_log,
                &prime_error_record(
                    selector,
                    task.position,
                    task.total,
                    lens,
                    runtime_lens_id,
                    &spec.runtime,
                    PrimeErrorEvent {
                        elapsed_ms: started.elapsed().as_millis(),
                        error_code: error.code.to_string(),
                        error_message: error.message.clone(),
                    },
                ),
            )?;
            return Err(warm_prime_error(
                lens,
                spec.name.as_str(),
                &spec.runtime,
                error,
            ));
        }
    };
    if let Err(error) = validate_vector_contract(&vector, lens.shape, spec.norm_policy) {
        append_shared_progress(
            progress_log,
            &prime_error_record(
                selector,
                task.position,
                task.total,
                lens,
                runtime_lens_id,
                &spec.runtime,
                PrimeErrorEvent {
                    elapsed_ms: started.elapsed().as_millis(),
                    error_code: error.code().to_string(),
                    error_message: error.message().to_string(),
                },
            ),
        )?;
        return Err(warm_prime_cli_error(
            lens,
            spec.name.as_str(),
            &spec.runtime,
            error,
        ));
    }
    let mut record = prime_progress_record(
        selector,
        "prime_ok",
        task.position,
        task.total,
        lens,
        runtime_lens_id,
        &spec.runtime,
    );
    record.elapsed_ms = Some(started.elapsed().as_millis());
    let (kind, len) = vector_kind_len(&vector);
    record.vector_kind = Some(kind);
    record.vector_len = Some(len);
    record.norm = Some(slot_norm(&vector));
    record.first_values = Some(slot_prefix(&vector, 4));
    append_shared_progress(progress_log, &record)?;
    Ok(())
}

fn register_prepared_warm_lenses(
    registry: &mut Registry,
    template: &mut template_store::SavedPanelTemplate,
    prepared: Vec<WarmPreparedLens>,
    selector: &str,
    progress_log: &SharedProgressLog,
) -> CliResult<usize> {
    let mut added = 0;
    for item in prepared {
        let lens = &mut template.lenses[item.task.template_idx];
        let spec_lens_id = item.prepared.spec.lens_id();
        if let Some(existing) = registry.find_lens_by_spec_id(spec_lens_id) {
            if registry.lens_spec(existing) != Some(&item.prepared.spec) {
                return Err(template_store::template_error(
                    template_store::TEMPLATE_INVALID,
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
                return Err(template_store::template_error(
                    template_store::TEMPLATE_INVALID,
                    format!("runtime resolved {existing}, expected {expected}"),
                    "recommission the lens so runtime and manifest contracts agree",
                ));
            }
            lens.runtime_lens_id = Some(existing);
            emit_registration_progress_shared(
                progress_log,
                selector,
                "existing_matched",
                item.task.position,
                item.task.total,
                lens,
                Some(item.prepare_ms),
            )?;
            continue;
        }
        emit_registration_progress_shared(
            progress_log,
            selector,
            "runtime_register_start",
            item.task.position,
            item.task.total,
            lens,
            Some(item.prepare_ms),
        )?;
        let registered = register_prepared_manifest_runtime(registry, item.prepared)?;
        if let Some(expected) = lens.runtime_lens_id
            && registered != expected
        {
            return Err(template_store::template_error(
                template_store::TEMPLATE_INVALID,
                format!("runtime registered {registered}, expected {expected}"),
                "recommission the lens so runtime and manifest contracts agree",
            ));
        }
        lens.runtime_lens_id = Some(registered);
        emit_registration_progress_shared(
            progress_log,
            selector,
            "runtime_register_ok",
            item.task.position,
            item.task.total,
            lens,
            Some(item.prepare_ms),
        )?;
        added += 1;
    }
    Ok(added)
}

fn append_shared_progress(
    progress_log: &SharedProgressLog,
    record: &WarmProgressRecord,
) -> CliResult {
    let Some(log) = progress_log else {
        return Ok(());
    };
    let log = log.lock().map_err(|_| {
        CliError::from(CalyxError::lens_unreachable(
            "warm progress log mutex was poisoned",
        ))
    })?;
    log.append(record)
}

fn task_progress_record(
    selector: &str,
    phase: &'static str,
    task: &WarmLensTask,
) -> WarmProgressRecord {
    registration_progress_record(
        selector,
        TemplateLensProgress {
            phase,
            ordinal: task.position,
            total: task.total,
            slot_key: task.lens.slot_key.clone(),
            lens_name: task.lens.lens_name.clone(),
            lens_id: task.lens.lens_id.to_string(),
            runtime_lens_id: task.lens.runtime_lens_id.map(|id| id.to_string()),
            runtime: task.lens.runtime.clone(),
            modality: format!("{:?}", task.lens.modality),
            shape: format!("{:?}", task.lens.shape),
            placement: format!("{:?}", task.lens.placement),
            manifest: task.lens.manifest.clone(),
        },
    )
}

fn emit_registration_progress_shared(
    progress_log: &SharedProgressLog,
    selector: &str,
    phase: &'static str,
    ordinal: usize,
    total: usize,
    lens: &template_store::TemplateLensRef,
    elapsed_ms: Option<u128>,
) -> CliResult {
    let mut record = registration_progress_record(
        selector,
        TemplateLensProgress {
            phase,
            ordinal,
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
        },
    );
    record.elapsed_ms = elapsed_ms;
    append_shared_progress(progress_log, &record)
}

fn prime_progress_record(
    template: &str,
    phase: &str,
    ordinal: usize,
    total: usize,
    lens: &template_store::TemplateLensRef,
    runtime_lens_id: calyx_core::LensId,
    runtime: &LensRuntime,
) -> WarmProgressRecord {
    let mut record = base_progress_record(template, phase);
    record.ordinal = Some(ordinal);
    record.total = Some(total);
    record.key = Some(lens.slot_key.clone());
    record.lens_id = Some(lens.lens_id.to_string());
    record.runtime_lens_id = Some(runtime_lens_id.to_string());
    record.lens_name = Some(lens.lens_name.clone());
    record.runtime = Some(runtime_name(runtime).to_string());
    record.runtime_detail = Some(runtime_detail(runtime));
    record.modality = Some(format!("{:?}", lens.modality));
    record.shape = Some(format!("{:?}", lens.shape));
    record.placement = Some(format!("{:?}", lens.placement));
    record.manifest = Some(lens.manifest.clone());
    record
}

struct PrimeErrorEvent {
    elapsed_ms: u128,
    error_code: String,
    error_message: String,
}

fn prime_error_record(
    template: &str,
    ordinal: usize,
    total: usize,
    lens: &template_store::TemplateLensRef,
    runtime_lens_id: calyx_core::LensId,
    runtime: &LensRuntime,
    error: PrimeErrorEvent,
) -> WarmProgressRecord {
    let mut record = prime_progress_record(
        template,
        "prime_error",
        ordinal,
        total,
        lens,
        runtime_lens_id,
        runtime,
    );
    record.elapsed_ms = Some(error.elapsed_ms);
    record.error_code = Some(error.error_code);
    record.error_message = Some(error.error_message);
    record
}

fn warm_prime_error(
    lens: &template_store::TemplateLensRef,
    spec_name: &str,
    runtime: &LensRuntime,
    error: CalyxError,
) -> CliError {
    CliError::from(CalyxError::lens_unreachable(format!(
        "panel warm prime failed key={} lens={} spec_name={} runtime={} runtime_detail={} \
         modality={:?} shape={:?} placement={:?}; cause_code={}; cause={}",
        lens.slot_key,
        lens.lens_id,
        spec_name,
        runtime_name(runtime),
        runtime_detail(runtime),
        lens.modality,
        lens.shape,
        lens.placement,
        error.code,
        error.message
    )))
}

fn warm_prime_cli_error(
    lens: &template_store::TemplateLensRef,
    spec_name: &str,
    runtime: &LensRuntime,
    error: CliError,
) -> CliError {
    CliError::from(CalyxError::lens_unreachable(format!(
        "panel warm prime failed key={} lens={} spec_name={} runtime={} runtime_detail={} \
         modality={:?} shape={:?} placement={:?}; cause_code={}; cause={}",
        lens.slot_key,
        lens.lens_id,
        spec_name,
        runtime_name(runtime),
        runtime_detail(runtime),
        lens.modality,
        lens.shape,
        lens.placement,
        error.code(),
        error.message()
    )))
}

fn is_content_slot(slot: &Slot) -> bool {
    slot.state == SlotState::Active && slot.modality != Modality::Structured
}

fn estimate_resident_vram_mib(declared_bytes: u64, multiplier_milli: u64) -> u64 {
    let adjusted = (declared_bytes as u128).saturating_mul(multiplier_milli as u128);
    let adjusted_bytes = adjusted.saturating_add(999) / 1000;
    let mib = 1024_u128 * 1024_u128;
    ((adjusted_bytes.saturating_add(mib - 1)) / mib) as u64
}

fn bytes_to_mib_ceil(bytes: u64) -> u64 {
    bytes.saturating_add((1024 * 1024) - 1) / (1024 * 1024)
}

fn multiplier_to_f32(multiplier_milli: u64) -> f32 {
    multiplier_milli as f32 / 1000.0
}

impl Flags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self::default();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--home" => {
                    idx += 1;
                    flags.home = Some(value(args, idx, "--home")?.into());
                }
                "--template" => {
                    idx += 1;
                    flags.template = Some(value(args, idx, "--template")?.to_string());
                }
                "--hold-secs" => {
                    idx += 1;
                    let raw = value(args, idx, "--hold-secs")?;
                    flags.hold_secs = raw.parse::<u64>().map_err(|err| {
                        CliError::usage(format!("parse --hold-secs {raw}: {err}"))
                    })?;
                }
                "--out" => {
                    idx += 1;
                    flags.out = Some(value(args, idx, "--out")?.into());
                }
                "--progress-out" => {
                    idx += 1;
                    flags.progress_out = Some(value(args, idx, "--progress-out")?.into());
                }
                "--max-resident-vram-mib" => {
                    idx += 1;
                    let raw = value(args, idx, "--max-resident-vram-mib")?;
                    flags.max_resident_vram_mib = Some(raw.parse::<u64>().map_err(|err| {
                        CliError::usage(format!("parse --max-resident-vram-mib {raw}: {err}"))
                    })?);
                }
                "--resident-overhead-multiplier" => {
                    idx += 1;
                    let raw = value(args, idx, "--resident-overhead-multiplier")?;
                    flags.resident_overhead_multiplier_milli = Some(parse_multiplier_milli(raw)?);
                }
                "--max-load-secs" => {
                    idx += 1;
                    let raw = value(args, idx, "--max-load-secs")?;
                    flags.max_load_secs = Some(raw.parse::<u64>().map_err(|err| {
                        CliError::usage(format!("parse --max-load-secs {raw}: {err}"))
                    })?);
                }
                "--load-parallelism" => {
                    idx += 1;
                    let raw = value(args, idx, "--load-parallelism")?;
                    let value = raw.parse::<usize>().map_err(|err| {
                        CliError::usage(format!("parse --load-parallelism {raw}: {err}"))
                    })?;
                    if value == 0 {
                        return Err(CliError::usage("--load-parallelism must be > 0"));
                    }
                    flags.load_parallelism = Some(value);
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected panel warm flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        Ok(flags)
    }
}

fn probe_panel(
    build: &SavedTemplatePanelBuild,
    progress_log: Option<&WarmProgressLog>,
    template: &str,
) -> CliResult<Vec<WarmProbeReport>> {
    let mut seen = BTreeSet::new();
    let mut reports = Vec::new();
    let slots = content_slots(build).collect::<Vec<_>>();
    let total = slots
        .iter()
        .map(|slot| slot.lens_id)
        .collect::<BTreeSet<_>>()
        .len();
    let mut ordinal = 0;
    for slot in slots {
        if !seen.insert(slot.lens_id) {
            continue;
        }
        ordinal += 1;
        let spec = build.registry.lens_spec(slot.lens_id).ok_or_else(|| {
            CliError::from(CalyxError::registry_unavailable(format!(
                "warm probe slot={} key={} lens={} has no LensSpec in registry",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id
            )))
        })?;
        if let Some(log) = progress_log {
            log.append(&probe_progress_record(
                template,
                ProbeProgressEvent {
                    phase: "probe_start",
                    ordinal,
                    total,
                    slot,
                    spec_name: spec.name.as_str(),
                    runtime: &spec.runtime,
                    elapsed_ms: None,
                    error: None,
                },
            ))?;
        }
        let input = Input::new(slot.modality, probe_bytes(slot.modality)?);
        let started = Instant::now();
        let vector = match build.registry.measure(slot.lens_id, &input) {
            Ok(vector) => vector,
            Err(error) => {
                if let Some(log) = progress_log {
                    log.append(&probe_progress_record(
                        template,
                        ProbeProgressEvent {
                            phase: "probe_error",
                            ordinal,
                            total,
                            slot,
                            spec_name: spec.name.as_str(),
                            runtime: &spec.runtime,
                            elapsed_ms: Some(started.elapsed().as_millis()),
                            error: Some((error.code, error.message.as_str())),
                        },
                    ))?;
                }
                return Err(warm_error(slot, spec.name.as_str(), &spec.runtime, error));
            }
        };
        if let Err(error) = validate_vector_contract(&vector, slot.shape, spec.norm_policy) {
            if let Some(log) = progress_log {
                log.append(&probe_progress_record(
                    template,
                    ProbeProgressEvent {
                        phase: "probe_error",
                        ordinal,
                        total,
                        slot,
                        spec_name: spec.name.as_str(),
                        runtime: &spec.runtime,
                        elapsed_ms: Some(started.elapsed().as_millis()),
                        error: Some((error.code(), error.message())),
                    },
                ))?;
            }
            return Err(warm_cli_error(
                slot,
                spec.name.as_str(),
                &spec.runtime,
                error,
            ));
        }
        let report = report_probe(
            slot,
            spec.name.as_str(),
            &spec.runtime,
            &vector,
            started.elapsed().as_millis(),
        );
        if let Some(log) = progress_log {
            log.append(&probe_ok_record(
                template,
                ordinal,
                total,
                slot,
                spec.name.as_str(),
                &spec.runtime,
                &report,
            ))?;
        }
        reports.push(report);
    }
    Ok(reports)
}

fn report_probe(
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
    vector: &SlotVector,
    elapsed_ms: u128,
) -> WarmProbeReport {
    let (vector_kind, vector_len) = vector_kind_len(vector);
    WarmProbeReport {
        slot: slot.slot_id.get(),
        key: slot.slot_key.key().to_string(),
        lens_id: slot.lens_id.to_string(),
        spec_name: spec_name.to_string(),
        runtime: runtime_name(runtime),
        runtime_detail: runtime_detail(runtime),
        modality: slot.modality,
        shape: slot.shape,
        placement: slot.resource.placement,
        vector_kind,
        vector_len,
        norm: slot_norm(vector),
        first_values: slot_prefix(vector, 4),
        elapsed_ms,
    }
}

fn run_progress_record(template: &str, phase: &str) -> WarmProgressRecord {
    base_progress_record(template, phase)
}

fn registration_progress_record(template: &str, event: TemplateLensProgress) -> WarmProgressRecord {
    let mut record = base_progress_record(template, event.phase);
    record.ordinal = Some(event.ordinal);
    record.total = Some(event.total);
    record.key = Some(event.slot_key);
    record.lens_id = Some(event.lens_id);
    record.runtime_lens_id = event.runtime_lens_id;
    record.lens_name = Some(event.lens_name);
    record.runtime = Some(event.runtime);
    record.modality = Some(event.modality);
    record.shape = Some(event.shape);
    record.placement = Some(event.placement);
    record.manifest = Some(event.manifest);
    record
}

struct ProbeProgressEvent<'a> {
    phase: &'a str,
    ordinal: usize,
    total: usize,
    slot: &'a Slot,
    spec_name: &'a str,
    runtime: &'a LensRuntime,
    elapsed_ms: Option<u128>,
    error: Option<(&'a str, &'a str)>,
}

fn probe_progress_record(template: &str, event: ProbeProgressEvent<'_>) -> WarmProgressRecord {
    let mut record = slot_progress_record(
        template,
        event.phase,
        event.ordinal,
        event.total,
        event.slot,
        event.spec_name,
        event.runtime,
    );
    record.elapsed_ms = event.elapsed_ms;
    if let Some((code, message)) = event.error {
        record.error_code = Some(code.to_string());
        record.error_message = Some(message.to_string());
    }
    record
}

fn probe_ok_record(
    template: &str,
    ordinal: usize,
    total: usize,
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
    report: &WarmProbeReport,
) -> WarmProgressRecord {
    let mut record = slot_progress_record(
        template, "probe_ok", ordinal, total, slot, spec_name, runtime,
    );
    record.elapsed_ms = Some(report.elapsed_ms);
    record.vector_kind = Some(report.vector_kind);
    record.vector_len = Some(report.vector_len);
    record.norm = Some(report.norm);
    record.first_values = Some(report.first_values.clone());
    record
}

fn slot_progress_record(
    template: &str,
    phase: &str,
    ordinal: usize,
    total: usize,
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
) -> WarmProgressRecord {
    let mut record = base_progress_record(template, phase);
    record.ordinal = Some(ordinal);
    record.total = Some(total);
    record.slot = Some(slot.slot_id.get());
    record.key = Some(slot.slot_key.key().to_string());
    record.lens_id = Some(slot.lens_id.to_string());
    record.spec_name = Some(spec_name.to_string());
    record.runtime = Some(runtime_name(runtime).to_string());
    record.runtime_detail = Some(runtime_detail(runtime));
    record.modality = Some(format!("{:?}", slot.modality));
    record.shape = Some(format!("{:?}", slot.shape));
    record.placement = Some(format!("{:?}", slot.resource.placement));
    record
}

fn base_progress_record(template: &str, phase: &str) -> WarmProgressRecord {
    WarmProgressRecord {
        schema: PROGRESS_SCHEMA,
        timestamp_ms: now_ms(),
        process_id: std::process::id(),
        template_selector: template.to_string(),
        phase: phase.to_string(),
        ordinal: None,
        total: None,
        slot: None,
        key: None,
        lens_id: None,
        runtime_lens_id: None,
        lens_name: None,
        spec_name: None,
        runtime: None,
        runtime_detail: None,
        modality: None,
        shape: None,
        placement: None,
        manifest: None,
        elapsed_ms: None,
        lens_count: None,
        semantic_lens_count: None,
        declared_template_vram_mib: None,
        estimated_resident_vram_mib: None,
        max_resident_vram_mib: None,
        resident_overhead_multiplier: None,
        load_parallelism: None,
        vector_kind: None,
        vector_len: None,
        norm: None,
        first_values: None,
        error_code: None,
        error_message: None,
        remediation: None,
    }
}

fn content_slots(build: &SavedTemplatePanelBuild) -> impl Iterator<Item = &Slot> {
    build.panel.slots.iter().filter(|slot| {
        slot.state == SlotState::Active && !slot.retrieval_only && !slot.excluded_from_dedup
    })
}

fn vector_kind_len(vector: &SlotVector) -> (&'static str, usize) {
    match vector {
        SlotVector::Dense { data, .. } => ("dense", data.len()),
        SlotVector::Sparse { entries, .. } => ("sparse", entries.len()),
        SlotVector::Multi { tokens, .. } => ("multi", tokens.len()),
        SlotVector::Absent { .. } => ("absent", 0),
    }
}

fn probe_bytes(modality: Modality) -> CliResult<Vec<u8>> {
    match modality {
        Modality::Text => Ok(b"Calyx Blackwell warm-load probe: semantic text path.".to_vec()),
        Modality::Code => Ok(b"fn calyx_warm_probe() -> u32 { 42 }".to_vec()),
        Modality::Image => Ok(one_pixel_png().to_vec()),
        Modality::Audio => Ok(warm_audio_wav()),
        Modality::Video => Ok(b"RIFF\x24\x00\x00\x00AVI LIST calyx warm probe".to_vec()),
        Modality::Protein => Ok(b"MKTFFVLLL".to_vec()),
        Modality::Dna => Ok(b"ACGTNACGTN".to_vec()),
        Modality::Molecule => Ok(b"CCO".to_vec()),
        Modality::Structured => Ok(br#"{"calyx_warm_probe":true,"value":42}"#.to_vec()),
        Modality::Mixed => Ok(b"Calyx mixed modality warm probe with text and metadata.".to_vec()),
    }
}

fn warm_error(slot: &Slot, spec_name: &str, runtime: &LensRuntime, error: CalyxError) -> CliError {
    CliError::from(CalyxError {
        code: error.code,
        message: warm_error_message(slot, spec_name, runtime, error.code, &error.message),
        remediation: error.remediation,
    })
}

fn warm_cli_error(
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
    error: CliError,
) -> CliError {
    CliError::from(CalyxError {
        code: error.code(),
        message: warm_error_message(slot, spec_name, runtime, error.code(), error.message()),
        remediation: error.remediation(),
    })
}

fn warm_error_message(
    slot: &Slot,
    spec_name: &str,
    runtime: &LensRuntime,
    code: &str,
    message: &str,
) -> String {
    format!(
        "panel warm failed slot={} key={} lens={} spec_name={} runtime={} runtime_detail={} modality={:?} shape={:?} placement={:?}; cause_code={code}; cause={message}",
        slot.slot_id.get(),
        slot.slot_key.key(),
        slot.lens_id,
        spec_name,
        runtime_name(runtime),
        runtime_detail(runtime),
        slot.modality,
        slot.shape,
        slot.resource.placement,
    )
}

fn runtime_detail(runtime: &LensRuntime) -> String {
    match runtime {
        LensRuntime::Algorithmic { kind } => kind.clone(),
        LensRuntime::TeiHttp { endpoint } => endpoint.clone(),
        LensRuntime::CandleLocal {
            model_id,
            dtype,
            pooling,
            ..
        } => format!("{model_id};dtype={dtype};pooling={pooling}"),
        LensRuntime::Onnx { model_id, .. }
        | LensRuntime::OnnxColbert { model_id, .. }
        | LensRuntime::FastembedSparse { model_id, .. }
        | LensRuntime::FastembedReranker { model_id, .. } => model_id.clone(),
        LensRuntime::FastembedBgem3 {
            model_id, output, ..
        } => format!("{model_id};output={output:?}"),
        LensRuntime::FastembedQwen3 {
            model_id, dtype, ..
        } => format!("{model_id};dtype={dtype}"),
        LensRuntime::StaticLookup {
            embeddings_file,
            tokenizer,
            ..
        } => format!(
            "embeddings={};tokenizer={}",
            embeddings_file.display(),
            tokenizer.display()
        ),
        LensRuntime::MultimodalAdapter {
            axis,
            model_id,
            adapter_config,
            ..
        } => format!(
            "axis={axis};model_id={model_id};adapter_config={}",
            adapter_config
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "missing".to_string())
        ),
        LensRuntime::ExternalCmd { cmd, args } => format!("{cmd} {}", args.join(" ")),
    }
}

fn write_report(path: &Path, report: &WarmReport) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(report)?)?;
    Ok(())
}

fn derive_progress_path(out: &Path) -> PathBuf {
    let file_name = out
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("warm.json");
    out.with_file_name(format!("{file_name}.progress.jsonl"))
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn parse_multiplier_milli(raw: &str) -> CliResult<u64> {
    let value = raw.parse::<f64>().map_err(|err| {
        CliError::usage(format!("parse --resident-overhead-multiplier {raw}: {err}"))
    })?;
    if !value.is_finite() || value <= 0.0 {
        return Err(CliError::usage(format!(
            "--resident-overhead-multiplier must be a positive finite number, got {raw}"
        )));
    }
    Ok((value * 1000.0).ceil() as u64)
}

fn calyx_home() -> CliResult<PathBuf> {
    env::var_os("CALYX_HOME")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::usage("CALYX_HOME is required or pass --home <dir>"))
}

fn source_of_truth(home: &Path, template_id: &str) -> String {
    format!(
        "{} + {} + in-process Registry + emitted warm report",
        home.join("panels")
            .join("templates")
            .join("index.json")
            .display(),
        home.join("panels")
            .join("templates")
            .join("objects")
            .join(format!("{template_id}.json"))
            .display()
    )
}

fn one_pixel_png() -> &'static [u8] {
    &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xdd, 0x8d, 0xb0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}

fn warm_audio_wav() -> Vec<u8> {
    const SAMPLE_RATE: u32 = 16_000;
    const SAMPLES: u32 = SAMPLE_RATE;
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;
    const BYTES_PER_SAMPLE: u16 = BITS_PER_SAMPLE / 8;
    const DATA_BYTES: u32 = SAMPLES * BYTES_PER_SAMPLE as u32;

    let mut wav = Vec::with_capacity(44 + DATA_BYTES as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + DATA_BYTES).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * CHANNELS as u32 * BYTES_PER_SAMPLE as u32).to_le_bytes());
    wav.extend_from_slice(&(CHANNELS * BYTES_PER_SAMPLE).to_le_bytes());
    wav.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&DATA_BYTES.to_le_bytes());
    for index in 0..SAMPLES {
        let phase = index as f32 * 2.0 * std::f32::consts::PI * 440.0 / SAMPLE_RATE as f32;
        let sample = (phase.sin() * 0.25 * i16::MAX as f32) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_vram_estimate_ceilings_declared_bytes() {
        let declared = 10_u64 * 1024 * 1024 * 1024;
        assert_eq!(estimate_resident_vram_mib(declared, 2100), 21 * 1024);
    }

    #[test]
    fn parses_warm_limit_flags() {
        let flags = Flags::parse(&[
            "--template".to_string(),
            "blackwell-42".to_string(),
            "--max-resident-vram-mib".to_string(),
            "22528".to_string(),
            "--resident-overhead-multiplier".to_string(),
            "2.1".to_string(),
            "--max-load-secs".to_string(),
            "30".to_string(),
            "--load-parallelism".to_string(),
            "4".to_string(),
        ])
        .unwrap();

        assert_eq!(flags.template.as_deref(), Some("blackwell-42"));
        assert_eq!(flags.max_resident_vram_mib, Some(22 * 1024));
        assert_eq!(flags.resident_overhead_multiplier_milli, Some(2100));
        assert_eq!(flags.max_load_secs, Some(30));
        assert_eq!(flags.load_parallelism, Some(4));
    }

    #[test]
    fn warm_defaults_use_sixty_second_template_parallel_readiness() {
        let flags = Flags::parse(&["--template".to_string(), "blackwell-42".to_string()]).unwrap();

        assert_eq!(flags.max_load_secs.unwrap_or(DEFAULT_MAX_LOAD_SECS), 60);
        assert_eq!(flags.load_parallelism, None);
        assert_eq!(default_load_parallelism(23), 8);
        assert_eq!(default_load_parallelism(4), 4);
        assert_eq!(default_load_parallelism(0), 1);
    }

    #[test]
    fn rejects_zero_load_parallelism() {
        let error = Flags::parse(&[
            "--template".to_string(),
            "blackwell-42".to_string(),
            "--load-parallelism".to_string(),
            "0".to_string(),
        ])
        .unwrap_err();

        assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
        assert!(error.message().contains("--load-parallelism must be > 0"));
    }

    #[test]
    fn rejects_non_positive_resident_multiplier() {
        let error = parse_multiplier_milli("0").unwrap_err();
        assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
        assert!(
            error
                .message()
                .contains("--resident-overhead-multiplier must be a positive")
        );
    }
}
