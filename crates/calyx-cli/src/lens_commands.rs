use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::SlotShape;
use calyx_registry::{LensRuntime, LensSpec, lens_spec_from_manifest_path};
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

pub(crate) fn run(topic: &str, rest: &[String]) -> CliResult {
    match topic {
        "add" => add(rest),
        "list" => list(rest),
        other => Err(CliError::usage(format!(
            "unknown lens subcommand {other}; expected add or list"
        ))),
    }
}

fn add(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
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

#[derive(Default)]
struct Flags {
    manifest: Option<PathBuf>,
    home: Option<PathBuf>,
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
                other => {
                    return Err(CliError::usage(format!("unexpected lens flag {other}")));
                }
            }
            idx += 1;
        }
        Ok(flags)
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
