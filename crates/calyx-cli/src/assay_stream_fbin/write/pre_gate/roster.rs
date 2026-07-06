use std::fs;
use std::path::Path;

use calyx_registry::LensForgeManifest;

use crate::error::CliResult;

use super::super::super::args::Args;
use super::super::super::{io_error, local_error};

pub(super) fn streamed_lens_names(args: &Args) -> CliResult<Vec<String>> {
    if args.lens_template_cf_root.is_some() {
        if args.lens_template_specs.is_empty() {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_EMPTY",
                "DB-native lens template specs were not loaded before pre-encode validation",
                "run stream-fbin through the Calyx/Aster lens template path",
            ));
        }
        return Ok(args
            .lens_template_specs
            .iter()
            .map(|spec| spec.name.clone())
            .collect());
    }
    args.manifests
        .iter()
        .map(|path| read_manifest_name(path))
        .collect()
}

fn read_manifest_name(path: &Path) -> CliResult<String> {
    let manifest: LensForgeManifest = serde_json::from_slice(&fs::read(path).map_err(io_error)?)
        .map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
                format!("parse {} failed: {error}", path.display()),
                "fix the frozen lens manifests before streaming FBIN",
            )
        })?;
    if manifest.name.trim().is_empty() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_LOAD",
            format!("{} has an empty lens name", path.display()),
            "fix the frozen lens manifest before streaming FBIN",
        ));
    }
    Ok(manifest.name)
}
