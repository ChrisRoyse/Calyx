use std::path::Path;

use calyx_sextant::index::partitioned_manifest_db_readback;

use crate::error::{CliError, CliResult};

pub(crate) fn run(args: &[String]) -> CliResult {
    let [vault_flag, vault] = args else {
        return Err(CliError::usage(
            "usage: calyx readback partitioned-manifest --vault <dir>",
        ));
    };
    if vault_flag != "--vault" {
        return Err(CliError::usage(
            "usage: calyx readback partitioned-manifest --vault <dir>",
        ));
    }
    let readback = partitioned_manifest_db_readback(Path::new(vault)).map_err(CliError::Calyx)?;
    let manifest = &readback.manifest;
    let non_empty_regions = manifest.regions.len();
    let max_region_count = manifest
        .regions
        .iter()
        .map(|region| region.count)
        .max()
        .unwrap_or(0);
    let min_region_count = manifest
        .regions
        .iter()
        .map(|region| region.count)
        .min()
        .unwrap_or(0);
    let replication_factor = manifest
        .final_assignment_closure
        .as_ref()
        .map(|closure| closure.replication_factor())
        .unwrap_or(1.0);
    println!(
        "partitioned_manifest_readback source=calyx_aster_graph_cf vault={} format={} n_cx={} dim={} n_regions={} non_empty_regions={} stored_region_members={} min_region_count={} max_region_count={} m_max={} ef_construction={} distance_metric={} graph_build_backend={} final_assignment_probe={} final_assignment_cap={} final_assignment_max_replication={} replication_factor={:.6} value_bytes={} value_blake3={}",
        vault,
        manifest.format,
        manifest.n_cx,
        manifest.dim,
        manifest.n_regions,
        non_empty_regions,
        manifest.stored_region_members,
        min_region_count,
        max_region_count,
        manifest.m_max,
        manifest.ef_construction,
        manifest.distance_metric.as_str(),
        manifest.graph_build_backend.as_str(),
        manifest.final_assignment_probe,
        manifest.final_assignment_cap.unwrap_or(0),
        manifest.final_assignment_max_replication,
        replication_factor,
        readback.value_bytes,
        readback.value_blake3,
    );
    Ok(())
}
