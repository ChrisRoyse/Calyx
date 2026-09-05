//! Unit coverage for the three resident-lease refusal codes, issue #1936.
//!
//! These drive [`published_resident_lease`] and [`match_live_lease_row`]
//! directly with a synthetic readiness value and reservation snapshot, so each
//! fail-closed branch is exercised without a GPU and without mutating live
//! reservation authority state.

use calyx_forge::vram::HostGpuReservationView;

use super::*;

const OP: &str = "ingest --batch";

fn addr() -> SocketAddr {
    "127.0.0.1:18401".parse().expect("valid test addr")
}

fn ready(reservation_id: Value, lease_pid: Value) -> Value {
    serde_json::json!({
        "ready": true,
        "bind": "127.0.0.1:18401",
        "reservation_id": reservation_id,
        "lease_pid": lease_pid,
    })
}

fn row(reservation_id: &str, pid: u32) -> HostGpuReservationView {
    HostGpuReservationView {
        reservation_id: reservation_id.to_string(),
        owner: "gpuhost-operator".to_string(),
        job_id: "legal-resident".to_string(),
        command: "calyx gpu run child=calyx panel resident serve".to_string(),
        pid,
        requested_mib: 9800,
        acquired_unix_ms: 1_784_545_563_882,
        lease_file: format!("/tmp/calyx-gpu-reservations/device-0/lease-{reservation_id}.lock"),
    }
}

fn snapshot(reservations: Vec<HostGpuReservationView>) -> HostGpuReservationSnapshot {
    let reserved_mib = reservations.iter().map(|row| row.requested_mib).sum();
    HostGpuReservationSnapshot {
        schema_version: 1,
        state_path: "/tmp/calyx-gpu-reservations/device-0/reservations.json".to_string(),
        state_sha256: "0".repeat(64),
        device_index: 0,
        device_uuid: "GPU-test".to_string(),
        device_name: "test-device".to_string(),
        device_total_mib: 32_607,
        host_cap_mib: 22_528,
        required_free_mib: 1_024,
        headroom_mib: 1_024,
        epoch_free_mib: 22_528,
        epoch_capacity_mib: 21_504,
        last_physical_free_mib: 22_528,
        reserved_mib,
        available_reservation_mib: 21_504 - reserved_mib,
        admitted_total: 1,
        rejected_total: 0,
        stale_reaped_total: 0,
        reservations,
        last_rejection: None,
    }
}

fn published(reservation_id: &str, lease_pid: u64) -> PublishedResidentLease {
    PublishedResidentLease {
        reservation_id: reservation_id.to_string(),
        lease_pid,
    }
}

/// A resident publishing a live lease validates and returns the snapshot
/// unchanged — admission reads authority state, it never mints a second row.
#[test]
fn live_matching_lease_is_admitted_without_minting_a_reservation() {
    let value = ready(
        Value::from("100c2b891114f9a891eafc508da24465"),
        Value::from(1210172),
    );
    let published = published_resident_lease(OP, addr(), &value).expect("lease published");
    assert_eq!(published.reservation_id, "100c2b891114f9a891eafc508da24465");
    assert_eq!(published.lease_pid, 1_210_172);

    let before = snapshot(vec![row("100c2b891114f9a891eafc508da24465", 1_210_172)]);
    let after = match_live_lease_row(OP, addr(), &published, before.clone())
        .expect("live matching lease admits");

    assert_eq!(after.reservations.len(), 1, "count stays exactly 1");
    assert_eq!(after.reserved_mib, before.reserved_mib);
    assert_eq!(after.reservations, before.reservations);
}

/// A reachable resident that published no reservation refuses `_ABSENT`.
#[test]
fn missing_reservation_id_refuses_lease_absent() {
    let err = published_resident_lease(OP, addr(), &ready(Value::Null, Value::from(1210172)))
        .expect_err("absent reservation_id must refuse");
    assert_eq!(err.code(), "CALYX_RESIDENT_LEASE_ABSENT");
}

/// A blank/whitespace reservation_id is absence, not a valid identity.
#[test]
fn blank_reservation_id_refuses_lease_absent() {
    let err =
        published_resident_lease(OP, addr(), &ready(Value::from("   "), Value::from(1210172)))
            .expect_err("blank reservation_id must refuse");
    assert_eq!(err.code(), "CALYX_RESIDENT_LEASE_ABSENT");
}

/// A published reservation_id without an owning pid is also `_ABSENT`.
#[test]
fn missing_lease_pid_refuses_lease_absent() {
    let err = published_resident_lease(
        OP,
        addr(),
        &ready(Value::from("100c2b891114f9a891eafc508da24465"), Value::Null),
    )
    .expect_err("absent lease_pid must refuse");
    assert_eq!(err.code(), "CALYX_RESIDENT_LEASE_ABSENT");
}

/// The SIGKILL-orphan production state: the resident still serves and still
/// publishes its reservation, but `prune_stale` has reaped the row, so the
/// store holds no live lease. That is `_STALE`, not `_ABSENT`.
#[test]
fn reaped_reservation_row_refuses_lease_stale() {
    let published = published("100c2b891114f9a891eafc508da24465", 1_210_172);
    let err = match_live_lease_row(OP, addr(), &published, snapshot(Vec::new()))
        .expect_err("reaped lease row must refuse");
    assert_eq!(err.code(), "CALYX_RESIDENT_LEASE_STALE");
}

/// A live store holding some *other* reservation is still `_STALE` for this
/// resident — matching is by exact reservation id, never "some lease exists".
#[test]
fn unrelated_live_reservation_still_refuses_lease_stale() {
    let published = published("100c2b891114f9a891eafc508da24465", 1_210_172);
    let err = match_live_lease_row(
        OP,
        addr(),
        &published,
        snapshot(vec![row("ffffffffffffffffffffffffffffffff", 1_210_172)]),
    )
    .expect_err("non-matching reservation id must refuse");
    assert_eq!(err.code(), "CALYX_RESIDENT_LEASE_STALE");
}

/// The restart case: the reservation id is live but a new wrapper owns it, so
/// the client's discovery record carries the previous identity.
#[test]
fn live_row_owned_by_another_pid_refuses_identity_mismatch() {
    let published = published("100c2b891114f9a891eafc508da24465", 1_210_172);
    let err = match_live_lease_row(
        OP,
        addr(),
        &published,
        snapshot(vec![row("100c2b891114f9a891eafc508da24465", 2_222_222)]),
    )
    .expect_err("pid mismatch must refuse");
    assert_eq!(err.code(), "CALYX_RESIDENT_IDENTITY_MISMATCH");
}

/// Every refusal names the lease remediation; none of them suggests a CPU
/// fallback or a second reservation.
#[test]
fn refusals_never_offer_a_fallback_remediation() {
    let published = published("100c2b891114f9a891eafc508da24465", 1_210_172);
    let errors = [
        published_resident_lease(OP, addr(), &ready(Value::Null, Value::Null)).expect_err("absent"),
        match_live_lease_row(OP, addr(), &published, snapshot(Vec::new())).expect_err("stale"),
        match_live_lease_row(
            OP,
            addr(),
            &published,
            snapshot(vec![row("100c2b891114f9a891eafc508da24465", 2_222_222)]),
        )
        .expect_err("mismatch"),
    ];
    for err in errors {
        let remediation = err.remediation();
        assert!(
            remediation.contains("calyx gpu run"),
            "remediation must point at the lease-owning wrapper: {remediation}"
        );
        assert!(
            remediation.contains("never mint a second reservation"),
            "remediation must forbid double-booking: {remediation}"
        );
    }
}
