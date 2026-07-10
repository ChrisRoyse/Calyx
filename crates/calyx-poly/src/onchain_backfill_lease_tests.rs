//! Unit + full-state tests for the on-chain backfill lease (issue #217).

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use super::*;
use crate::StructuredLogSink;
use calyx_core::{FixedClock, SystemClock};

fn unique_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "calyx-poly-lease-{name}-{}-{}",
        std::process::id(),
        now_unix_ms(&SystemClock)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn lease_with(pid: u32, started_at: u128, heartbeat_at: u128, deadline: u128) -> BackfillLease {
    BackfillLease {
        schema_version: ONCHAIN_BACKFILL_LEASE_SCHEMA_VERSION.to_string(),
        pid,
        hostname: "test-host".to_string(),
        command: "calyx-poly-onchain-backfill --readback-only".to_string(),
        started_at_unix_ms: started_at,
        heartbeat_at_unix_ms: heartbeat_at,
        heartbeat_interval_secs: DEFAULT_HEARTBEAT_INTERVAL_SECS,
        max_runtime_secs: DEFAULT_MAX_RUNTIME_SECS,
        deadline_unix_ms: deadline,
    }
}

#[test]
fn acquire_decision_proceeds_when_no_existing_lease() {
    assert_eq!(
        acquire_decision(None, 1_000, false),
        AcquireDecision::Proceed
    );
}

#[test]
fn acquire_decision_reaps_stale_heartbeat() {
    // interval 10s * 6 missed = 60s window; heartbeat 61s ago => stale.
    let now = 1_000_000u128;
    let lease = lease_with(4242, now - 120_000, now - 61_000, now + 600_000);
    match acquire_decision(Some(&lease), now, false) {
        AcquireDecision::Reap(reason) => assert!(reason.contains("heartbeat stale"), "{reason}"),
        other => panic!("expected Reap(stale heartbeat), got {other:?}"),
    }
}

#[test]
fn acquire_decision_reaps_past_deadline_even_with_fresh_heartbeat() {
    let now = 3_000_000u128;
    // Heartbeat is fresh (1s ago) but the deadline already passed.
    let lease = lease_with(4242, now - 2_000_000, now - 1_000, now - 5_000);
    match acquire_decision(Some(&lease), now, false) {
        AcquireDecision::Reap(reason) => assert!(reason.contains("deadline"), "{reason}"),
        other => panic!("expected Reap(deadline), got {other:?}"),
    }
}

#[test]
fn acquire_decision_blocks_live_lease_without_takeover() {
    let now = 1_000_000u128;
    // Heartbeat 5s ago (< 60s window), deadline in the future => live peer.
    let lease = lease_with(4242, now - 30_000, now - 5_000, now + 600_000);
    assert_eq!(
        acquire_decision(Some(&lease), now, false),
        AcquireDecision::Blocked {
            pid: 4242,
            age_ms: 5_000
        }
    );
}

#[test]
fn acquire_decision_reaps_live_lease_with_takeover() {
    let now = 1_000_000u128;
    let lease = lease_with(4242, now - 30_000, now - 5_000, now + 600_000);
    match acquire_decision(Some(&lease), now, true) {
        AcquireDecision::Reap(reason) => assert!(reason.contains("--takeover"), "{reason}"),
        other => panic!("expected Reap(takeover), got {other:?}"),
    }
}

#[test]
fn lease_round_trips_through_disk() {
    let dir = unique_dir("roundtrip");
    let path = lease_path_for_output_root(&dir);
    let lease = lease_with(7, 100, 100, 100 + 1000);
    write_lease_atomic(&path, &lease).unwrap();
    let read = read_lease(&path).unwrap().expect("lease present");
    assert_eq!(read, lease);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn read_lease_returns_none_when_absent() {
    let dir = unique_dir("absent");
    let path = lease_path_for_output_root(&dir);
    assert!(read_lease(&path).unwrap().is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn acquire_writes_pid_and_started_at_then_releases_on_drop() {
    let dir = unique_dir("acquire");
    let path = lease_path_for_output_root(&dir);
    let sink = StructuredLogSink::new(dir.join("lease.log.jsonl")).unwrap();
    let clock = FixedClock::new(1_785_600_217_000);
    {
        let guard = acquire(
            &path,
            "test command".to_string(),
            3_600, // large max-runtime so the watchdog never fires here
            DEFAULT_HEARTBEAT_INTERVAL_SECS,
            false,
            &sink,
            clock,
        )
        .unwrap();
        assert_eq!(guard.lease_path(), path.as_path());
        // Full-state verification: the lease physically exists on disk and
        // carries THIS process's pid + a started_at timestamp.
        let lease = read_lease(&path).unwrap().expect("lease written to disk");
        assert_eq!(lease.pid, std::process::id());
        assert_eq!(lease.started_at_unix_ms, 1_785_600_217_000);
        assert_eq!(lease.max_runtime_secs, 3_600);
        assert_eq!(lease.deadline_unix_ms, lease.started_at_unix_ms + 3_600_000);
    }
    // Guard dropped => lease released.
    assert!(
        read_lease(&path).unwrap().is_none(),
        "lease must be removed when the guard drops"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn acquire_refuses_when_a_live_lease_is_present() {
    let dir = unique_dir("refuse");
    let path = lease_path_for_output_root(&dir);
    let sink = StructuredLogSink::new(dir.join("lease.log.jsonl")).unwrap();
    // Plant a fresh, live lease held by a different pid.
    let now = now_unix_ms(&SystemClock);
    let live = lease_with(999_999, now - 1_000, now - 1_000, now + 600_000);
    write_lease_atomic(&path, &live).unwrap();

    let error = acquire(
        &path,
        "second instance".to_string(),
        3_600,
        DEFAULT_HEARTBEAT_INTERVAL_SECS,
        false,
        &sink,
        SystemClock,
    )
    .expect_err("must refuse to trample a live lease");
    assert_eq!(error.diagnostic().code, CODE_LEASE_HELD);
    // The live lease is untouched.
    let still = read_lease(&path).unwrap().expect("live lease preserved");
    assert_eq!(still.pid, 999_999);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn acquire_reaps_a_stale_lease_and_logs_the_reap() {
    let dir = unique_dir("reap");
    let path = lease_path_for_output_root(&dir);
    let log_path = dir.join("lease.log.jsonl");
    let sink = StructuredLogSink::new(&log_path).unwrap();
    // Plant a stale lease: heartbeat far in the past.
    let now = now_unix_ms(&SystemClock);
    let stale = lease_with(888_888, now - 3_600_000, now - 3_600_000, now + 600_000);
    write_lease_atomic(&path, &stale).unwrap();

    let guard = acquire(
        &path,
        "reaping instance".to_string(),
        3_600,
        DEFAULT_HEARTBEAT_INTERVAL_SECS,
        false,
        &sink,
        SystemClock,
    )
    .expect("stale lease must be reaped, not blocking");
    // Fresh lease now belongs to us.
    let fresh = read_lease(&path).unwrap().expect("fresh lease");
    assert_eq!(fresh.pid, std::process::id());
    drop(guard);

    // The reap was logged with the stale pid (fail-loud provenance).
    let log = fs::read_to_string(&log_path).unwrap();
    assert!(log.contains(CODE_LEASE_REAPED), "reap not logged: {log}");
    assert!(log.contains("888888"), "stale pid not logged: {log}");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn heartbeat_advances_the_lease_timestamp_on_disk() {
    let dir = unique_dir("heartbeat");
    let path = lease_path_for_output_root(&dir);
    let sink = StructuredLogSink::new(dir.join("lease.log.jsonl")).unwrap();
    let guard = acquire(
        &path,
        "heartbeat instance".to_string(),
        3_600,
        1, // 1s heartbeat cadence
        false,
        &sink,
        SystemClock,
    )
    .unwrap();
    let first = read_lease(&path).unwrap().unwrap().heartbeat_at_unix_ms;
    thread::sleep(Duration::from_millis(1_500));
    let second = read_lease(&path).unwrap().unwrap().heartbeat_at_unix_ms;
    assert!(
        second > first,
        "heartbeat did not advance: {first} -> {second}"
    );
    drop(guard);
    fs::remove_dir_all(&dir).ok();
}
