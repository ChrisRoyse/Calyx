use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::{Input, Modality, SlotShape, SlotVector};
use calyx_registry::{Lens, LensRuntime, LensSpec, StaticLookupLens, lens_spec_from_manifest_path};
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LensCatalog {
    lenses: Vec<LensCatalogEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LensCatalogEntry {
    lens_id: String,
    name: String,
    modality: String,
    runtime: String,
    dim: u32,
    weights_sha256: String,
    manifest: PathBuf,
}

#[derive(Serialize)]
struct AddReport {
    catalog: PathBuf,
    lens_id: String,
    name: String,
    manifest: PathBuf,
    count: usize,
}

#[derive(Serialize)]
struct ListReport {
    catalog: PathBuf,
    count: usize,
    lenses: Vec<LensCatalogEntry>,
}

#[derive(Serialize)]
struct ExplainReport {
    manifest: PathBuf,
    lens_id: String,
    name: String,
    runtime: String,
    dtype: String,
    dim: u32,
    rows: Option<u32>,
    norm: f32,
    first_values: Vec<f32>,
    total_ms: f32,
    ms_per_input: f32,
    vram_bytes: u64,
    vram_mb: f32,
}

pub(crate) fn run(topic: &str, rest: &[String]) -> CliResult {
    match topic {
        "add" => add(rest),
        "list" => list(rest),
        "explain" => explain(rest),
        other => Err(CliError::usage(format!(
            "unknown lens subcommand {other}; expected add, list, or explain"
        ))),
    }
}

fn add(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    flags.reject_measure_flags("calyx lens add")?;
    let manifest = flags
        .manifest
        .ok_or_else(|| CliError::usage("calyx lens add requires --manifest <path>"))?;
    let spec = lens_spec_from_manifest_path(&manifest)?;
    let catalog_path = catalog_path(flags.home.as_deref())?;
    let mut catalog = read_catalog(&catalog_path)?;
    let entry = entry_from_spec(&spec, manifest);
    catalog.lenses.retain(|item| item.lens_id != entry.lens_id);
    catalog.lenses.push(entry.clone());
    catalog
        .lenses
        .sort_by(|left, right| left.lens_id.cmp(&right.lens_id));
    write_catalog(&catalog_path, &catalog)?;
    print_json(&AddReport {
        catalog: catalog_path,
        lens_id: entry.lens_id,
        name: entry.name,
        manifest: entry.manifest,
        count: catalog.lenses.len(),
    })
}

fn list(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    flags.reject_measure_flags("calyx lens list")?;
    if flags.manifest.is_some() {
        return Err(CliError::usage(
            "calyx lens list does not accept --manifest",
        ));
    }
    let catalog_path = catalog_path(flags.home.as_deref())?;
    let catalog = read_catalog(&catalog_path)?;
    print_json(&ListReport {
        catalog: catalog_path,
        count: catalog.lenses.len(),
        lenses: catalog.lenses,
    })
}

fn explain(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let manifest = flags
        .manifest
        .ok_or_else(|| CliError::usage("calyx lens explain requires --manifest <path>"))?;
    let repeat = flags.repeat.unwrap_or(1);
    if repeat == 0 {
        return Err(CliError::usage("--repeat must be > 0"));
    }
    let spec = lens_spec_from_manifest_path(&manifest)?;
    let input = flags
        .input
        .unwrap_or_else(|| "Calyx static lookup explain probe".to_string());
    match &spec.runtime {
        LensRuntime::StaticLookup { .. } => explain_static_lookup(manifest, spec, input, repeat),
        other => Err(CliError::usage(format!(
            "calyx lens explain currently supports static_lookup manifests, got {}",
            runtime_name(other)
        ))),
    }
}

fn explain_static_lookup(
    manifest: PathBuf,
    spec: LensSpec,
    input: String,
    repeat: usize,
) -> CliResult {
    let lens = StaticLookupLens::from_lens_spec(&spec)?;
    let lens_id = spec.lens_id().to_string();
    let probe = Input::new(Modality::Text, input.into_bytes());
    let started = Instant::now();
    let mut last = None;
    for _ in 0..repeat {
        last = Some(lens.measure(&probe)?);
    }
    let total_ms = started.elapsed().as_secs_f64() as f32 * 1000.0;
    let vector = last.ok_or_else(|| CliError::usage("repeat produced no vector"))?;
    print_json(&ExplainReport {
        manifest,
        lens_id,
        name: spec.name,
        runtime: runtime_name(&spec.runtime).to_string(),
        dtype: lens.dtype().as_str().to_string(),
        dim: dim(spec.output),
        rows: Some(lens.row_count()),
        norm: slot_norm(&vector),
        first_values: slot_prefix(&vector, 4),
        total_ms,
        ms_per_input: total_ms / repeat as f32,
        vram_bytes: 0,
        vram_mb: 0.0,
    })
}

#[derive(Default)]
struct Flags {
    manifest: Option<PathBuf>,
    home: Option<PathBuf>,
    input: Option<String>,
    repeat: Option<usize>,
}

impl Flags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut flags = Self::default();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--manifest" => {
                    idx += 1;
                    flags.manifest = Some(value(args, idx, "--manifest")?.into());
                }
                "--home" => {
                    idx += 1;
                    flags.home = Some(value(args, idx, "--home")?.into());
                }
                "--input" => {
                    idx += 1;
                    flags.input = Some(value(args, idx, "--input")?.to_string());
                }
                "--repeat" => {
                    idx += 1;
                    let raw = value(args, idx, "--repeat")?;
                    flags.repeat = Some(raw.parse().map_err(|err| {
                        CliError::usage(format!("parse --repeat value {raw}: {err}"))
                    })?);
                }
                other => {
                    return Err(CliError::usage(format!("unexpected lens flag {other}")));
                }
            }
            idx += 1;
        }
        Ok(flags)
    }

    fn reject_measure_flags(&self, command: &str) -> CliResult {
        if self.input.is_some() || self.repeat.is_some() {
            return Err(CliError::usage(format!(
                "{command} does not accept --input or --repeat"
            )));
        }
        Ok(())
    }
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn catalog_path(home: Option<&Path>) -> CliResult<PathBuf> {
    let root = match home {
        Some(path) => path.to_path_buf(),
        None => env::var_os("CALYX_HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CliError::usage("CALYX_HOME is required or pass --home <dir>"))?,
    };
    Ok(root.join("lenses").join("registry.json"))
}

fn read_catalog(path: &Path) -> CliResult<LensCatalog> {
    if !path.exists() {
        return Ok(LensCatalog { lenses: Vec::new() });
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|err| CliError::usage(format!("parse lens catalog {}: {err}", path.display())))
}

fn write_catalog(path: &Path, catalog: &LensCatalog) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(catalog)
        .map_err(|err| CliError::usage(format!("serialize lens catalog: {err}")))?;
    fs::write(path, bytes)?;
    Ok(())
}

fn entry_from_spec(spec: &LensSpec, manifest: PathBuf) -> LensCatalogEntry {
    LensCatalogEntry {
        lens_id: spec.lens_id().to_string(),
        name: spec.name.clone(),
        modality: format!("{:?}", spec.modality).to_lowercase(),
        runtime: runtime_name(&spec.runtime).to_string(),
        dim: dim(spec.output),
        weights_sha256: hex_from_bytes(&spec.weights_sha256),
        manifest,
    }
}

fn runtime_name(runtime: &LensRuntime) -> &'static str {
    match runtime {
        LensRuntime::Algorithmic { .. } => "algorithmic",
        LensRuntime::TeiHttp { .. } => "tei_http",
        LensRuntime::CandleLocal { .. } => "candle_local",
        LensRuntime::Onnx { .. } => "onnx",
        LensRuntime::StaticLookup { .. } => "static_lookup",
        LensRuntime::ExternalCmd { .. } => "external_cmd",
    }
}

fn dim(shape: SlotShape) -> u32 {
    match shape {
        SlotShape::Dense(dim) | SlotShape::Sparse(dim) => dim,
        SlotShape::Multi { token_dim } => token_dim,
    }
}

fn hex_from_bytes(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn slot_norm(vector: &SlotVector) -> f32 {
    match vector {
        SlotVector::Dense { data, .. } => {
            data.iter().map(|value| value * value).sum::<f32>().sqrt()
        }
        SlotVector::Sparse { entries, .. } => entries
            .iter()
            .map(|entry| entry.val * entry.val)
            .sum::<f32>()
            .sqrt(),
        SlotVector::Multi { tokens, .. } => tokens
            .iter()
            .flat_map(|token| token.iter())
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt(),
        SlotVector::Absent { .. } => 0.0,
    }
}

fn slot_prefix(vector: &SlotVector, limit: usize) -> Vec<f32> {
    match vector {
        SlotVector::Dense { data, .. } => data.iter().take(limit).copied().collect(),
        SlotVector::Sparse { dim, entries } => {
            let mut values = vec![0.0; (*dim as usize).min(limit)];
            for entry in entries {
                if let Some(value) = values.get_mut(entry.idx as usize) {
                    *value = entry.val;
                }
            }
            values
        }
        SlotVector::Multi { tokens, .. } => tokens
            .first()
            .map(|token| token.iter().take(limit).copied().collect())
            .unwrap_or_default(),
        SlotVector::Absent { .. } => Vec::new(),
    }
}
