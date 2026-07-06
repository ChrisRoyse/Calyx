use std::collections::BTreeMap;
use std::path::PathBuf;

use calyx_registry::{LensSpec as RegistryLensSpec, lens_spec_from_manifest_path};

use crate::a35_signal::{lens_spec_signal_kind_name, require_countable_content_signal_kind};
use crate::assay_corpus_build::lens::{BuildLens, build_lens_from_spec};
use crate::error::CliResult;

use super::super::args::Args;
use super::super::{MIN_A35_LENSES, local_error};
use super::bits::{BitsLens, diagnostic_bootstrap_bits, load_bits, streamable_for_mode};

pub(super) struct SelectedLens {
    pub(super) manifest: Option<PathBuf>,
    pub(super) descriptor_ref: String,
    pub(super) spec: RegistryLensSpec,
    pub(super) bits: BitsLens,
}

impl SelectedLens {
    pub(super) fn load_runtime(&self) -> CliResult<BuildLens> {
        let manifest_ref = self
            .manifest
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("db-lens-{}", self.spec.name)));
        build_lens_from_spec(manifest_ref, self.spec.clone()).map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                format!("{}: {error}", self.descriptor_ref),
                "fix the DB-native lens descriptor before streaming this slot",
            )
        })
    }
}

pub(super) fn selected_lenses(args: &Args) -> CliResult<Vec<SelectedLens>> {
    selected_lenses_with_min(args, MIN_A35_LENSES)
}

pub(super) fn selected_lenses_for_worker(args: &Args) -> CliResult<Vec<SelectedLens>> {
    selected_lenses_with_min(args, 1)
}

fn selected_lenses_with_min(args: &Args, min_lenses: usize) -> CliResult<Vec<SelectedLens>> {
    let bits = load_bits(args)?;
    let mut names = BTreeMap::new();
    let mut selected = Vec::with_capacity(args.manifests.len().max(args.lens_template_specs.len()));
    for (manifest, descriptor_ref, spec) in selected_specs(args)? {
        if let Some(previous) = names.insert(spec.name.clone(), descriptor_ref.clone()) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                format!(
                    "duplicate lens {} descriptors={} and {}",
                    spec.name, previous, descriptor_ref
                ),
                "deduplicate the stream-fbin manifest roster",
            ));
        }
        let bits = if let Some(bits) = bits.get(&spec.name).cloned() {
            bits
        } else if args.diagnostic_bootstrap_without_admission() {
            diagnostic_bootstrap_bits(&spec.name, args)
        } else {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_MISSING",
                format!("lens {} missing from bits report", spec.name),
                "run bits-validate and pass a report containing every streamed lens",
            ));
        };
        if !streamable_for_mode(&bits, args) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_REJECTED",
                format!(
                    "lens {} admitted={} bits_about={} min_bits={}",
                    spec.name, bits.admitted, bits.bits_about, args.min_bits
                ),
                "stream only admitted signal-bearing lenses in gate mode, or use diagnostic mode for measurement-only roster analysis",
            ));
        }
        require_countable_content_signal_kind(
            &spec.name,
            lens_spec_signal_kind_name(&spec),
            "assay stream-fbin runtime A35 gate",
        )?;
        selected.push(SelectedLens {
            manifest,
            descriptor_ref,
            spec,
            bits,
        });
    }
    if selected.len() < min_lenses {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PANEL_TOO_SMALL",
            format!(
                "selected {} streamable lenses; requires at least {min_lenses}",
                selected.len(),
            ),
            "provide at least ten real frozen content lens manifests",
        ));
    }
    Ok(selected)
}

fn selected_specs(args: &Args) -> CliResult<Vec<(Option<PathBuf>, String, RegistryLensSpec)>> {
    if args.lens_template_cf_root.is_some() {
        if args.lens_template_specs.is_empty() {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_EMPTY",
                "DB-native lens template specs were not loaded before selection",
                "run stream-fbin through the Calyx/Aster lens template path",
            ));
        }
        return Ok(args
            .lens_template_specs
            .iter()
            .map(|spec| (None, args.lens_descriptor_ref(&spec.name), spec.clone()))
            .collect());
    }
    let mut selected = Vec::with_capacity(args.manifests.len());
    for manifest in &args.manifests {
        let spec = lens_spec_from_manifest_path(manifest).map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                format!("{}: {}", manifest.display(), error.message),
                "fix the frozen lens manifest before streaming FBIN",
            )
        })?;
        selected.push((Some(manifest.clone()), manifest.display().to_string(), spec));
    }
    Ok(selected)
}
