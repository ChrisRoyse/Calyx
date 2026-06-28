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

mod load_limit;

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

mod preflight;
#[cfg(test)]
use preflight::estimate_resident_vram_mib;
use preflight::{
    build_warm_template_panel, default_load_parallelism, multiplier_to_f32, warm_preflight,
};

mod load;

mod load_progress;

mod flags;

mod probe;
use probe::{content_slots, probe_panel, run_progress_record};

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
mod tests;
