//! Route-aware GPU admission for batch ingest, issue #1936.
//!
//! The pre-fix ingest path demanded the *caller's* own live lease before the
//! measurement route was resolved (`cmd/ingest/command.rs`). On the resident
//! route the VRAM consumer is the resident — which already runs under a live
//! host lease — so requiring the caller to hold a second lease double-booked
//! `epoch_capacity_mib` and refused legitimate un-leased ingest clients.
//!
//! [`admit_batch_gpu`] resolves admission AFTER the route is known. No
//! GPU-backed lens spec means no lease at all (the #1925 CPU path: CPU-native
//! batches proceed with an empty reservation table). GPU work on the local
//! route (no resident, in-process workers) requires the caller to hold a live
//! lease (`CALYX_BACKPRESSURE` otherwise — the #1925 local-route regression
//! guard) because the caller's own process consumes the VRAM. GPU work on the
//! resident route VALIDATES the resident's live lease row — its published
//! `reservation_id` must be a live row whose owning pid matches the resident's
//! published `lease_pid` — and NEVER mints a second reservation, so the
//! reservation count is unchanged by admission.
//!
//! Fail-closed structured codes for the resident branch:
//! `CALYX_RESIDENT_LEASE_ABSENT` (resident published no reservation),
//! `CALYX_RESIDENT_LEASE_STALE` (published reservation is not a live row —
//! reaped or never admitted), and `CALYX_RESIDENT_IDENTITY_MISMATCH` (the live
//! row's pid differs from the published `lease_pid`).

use std::net::SocketAddr;

use calyx_core::CalyxError;
use calyx_forge::vram::{HostGpuReservationSnapshot, HostGpuReservationStore};
use calyx_registry::LensSpec;
use serde_json::Value;

use crate::error::{CliError, CliResult};

const RESIDENT_LEASE_REMEDIATION: &str = "route ingest at a resident started under a live host GPU lease (`calyx gpu run --mib <measured-peak> --job <stable-id> -- calyx panel resident serve ...`); never mint a second reservation for the same VRAM, fall back to CPU, or bypass admission";

/// Resolve GPU admission for a batch after the measurement route is known.
///
/// Returns the host reservation snapshot consulted (`Some`) when GPU work is
/// admitted, or `None` when the batch has no GPU-backed lens spec and needs no
/// lease.
pub(crate) fn admit_batch_gpu(
    operation: &'static str,
    specs: &[LensSpec],
    resident_addr: Option<SocketAddr>,
) -> CliResult<Option<HostGpuReservationSnapshot>> {
    if !specs
        .iter()
        .any(|spec| super::runtime_uses_gpu(&spec.runtime))
    {
        // No GPU-backed lens is applicable: no lease is required. This is the
        // #1925 CPU-native path — the reservation table stays untouched.
        return Ok(None);
    }
    match resident_addr {
        // Resident route: the resident owns the VRAM under its own live lease.
        // Validate it rather than acquiring a second reservation.
        Some(addr) => validate_resident_lease(operation, addr).map(Some),
        // Local route (in-process GPU workers): the caller consumes the VRAM,
        // so the caller must hold a live lease.
        None => super::require_active_reservation(operation).map(Some),
    }
}

/// The lease identity a resident publishes in its readiness response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PublishedResidentLease {
    pub(crate) reservation_id: String,
    pub(crate) lease_pid: u64,
}

/// Confirm the resident at `addr` is running under a live host GPU lease whose
/// identity matches what it published, without minting a new reservation.
///
/// The two decision steps are pure functions ([`published_resident_lease`] and
/// [`match_live_lease_row`]) so every refusal branch is unit-testable without a
/// GPU and without fault-injecting live reservation state.
fn validate_resident_lease(
    operation: &'static str,
    addr: SocketAddr,
) -> CliResult<HostGpuReservationSnapshot> {
    let ready = crate::panel_commands::resident_ready_value_at(addr)?;
    let published = published_resident_lease(operation, addr, &ready)?;

    // `readback` prunes crashed leases, so a reservation absent here is dead.
    let snapshot = HostGpuReservationStore::from_env(0)
        .and_then(|store| store.readback())
        .map_err(super::reservation_error)?;
    match_live_lease_row(operation, addr, &published, snapshot)
}

/// Extract the resident's published lease identity, or refuse `_ABSENT`.
pub(crate) fn published_resident_lease(
    operation: &'static str,
    addr: SocketAddr,
    ready: &Value,
) -> CliResult<PublishedResidentLease> {
    let reservation_id = ready
        .get("reservation_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let lease_pid = ready.get("lease_pid").and_then(Value::as_u64);
    if reservation_id.is_empty() || lease_pid.is_none() {
        return Err(CliError::from(CalyxError {
            code: "CALYX_RESIDENT_LEASE_ABSENT",
            message: format!(
                "{operation}: resident {addr} published no live GPU reservation (reservation_id={reservation_id:?}, lease_pid={lease_pid:?}); it is not holding a lease this client can validate"
            ),
            remediation: RESIDENT_LEASE_REMEDIATION,
        }));
    }
    Ok(PublishedResidentLease {
        reservation_id,
        lease_pid: lease_pid.expect("lease_pid checked present"),
    })
}

/// Match the published lease against the live reservation rows, or refuse
/// `_STALE` (no such live row) / `_IDENTITY_MISMATCH` (row owned by another pid).
pub(crate) fn match_live_lease_row(
    operation: &'static str,
    addr: SocketAddr,
    published: &PublishedResidentLease,
    snapshot: HostGpuReservationSnapshot,
) -> CliResult<HostGpuReservationSnapshot> {
    let PublishedResidentLease {
        reservation_id,
        lease_pid,
    } = published;
    let Some(row) = snapshot
        .reservations
        .iter()
        .find(|row| &row.reservation_id == reservation_id)
    else {
        return Err(CliError::from(CalyxError {
            code: "CALYX_RESIDENT_LEASE_STALE",
            message: format!(
                "{operation}: resident {addr} published reservation {reservation_id} but it is not a live row in {} (reaped or never admitted)",
                snapshot.state_path
            ),
            remediation: RESIDENT_LEASE_REMEDIATION,
        }));
    };
    if u64::from(row.pid) != *lease_pid {
        return Err(CliError::from(CalyxError {
            code: "CALYX_RESIDENT_IDENTITY_MISMATCH",
            message: format!(
                "{operation}: resident {addr} published lease_pid={lease_pid} for reservation {reservation_id} but the live row is owned by pid {} in {}",
                row.pid, snapshot.state_path
            ),
            remediation: RESIDENT_LEASE_REMEDIATION,
        }));
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests;
