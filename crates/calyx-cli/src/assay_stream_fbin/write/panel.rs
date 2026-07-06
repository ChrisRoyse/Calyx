use calyx_registry::{LensSpec as RegistryLensSpec, lens_spec_from_manifest_path};

use crate::error::CliResult;

use super::super::args::Args;
use super::super::{MIN_A35_LENSES, local_error};
use super::bits::{diagnostic_bootstrap_bits, load_bits, streamable_for_mode};
use crate::a35_signal::{lens_spec_signal_kind_name, require_countable_content_signal_kind};

pub(super) fn validate_floor_before_runtime(args: &Args) -> CliResult {
    let bits = load_bits(args)?;
    let mut selected = 0usize;
    for spec in panel_specs(args)? {
        require_countable_content_signal_kind(
            &spec.name,
            lens_spec_signal_kind_name(&spec),
            "assay stream-fbin pre-runtime A35 gate",
        )?;
        let bootstrap_bits;
        let bits = if let Some(bits) = bits.get(&spec.name) {
            bits
        } else if args.diagnostic_bootstrap_without_admission() {
            bootstrap_bits = diagnostic_bootstrap_bits(&spec.name, args);
            &bootstrap_bits
        } else {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_MISSING",
                format!("lens {} missing from bits report", spec.name),
                "run bits-validate and pass a report containing every streamed lens",
            ));
        };
        if !streamable_for_mode(bits, args) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_REJECTED",
                format!(
                    "lens {} admitted={} bits_about={} min_bits={}",
                    spec.name, bits.admitted, bits.bits_about, args.min_bits
                ),
                "stream only admitted signal-bearing lenses in gate mode, or use diagnostic mode for measurement-only roster analysis",
            ));
        }
        selected += 1;
    }
    if selected < MIN_A35_LENSES {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PANEL_TOO_SMALL",
            format!(
                "selected {selected} streamable lenses; A35 requires at least {MIN_A35_LENSES}"
            ),
            "provide at least ten real frozen content lens manifests",
        ));
    }
    Ok(())
}

fn panel_specs(args: &Args) -> CliResult<Vec<RegistryLensSpec>> {
    if args.lens_template_cf_root.is_some() {
        if args.lens_template_specs.is_empty() {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_EMPTY",
                "DB-native lens template specs were not loaded before panel validation",
                "run stream-fbin through the Calyx/Aster lens template path",
            ));
        }
        return Ok(args.lens_template_specs.clone());
    }
    let mut specs = Vec::with_capacity(args.manifests.len());
    for manifest_path in &args.manifests {
        specs.push(
            lens_spec_from_manifest_path(manifest_path).map_err(|error| {
                local_error(
                    "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                    format!("{}: {}", manifest_path.display(), error.message),
                    "fix the frozen lens manifests before streaming FBIN",
                )
            })?,
        );
    }
    Ok(specs)
}
