use calyx_registry::lens_spec_from_manifest_path;

use crate::error::CliResult;

use super::super::args::Args;
use super::super::{MIN_A35_LENSES, local_error};
use super::bits::load_bits;
use crate::a35_signal::{require_countable_content_signal_kind, runtime_signal_kind_name};

pub(super) fn validate_floor_before_runtime(args: &Args) -> CliResult {
    let bits = load_bits(args)?;
    let mut admitted = 0usize;
    for manifest_path in &args.manifests {
        let spec = lens_spec_from_manifest_path(manifest_path).map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                format!("{}: {}", manifest_path.display(), error.message),
                "fix the frozen lens manifests before streaming FBIN",
            )
        })?;
        require_countable_content_signal_kind(
            &spec.name,
            runtime_signal_kind_name(&spec.runtime),
            "assay stream-fbin pre-runtime A35 gate",
        )?;
        let Some(bits) = bits.get(&spec.name) else {
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
        admitted += 1;
    }
    if admitted < MIN_A35_LENSES {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PANEL_TOO_SMALL",
            format!("selected {admitted} admitted lenses; A35 requires at least {MIN_A35_LENSES}"),
            "provide at least ten real frozen content lens manifests",
        ));
    }
    Ok(())
}
