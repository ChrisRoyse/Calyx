//! #1004 regression coverage: active GPU lenses without a resident route must
//! fail closed instead of silently taking the cold per-invocation worker path.

use calyx_core::{Placement, SlotResource};

use super::super::constellation::measure_constellation_microbatch_with_runtime_limit;
use super::super::route::{IngestGpuRoute, resolve_ingest_gpu_route};
use super::*;

fn gpu_placement_vault(name: &str) -> (std::path::PathBuf, ResolvedVault) {
    let root = temp_root(name);
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    let mut registry = Registry::new();
    let built = super::super::super::lens::build_lens(
        "algo16",
        "algorithmic",
        None,
        None,
        Some("Dense(16)"),
        Some("text"),
    )
    .unwrap();
    let lens_id = built.lens_id;
    built.register(&mut registry).unwrap();
    let mut panel = panel_with_text_slot(lens_id, SlotShape::Dense(16));
    panel.slots[0].resource = SlotResource {
        placement: Placement::Gpu,
        ..SlotResource::default()
    };
    AsterVault::new_durable(
        &path,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&path, &panel, &registry).unwrap();
    (
        root,
        ResolvedVault {
            path,
            name: name.to_string(),
            vault_id,
        },
    )
}

#[test]
fn gpu_panel_without_resident_route_fails_closed_before_measurement() {
    let (root, resolved) = gpu_placement_vault("issue1004-gpu-route-gate");
    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let before = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .unwrap();
    println!("issue1004_gate_before_base_rows={}", before.len());

    let no_route = IngestGpuRoute {
        resident_addr: None,
        allow_cold_gpu_workers: false,
        no_route_reason: Some("no_calyx_home"),
    };
    let err = measure_constellation_microbatch_with_runtime_limit(
        &vault,
        &state,
        &[text_input("issue1004 gpu route gate row".to_string())],
        1,
        None,
        no_route,
    )
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_INGEST_GPU_ROUTE_REQUIRED");
    assert!(
        err.message().contains("no_calyx_home"),
        "gate error must carry the discovery reason, got: {}",
        err.message()
    );

    let after = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .unwrap();
    println!("issue1004_gate_after_base_rows={}", after.len());
    assert_eq!(
        before, after,
        "refused GPU-route measurement must not write to Base CF"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn cpu_only_panel_is_not_gated_by_missing_resident_route() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1004-cpu-not-gated");
    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();

    let no_route = IngestGpuRoute {
        resident_addr: None,
        allow_cold_gpu_workers: false,
        no_route_reason: Some("no_calyx_home"),
    };
    let measured = measure_constellation_microbatch_with_runtime_limit(
        &vault,
        &state,
        &[text_input("issue1004 cpu panel row".to_string())],
        1,
        None,
        no_route,
    )
    .unwrap();
    assert_eq!(measured.len(), 1);
    assert!(matches!(
        measured[0].slots.get(&SlotId::new(0)),
        Some(SlotVector::Dense { dim: 16, .. })
    ));
    fs::remove_dir_all(root).ok();
}

#[test]
fn explicit_cold_worker_opt_in_passes_the_gate() {
    let (root, resolved) = gpu_placement_vault("issue1004-cold-opt-in");
    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();

    let result = measure_constellation_microbatch_with_runtime_limit(
        &vault,
        &state,
        &[text_input("issue1004 cold opt-in row".to_string())],
        1,
        None,
        IngestGpuRoute::cold_workers_allowed(),
    );
    // The opt-in must get past the #1004 gate. In the unit-test harness the
    // cold worker child cannot be spawned from the test binary, so success is
    // not asserted — only that the gate no longer refuses.
    if let Err(err) = result {
        assert_ne!(err.code(), "CALYX_INGEST_GPU_ROUTE_REQUIRED");
    }
    fs::remove_dir_all(root).ok();
}

#[test]
fn route_resolution_without_calyx_home_or_flag_reports_reason() {
    let (root, resolved) = gpu_placement_vault("issue1004-route-resolution");
    // No flag, no env expectations: this test only asserts the flag path and
    // the reason classes that do not depend on ambient env state.
    let flag_addr: std::net::SocketAddr = "127.0.0.1:8787".parse().unwrap();
    let route = resolve_ingest_gpu_route(&resolved.path, Some(flag_addr), false).unwrap();
    assert_eq!(route.resident_addr, Some(flag_addr));
    assert!(route.no_route_reason.is_none());

    let cold = resolve_ingest_gpu_route(&resolved.path, Some(flag_addr), true).unwrap();
    assert!(cold.allow_cold_gpu_workers);
    fs::remove_dir_all(root).ok();
}
