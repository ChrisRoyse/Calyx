use std::collections::BTreeMap;
use std::path::PathBuf;

use calyx_registry::lens_spec_from_manifest_path;

use crate::a35_signal::{require_countable_content_signal_kind, runtime_signal_kind_name};
use crate::assay_corpus_build::lens::{BuildLens, load_lenses};
use crate::error::CliResult;

use super::super::args::Args;
use super::super::{MIN_A35_LENSES, local_error};
use super::bits::{BitsLens, load_bits};

pub(super) struct SelectedLens {
    pub(super) manifest: PathBuf,
    pub(super) bits: BitsLens,
}

impl SelectedLens {
    pub(super) fn load_runtime(&self, args: &Args) -> CliResult<BuildLens> {
        let mut request = args.corpus_request();
        request.manifests = vec![self.manifest.clone()];
        let mut lenses = load_lenses(&request).map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                error,
                "fix the frozen lens manifest before streaming this slot",
            )
        })?;
        if lenses.len() != 1 {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                format!("loaded {} runtimes for one selected manifest", lenses.len()),
                "fix stream-fbin single-slot runtime loading",
            ));
        }
        Ok(lenses.remove(0))
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
    let mut selected = Vec::with_capacity(args.manifests.len());
    for manifest in &args.manifests {
        let spec = lens_spec_from_manifest_path(manifest).map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                format!("{}: {}", manifest.display(), error.message),
                "fix the frozen lens manifest before streaming FBIN",
            )
        })?;
        if let Some(previous) = names.insert(spec.name.clone(), manifest.clone()) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                format!(
                    "duplicate lens {} manifests={} and {}",
                    spec.name,
                    previous.display(),
                    manifest.display()
                ),
                "deduplicate the stream-fbin manifest roster",
            ));
        }
        let Some(bits) = bits.get(&spec.name).cloned() else {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_MISSING",
                format!("lens {} missing from bits report", spec.name),
                "run bits-validate and pass a report containing every streamed lens",
            ));
        };
        if !bits.admitted || !bits.bits_about.is_finite() || bits.bits_about < args.min_bits {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_REJECTED",
                format!(
                    "lens {} admitted={} bits_about={} min_bits={}",
                    spec.name, bits.admitted, bits.bits_about, args.min_bits
                ),
                "stream only admitted signal-bearing lenses or lower --min-bits deliberately",
            ));
        }
        require_countable_content_signal_kind(
            &spec.name,
            runtime_signal_kind_name(&spec.runtime),
            "assay stream-fbin runtime A35 gate",
        )?;
        selected.push(SelectedLens {
            manifest: manifest.clone(),
            bits,
        });
    }
    if selected.len() < min_lenses {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PANEL_TOO_SMALL",
            format!(
                "selected {} admitted lenses; requires at least {min_lenses}",
                selected.len(),
            ),
            "provide at least ten real frozen content lens manifests",
        ));
    }
    Ok(selected)
}
