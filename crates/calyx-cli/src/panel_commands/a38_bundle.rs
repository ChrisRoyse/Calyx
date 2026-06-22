use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

use calyx_core::CalyxError;
use serde::Serialize;

mod input;
mod model;
mod store;

use input::{base_template_ref, evidence_refs, registry_ref, required_modalities, select_lenses};
use store::{A38BundleStore, BundleSave};

use super::{catalog_path, read_catalog};
use crate::cmd::vault;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_BUDGET_VRAM_MIB: u64 = 20 * 1024;
const A38_BUNDLE_INVALID: &str = "CALYX_A38_BUNDLE_INVALID";
const A38_BUNDLE_INCOMPLETE: &str = "CALYX_A38_BUNDLE_INCOMPLETE";
const A38_BUNDLE_BUDGET_EXCEEDED: &str = "CALYX_A38_BUNDLE_BUDGET_EXCEEDED";
const A38_BUNDLE_BASE_A37_REFUSED: &str = "CALYX_A38_BUNDLE_BASE_A37_REFUSED";
#[cfg(test)]
const A38_BUNDLE_NOT_FOUND: &str = "CALYX_A38_BUNDLE_NOT_FOUND";

#[derive(Default)]
struct Flags {
    home: Option<PathBuf>,
    name: Option<String>,
    base_template: Option<String>,
    include_lenses: Vec<String>,
    required_modalities: Vec<String>,
    evidence: Vec<PathBuf>,
    budget_vram_mib: Option<u64>,
}

#[derive(Serialize)]
struct SaveReport {
    action: &'static str,
    bundle_id: String,
    object_path: PathBuf,
    index_path: PathBuf,
    name: String,
    version: u32,
    base_template_id: String,
    base_a37_status: String,
    content_lens_count: usize,
    modality_counts: BTreeMap<String, usize>,
    evidence_ref_count: usize,
    total_vram_bytes: u64,
    total_vram_mib: f32,
    budget_vram_mib: u64,
    under_budget: bool,
    coverage_status: String,
}

#[derive(Serialize)]
struct ListReport {
    index_path: PathBuf,
    count: usize,
    bundles: Vec<model::BundleSummary>,
}

pub(super) fn run(rest: &[String]) -> CliResult {
    let (command, args) = rest
        .split_first()
        .ok_or_else(|| CliError::usage("calyx panel a38-bundle requires a subcommand"))?;
    match command.as_str() {
        "save" => save(args),
        "list" => list(args),
        other => Err(CliError::usage(format!(
            "unknown panel a38-bundle subcommand {other}; expected save or list"
        ))),
    }
}

fn save(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let home = home(flags.home.clone())?;
    let name = flags
        .name
        .ok_or_else(|| CliError::usage("panel a38-bundle save requires --name <name>"))?;
    let base_selector = flags.base_template.ok_or_else(|| {
        CliError::usage("panel a38-bundle save requires --base-template <name-or-id>")
    })?;
    let budget_vram_mib = flags.budget_vram_mib.unwrap_or(DEFAULT_BUDGET_VRAM_MIB);
    let required_modalities = required_modalities(&flags.required_modalities)?;
    let registry_path = catalog_path(Some(&home))?;
    let catalog = read_catalog(&registry_path)?;
    let registry_ref = registry_ref(&registry_path, catalog.lenses.len())?;
    let lenses = select_lenses(&catalog.lenses, &flags.include_lenses)?;
    let evidence_refs = evidence_refs(&flags.evidence)?;
    let base_template = base_template_ref(&home, &base_selector)?;
    let saved = A38BundleStore::open(&home).save(
        model::BundleDraft {
            name,
            base_template,
            registry_ref,
            required_modalities,
            evidence_refs,
            lenses,
            budget_vram_mib,
        },
        vault::now_ms(),
    )?;
    print_json(&save_report(saved))
}

fn list(args: &[String]) -> CliResult {
    let flags = Flags::parse(args)?;
    let home = home(flags.home)?;
    let store = A38BundleStore::open(&home);
    let bundles = store.list()?;
    print_json(&ListReport {
        index_path: store.index_path(),
        count: bundles.len(),
        bundles,
    })
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
                "--name" => {
                    idx += 1;
                    flags.name = Some(value(args, idx, "--name")?.to_string());
                }
                "--base-template" => {
                    idx += 1;
                    flags.base_template = Some(value(args, idx, "--base-template")?.to_string());
                }
                "--include-lens" | "--lens" => {
                    idx += 1;
                    flags
                        .include_lenses
                        .push(value(args, idx, "--include-lens")?.to_string());
                }
                "--required-modality" => {
                    idx += 1;
                    flags
                        .required_modalities
                        .push(value(args, idx, "--required-modality")?.to_string());
                }
                "--evidence" => {
                    idx += 1;
                    flags.evidence.push(value(args, idx, "--evidence")?.into());
                }
                "--budget-vram-mib" => {
                    idx += 1;
                    let raw = value(args, idx, "--budget-vram-mib")?;
                    flags.budget_vram_mib = Some(raw.parse::<u64>().map_err(|err| {
                        CliError::usage(format!("parse --budget-vram-mib {raw}: {err}"))
                    })?);
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected panel a38-bundle flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        Ok(flags)
    }
}

fn save_report(saved: BundleSave) -> SaveReport {
    SaveReport {
        action: "save",
        bundle_id: saved.bundle_id,
        object_path: saved.object_path,
        index_path: saved.index_path,
        name: saved.bundle.name,
        version: saved.bundle.version,
        base_template_id: saved.bundle.base_template.template_id,
        base_a37_status: saved.bundle.base_template.a37_status,
        content_lens_count: saved.bundle.content_lens_count,
        modality_counts: saved.bundle.modality_counts,
        evidence_ref_count: saved.bundle.evidence_refs.len(),
        total_vram_bytes: saved.bundle.total_vram_bytes,
        total_vram_mib: saved.bundle.total_vram_mib,
        budget_vram_mib: saved.bundle.budget_vram_mib,
        under_budget: saved.bundle.under_budget,
        coverage_status: saved.bundle.coverage_status,
    }
}

fn home(value: Option<PathBuf>) -> CliResult<PathBuf> {
    match value {
        Some(path) => Ok(path),
        None => env::var_os("CALYX_HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CliError::usage("CALYX_HOME is required or pass --home <dir>")),
    }
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn bundle_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::from(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

#[cfg(test)]
mod tests;
