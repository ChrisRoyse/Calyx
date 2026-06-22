#[path = "fsv_corrupt_rebuild_edges.rs"]
mod edges;
#[path = "fsv_corrupt_rebuild_helpers.rs"]
mod helpers;

use std::fs;

use calyx_anneal::{
    ArtifactPtr, ComponentHealth, RebuildOutcome, RebuildPriority, RebuildScheduler, RebuildTarget,
};
use calyx_core::SlotId;
use serde_json::json;

use crate::fsv_support::vault_id;
use edges::{
    run_base_corruption_edge, run_corrupt_ann_fault, run_empty_scheduler_edge,
    run_failing_lens_route, run_tripwire_failure_edge,
};
use helpers::{
    FsvRegistry, artifact_readback, brute_force_recall_readback, cf_sha256, fsv_paths,
    ledger_has_lens_degrade_change, ledger_has_rebuild_with_hashes,
    ledger_has_tripwire_revert_metrics, ledger_rows, list_files, reset_dir, sha256_file, substrate,
    write_ann_file, write_source_rows,
};

pub fn run_issue405_fsv() {
    let paths = fsv_paths();
    let _root = reset_dir(&paths.root);
    let vault_dir = reset_dir(&paths.vault);
    let ann_dir = reset_dir(&paths.ann);
    let ann_path = ann_dir.join("slot_0.hnsw");
    let fail_prior_path = ann_dir.join("slot_1-prior.hnsw");

    let vault = calyx_aster::vault::AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue405-corrupt-rebuild-fsv-salt".to_vec(),
        calyx_aster::vault::VaultOptions::default(),
    )
    .unwrap();
    write_source_rows(&vault);
    vault.flush().unwrap();

    let clock = calyx_core::FixedClock::new(helpers::TEST_TS);
    let mut registry = FsvRegistry::open(&clock, &vault);
    let mut substrate = substrate(&clock, &vault, &vault_dir, 0.90);
    let target = RebuildTarget::AnnIndex {
        slot_id: SlotId::new(0),
    };

    let base_before = cf_sha256(&vault, calyx_aster::cf::ColumnFamily::Base);
    write_ann_file(&ann_path, b"issue405-slot0-ann-index-original");
    let ann_before = sha256_file(&ann_path);
    helpers::flip_byte(&ann_path, 42);
    let ann_corrupt = sha256_file(&ann_path);
    assert_ne!(ann_before, ann_corrupt);

    let fault = run_corrupt_ann_fault(
        &clock,
        &vault,
        &mut registry.0,
        &mut substrate,
        &ann_path,
        ann_before,
    );
    substrate
        .rollback
        .install_live_ptr(
            target.artifact_key(),
            ArtifactPtr::HnswGraphPath(ann_path.to_string_lossy().into_owned()),
        )
        .unwrap();
    let mut scheduler = RebuildScheduler::new(&clock, &vault, &ann_dir);
    scheduler.enqueue(target.clone(), RebuildPriority::HIGH);
    let outcome = scheduler.run_next(&mut registry.0, &mut substrate).unwrap();
    let RebuildOutcome::Completed {
        prior_ptr, new_ptr, ..
    } = &outcome
    else {
        panic!("expected completed rebuild");
    };
    assert_eq!(
        prior_ptr,
        &ArtifactPtr::HnswGraphPath(ann_path.to_string_lossy().into_owned())
    );
    assert_ne!(new_ptr, prior_ptr);
    assert_eq!(registry.0.health(&target.component()), &ComponentHealth::Ok);
    vault.flush().unwrap();

    let base_after = cf_sha256(&vault, calyx_aster::cf::ColumnFamily::Base);
    assert_eq!(base_before, base_after);
    let brute_force = brute_force_recall_readback();
    assert_eq!(brute_force["recall_at_1"], json!(1.0));

    let empty_edge =
        run_empty_scheduler_edge(&clock, &vault, &ann_dir, &mut registry.0, &mut substrate);
    let lens_route = run_failing_lens_route(&clock, &vault, &mut registry.0, &mut substrate);
    write_ann_file(&fail_prior_path, b"issue405-slot1-ann-prior");
    let tripwire_fail = run_tripwire_failure_edge(
        &clock,
        &vault,
        &vault_dir,
        &ann_dir,
        &mut registry.0,
        &fail_prior_path,
    );
    let base_corrupt = run_base_corruption_edge(&clock, &vault);
    vault.flush().unwrap();

    let ledger_rows = ledger_rows(&vault);
    assert!(ledger_has_rebuild_with_hashes(&ledger_rows));
    assert!(ledger_has_lens_degrade_change(&ledger_rows));
    assert!(ledger_has_tripwire_revert_metrics(&ledger_rows));

    let readback = json!({
        "source_of_truth": "Aster base/slot/anneal_health/anneal_rollback/ledger CF bytes, WAL files, and vault/ann artifacts",
        "vault": vault_dir,
        "ann_path": ann_path,
        "base_cf_sha256_before_ann_rebuild": helpers::hex(&base_before),
        "base_cf_sha256_after_ann_rebuild": helpers::hex(&base_after),
        "ann_sha256_before": helpers::hex(&ann_before),
        "ann_sha256_after_byte42_flip": helpers::hex(&ann_corrupt),
        "corrupt_ann_fault": fault,
        "rebuild_outcome": outcome,
        "after_rebuild_slot0_health": registry.0.health(&target.component()),
        "rebuilt_artifact": artifact_readback(new_ptr),
        "brute_force_recall_baseline": brute_force,
        "empty_scheduler_edge": empty_edge,
        "failing_lens_route": lens_route,
        "tripwire_failure_edge": tripwire_fail,
        "base_corruption_edge": base_corrupt,
        "health_rows": helpers::health_rows(&vault),
        "rollback_rows": helpers::raw_cf_rows(&vault, calyx_aster::cf::ColumnFamily::AnnealRollback),
        "ledger_rows": ledger_rows,
        "wal_files": list_files(&paths.wal),
        "cf_files": list_files(&paths.cf),
    });
    fs::write(
        &paths.readback,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    println!("ISSUE405_FSV_READBACK {}", paths.readback.display());
}
