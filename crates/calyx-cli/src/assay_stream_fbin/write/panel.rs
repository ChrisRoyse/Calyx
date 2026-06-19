use std::fs;

use calyx_registry::LensForgeManifest;

use crate::error::CliResult;

use super::super::args::Args;
use super::super::{MIN_A35_LENSES, io_error, local_error};
use super::bits::load_bits;

pub(super) fn validate_floor_before_runtime(args: &Args) -> CliResult {
    let bits = load_bits(args)?;
    let mut admitted = 0usize;
    for manifest_path in &args.manifests {
        let manifest = read_manifest(manifest_path)?;
        let Some(bits) = bits.get(&manifest.name) else {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_MISSING",
                format!("lens {} missing from bits report", manifest.name),
                "run bits-validate and pass a report containing every streamed lens",
            ));
        };
        if !bits.admitted || !bits.bits_about.is_finite() || bits.bits_about < args.min_bits {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_REJECTED",
                format!(
                    "lens {} admitted={} bits_about={} min_bits={}",
                    manifest.name, bits.admitted, bits.bits_about, args.min_bits
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

fn read_manifest(path: &std::path::Path) -> CliResult<LensForgeManifest> {
    serde_json::from_slice(&fs::read(path).map_err(io_error)?).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
            format!("parse {} failed: {error}", path.display()),
            "fix the frozen lens manifests before streaming FBIN",
        )
    })
}
