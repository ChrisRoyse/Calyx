use std::path::Path;
use std::sync::Arc;

use calyx_anneal::{
    ArtifactPtr, BaseShard, BudgetHandle, CALYX_ANNEAL_REBUILD_TRIPWIRE_FAILED, ChecksumDetector,
    ChecksumEntry, ComponentHealth, ComponentKind, EndpointUrl, FaultDetector, FaultMonitor,
    HttpProbe, LensProbeDetector, ProbeStatus, RebuildOutcome, RebuildPriority, RebuildScheduler,
    RebuildTarget, ShardId, base_shard_checksum, fail_reads_on_range, record_base_shard_checksum,
    verify_base_shards,
};
use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::mvcc::CALYX_ASTER_BASE_CORRUPT;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, CxId, FixedClock, Result, SlotId};
use serde_json::{Value, json};

use super::helpers::{
    FsvRegistryInner, FsvSubstrate, cx, cx_range, health_rows, ledger_rows, lens, sha256_file,
    substrate,
};

pub(crate) fn run_corrupt_ann_fault(
    clock: &FixedClock,
    vault: &AsterVault,
    registry: &mut FsvRegistryInner<'_>,
    substrate: &mut FsvSubstrate<'_>,
    ann_path: &Path,
    expected_sha: [u8; 32],
) -> Value {
    let before = health_rows(vault);
    let detector = ChecksumDetector::new(
        vec![(
            ComponentKind::ann_index(SlotId::new(0)),
            ChecksumEntry::new(ann_path, expected_sha),
        )],
        Arc::new(*clock),
    );
    let mut monitor = FaultMonitor::new(
        vec![Box::new(detector) as Box<dyn FaultDetector<_>>],
        BudgetHandle::new(8),
        100,
    );
    let events = monitor.run_once(registry, &mut substrate.ledger).unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        registry.health(&ComponentKind::ann_index(SlotId::new(0))),
        ComponentHealth::Degraded { .. }
    ));
    json!({"before_health": before, "events": events, "after_health": health_rows(vault)})
}

pub(crate) fn run_failing_lens_route(
    clock: &FixedClock,
    vault: &AsterVault,
    registry: &mut FsvRegistryInner<'_>,
    substrate: &mut FsvSubstrate<'_>,
) -> Value {
    let l1 = lens(0xa1);
    let l2 = lens(0xb2);
    let detector = LensProbeDetector::new(
        vec![
            (l1, EndpointUrl::new("mock://lens-l1")),
            (l2, EndpointUrl::new("mock://lens-l2")),
        ],
        Arc::new(ScriptedProbe {
            failing_endpoint: "mock://lens-l1",
        }),
        Arc::new(*clock),
    );
    let mut monitor = FaultMonitor::new(
        vec![Box::new(detector) as Box<dyn FaultDetector<_>>],
        BudgetHandle::new(8),
        100,
    );
    let events = monitor.run_once(registry, &mut substrate.ledger).unwrap();
    assert_eq!(events.len(), 1);
    let route = registry.route_lens_panel(&[l1, l2]);
    assert_eq!(route.active, vec![l2]);
    assert!(route.degraded);
    let results = route
        .active
        .iter()
        .map(|lens_id| json!({"lens_id": lens_id.to_string(), "cx_id": cx(0x22).to_string()}))
        .collect::<Vec<_>>();
    json!({
        "events": events,
        "route": route,
        "search_response": {"degraded": true, "timed_out": false, "results": results},
        "health_rows": health_rows(vault),
    })
}

pub(crate) fn run_empty_scheduler_edge(
    clock: &FixedClock,
    vault: &AsterVault,
    ann_dir: &Path,
    registry: &mut FsvRegistryInner<'_>,
    substrate: &mut FsvSubstrate<'_>,
) -> Value {
    let before_ledger_count = ledger_rows(vault).len();
    let mut scheduler = RebuildScheduler::new(clock, vault, ann_dir);
    let first = scheduler.run_next(registry, substrate).unwrap();
    let second = scheduler.run_next(registry, substrate).unwrap();
    assert_eq!(first, RebuildOutcome::NothingQueued);
    assert_eq!(second, RebuildOutcome::NothingQueued);
    let after_ledger_count = ledger_rows(vault).len();
    assert_eq!(before_ledger_count, after_ledger_count);
    json!({ "first": first, "second": second, "before_ledger_count": before_ledger_count, "after_ledger_count": after_ledger_count })
}

pub(crate) fn run_tripwire_failure_edge(
    clock: &FixedClock,
    vault: &AsterVault,
    vault_dir: &Path,
    ann_dir: &Path,
    registry: &mut FsvRegistryInner<'_>,
    prior_path: &Path,
) -> Value {
    let target = RebuildTarget::AnnIndex {
        slot_id: SlotId::new(1),
    };
    let mut substrate = substrate(clock, vault, vault_dir, 0.99);
    registry
        .set_health(
            target.component(),
            ComponentHealth::degraded(clock.now(), "synthetic tripwire edge"),
            &mut substrate.ledger,
        )
        .unwrap();
    let prior = ArtifactPtr::HnswGraphPath(prior_path.to_string_lossy().into_owned());
    substrate
        .rollback
        .install_live_ptr(target.artifact_key(), prior.clone())
        .unwrap();
    let before_sha = sha256_file(prior_path);
    let mut scheduler = RebuildScheduler::new(clock, vault, ann_dir);
    scheduler.enqueue(target.clone(), RebuildPriority::HIGH);
    let outcome = scheduler.run_next(registry, &mut substrate).unwrap();
    assert!(matches!(
        outcome,
        RebuildOutcome::Failed {
            ref reason_code,
            ..
        } if reason_code == CALYX_ANNEAL_REBUILD_TRIPWIRE_FAILED
    ));
    assert!(matches!(
        registry.health(&target.component()),
        ComponentHealth::Degraded { .. }
    ));
    assert_eq!(
        substrate.rollback.live_ptr(&target.artifact_key()).unwrap(),
        Some(prior)
    );
    assert_eq!(before_sha, sha256_file(prior_path));
    json!({ "target": target, "outcome": outcome, "prior_artifact_sha256_before": super::helpers::hex(&before_sha), "prior_artifact_sha256_after": super::helpers::hex(&sha256_file(prior_path)), "ledger_rows_after": ledger_rows(vault) })
}

pub(crate) fn run_base_corruption_edge(clock: &FixedClock, vault: &AsterVault) -> Value {
    let id = cx(0xee);
    let range = cx_range(id);
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(id),
            b"issue405-base-edge-good".to_vec(),
        )
        .unwrap();
    let expected = base_shard_checksum(vault, &range).unwrap();
    let shard = BaseShard::new(ShardId::new("issue405_base_edge"), range, expected);
    record_base_shard_checksum(vault, &shard, clock).unwrap();
    vault
        .write_cf(
            ColumnFamily::Base,
            base_key(id),
            b"issue405-base-edge-corrupt".to_vec(),
        )
        .unwrap();
    let corrupt_before = base_shard_checksum(vault, &shard.cf_range).unwrap();
    let events = verify_base_shards(vault, clock).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actual(), corrupt_before);
    let events_json = events
        .iter()
        .map(|event| {
            json!({
                "shard_id": event.shard().shard_id.as_str(),
                "range_start_hex": super::helpers::hex(&event.shard().cf_range.start),
                "range_end_hex": event.shard().cf_range.end.as_ref().map(|end| super::helpers::hex(end)),
                "expected_sha256": super::helpers::hex(&event.expected()),
                "actual_sha256": super::helpers::hex(&event.actual()),
                "detected_at": event.detected_at(),
            })
        })
        .collect::<Vec<_>>();
    fail_reads_on_range(vault, &events[0]).unwrap();
    let err = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(id))
        .expect_err("corrupt base read must fail closed");
    assert_eq!(err.code, CALYX_ASTER_BASE_CORRUPT);
    let checksum_rows_after = super::helpers::raw_cf_rows(vault, ColumnFamily::AnnealChecksums);
    json!({
        "events": events_json,
        "error_code": err.code,
        "range_sha256_before_barrier": super::helpers::hex(&corrupt_before),
        "range_sha256_after_barrier_recorded": super::helpers::hex(&events[0].actual()),
        "checksum_rows_after_barrier": checksum_rows_after,
        "auto_rebuild_attempted": false
    })
}

struct ScriptedProbe {
    failing_endpoint: &'static str,
}

impl HttpProbe for ScriptedProbe {
    fn probe(&self, endpoint: &EndpointUrl) -> Result<ProbeStatus> {
        if endpoint.as_str() == self.failing_endpoint {
            return Err(CalyxError {
                code: "CALYX_TEST_LENS_TIMEOUT",
                message: format!("synthetic timeout probing {}", endpoint.as_str()),
                remediation: "route to remaining healthy lenses",
            });
        }
        Ok(ProbeStatus { ok: true })
    }
}

#[allow(dead_code)]
fn _assert_send_sync<T: Send + Sync>() {}

#[allow(dead_code)]
fn _cx_type(_: CxId) {}
