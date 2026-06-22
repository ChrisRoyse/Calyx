use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::model::{
    BaseTemplateRef, BundleLensRef, EvidenceRef, RegistryRef, base_gate_passed, parse_modality,
    sha256_hex, template_id,
};
use super::{A38_BUNDLE_BASE_A37_REFUSED, A38_BUNDLE_INVALID, bundle_error};
use crate::error::{CliError, CliResult};

use crate::panel_commands::LensCatalogEntry;
use crate::panel_commands::template_store::TemplateStore;

pub(super) fn select_lenses(
    catalog: &[LensCatalogEntry],
    selectors: &[String],
) -> CliResult<Vec<BundleLensRef>> {
    if selectors.is_empty() {
        return Err(CliError::usage(
            "panel a38-bundle save requires at least one --include-lens <name-or-id>",
        ));
    }
    let mut selected = Vec::new();
    let mut seen_selectors = BTreeSet::new();
    let mut seen_ids = BTreeSet::new();
    for selector in selectors {
        if !seen_selectors.insert(selector.clone()) {
            return Err(bundle_error(
                A38_BUNDLE_INVALID,
                format!("lens selector {selector} was provided more than once"),
                "provide each included lens once",
            ));
        }
        let entry = catalog
            .iter()
            .find(|entry| entry.name == *selector || entry.lens_id == *selector)
            .ok_or_else(|| CliError::usage(format!("lens {selector} not found in catalog")))?;
        if !seen_ids.insert(entry.lens_id.clone()) {
            return Err(bundle_error(
                A38_BUNDLE_INVALID,
                format!("lens {} was selected more than once", entry.name),
                "provide each included lens once by name or id, not both",
            ));
        }
        parse_modality(&entry.modality)?;
        selected.push(BundleLensRef {
            lens_id: entry.lens_id.clone(),
            name: entry.name.clone(),
            modality: entry.modality.clone(),
            runtime: entry.runtime.clone(),
            weights_sha256: entry.weights_sha256.clone(),
            manifest: entry.manifest.display().to_string(),
            placement: entry.placement,
            cost: entry.cost,
        });
    }
    Ok(selected)
}

pub(super) fn required_modalities(values: &[String]) -> CliResult<Vec<String>> {
    if values.is_empty() {
        return Err(CliError::usage(
            "panel a38-bundle save requires at least one --required-modality <m>",
        ));
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for value in values {
        let modality = parse_modality(value)?;
        if !seen.insert(modality) {
            return Err(bundle_error(
                A38_BUNDLE_INVALID,
                format!("required modality {modality} was provided more than once"),
                "provide each required modality once",
            ));
        }
        out.push(modality.to_string());
    }
    Ok(out)
}

pub(super) fn evidence_refs(paths: &[PathBuf]) -> CliResult<Vec<EvidenceRef>> {
    if paths.is_empty() {
        return Err(CliError::usage(
            "panel a38-bundle save requires at least one --evidence <json>",
        ));
    }
    paths
        .iter()
        .map(|path| {
            let bytes = fs::read(path)?;
            if bytes.is_empty() {
                return Err(bundle_error(
                    A38_BUNDLE_INVALID,
                    format!("evidence artifact {} is empty", path.display()),
                    "point --evidence at a persisted non-empty FSV artifact",
                ));
            }
            Ok(EvidenceRef {
                path: path.display().to_string(),
                sha256_hex: sha256_hex(&bytes),
                size_bytes: bytes.len() as u64,
            })
        })
        .collect()
}

pub(super) fn registry_ref(path: &Path, lens_count: usize) -> CliResult<RegistryRef> {
    let bytes = fs::read(path)?;
    Ok(RegistryRef {
        path: path.display().to_string(),
        sha256_hex: sha256_hex(&bytes),
        size_bytes: bytes.len() as u64,
        lens_count,
    })
}

pub(super) fn base_template_ref(home: &Path, selector: &str) -> CliResult<BaseTemplateRef> {
    let template = TemplateStore::open(home).load(selector)?;
    let a37 = template.a37_admission();
    let base = BaseTemplateRef {
        name: template.name.clone(),
        version: template.version,
        template_id: template_id(&template)?,
        content_lens_count: template.content_lens_count(),
        a37_gate_eligible: a37.gate_eligible,
        a37_status: a37.status,
    };
    if base_gate_passed(&base) {
        return Ok(base);
    }
    Err(bundle_error(
        A38_BUNDLE_BASE_A37_REFUSED,
        format!(
            "base template {} A37 status is {}; gate_eligible={}",
            base.name, base.a37_status, base.a37_gate_eligible
        ),
        "profile the base template with an A37 gate_passed ensemble card first",
    ))
}
