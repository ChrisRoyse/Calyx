use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bincode::config;
use calyx_core::{
    AbsentReason, CalyxError, CxId, Input, LensId, Modality, SlotId, SlotState, SlotVector,
};
use calyx_lodestar::{TypedPanelFusionHit, TypedPanelVectors};
use calyx_search::{
    FusionChoice, GuardChoice, PersistedSearchGeneration, SearchFreshness, SearchOutcome,
    SearchSlotCache,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

mod discovery;
mod flags;
mod protocol;

pub(crate) use discovery::{ResidentDiscovery, read_resident_discovery, resident_discovery_path};

use flags::{
    ServeFlags, calyx_home, ensure_loopback, parse_addr, parse_client_flags, parse_serve_flags,
};
use protocol::{
    ClientMeasureInput, KERNEL_RANK_BINARY_PROTOCOL_VERSION, MAX_KERNEL_RANK_EVIDENCE_JSON_BYTES,
    MAX_RESIDENT_SEARCH_OUTCOME_JSON_BYTES, MEASURE_BATCH_SCHEMA, MEASURE_SCHEMA, MeasureResponse,
    READY_SCHEMA, RESIDENT_BINARY_PROTOCOL_VERSION, ReadyResponse, ResidentKernelRankBinaryRequest,
    ResidentKernelRankBinaryResponse, ResidentKernelRankReady, ResidentMeasureBatchBinaryRequest,
    ResidentMeasureBatchStreamEnd, ResidentMeasureBatchStreamFrame,
    ResidentMeasureBatchStreamHeader, ResidentRequest, ResidentSearchAtResponse,
    ResidentSearchBinaryRequest, ResidentSearchBinaryResponse, SEARCH_BINARY_PROTOCOL_VERSION,
    hex_decode,
};
pub(crate) use protocol::{
    KernelRankAtResponse, MeasureBatchAtResponse, MeasureBatchResponse,
    MeasureBatchSummaryResponse, ResidentMeasuredInput, ResidentSearchTrace, ResidentSlotMeasure,
};

use super::warm::resident_support::{
    ResidentWarmOptions, ResidentWarmState, load_resident_warm_state,
};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_BIND: &str = "127.0.0.1:8787";
const DEFAULT_MAX_RESIDENT_VRAM_MIB: u64 = 22 * 1024;
const DEFAULT_RESIDENT_OVERHEAD_MULTIPLIER_MILLI: u64 = 2100;
const DEFAULT_MAX_LOAD_SECS: u64 = 60;
const DEFAULT_MAX_RUNTIME_BATCH: usize = 4;
/// Default resident-service socket timeout. One request measures a whole
/// window of inputs across every panel lane, so a panel of long-document
/// lanes -- where a single opinion is segmented, tokenized and embedded per
/// lane -- legitimately takes longer than a short-text panel.
const DEFAULT_CLIENT_TIMEOUT_SECS: u64 = 30;
const CLIENT_TIMEOUT_SECS_ENV: &str = "CALYX_RESIDENT_CLIENT_TIMEOUT_SECS";
const MAX_CLIENT_TIMEOUT_SECS: u64 = 3600;

/// Resident-service socket timeout, overridable for long-document panels.
///
/// Bounded and fail-closed: an unparseable, zero, or out-of-range value is an
/// error rather than a silent fallback, because a silently-wrong timeout shows
/// up as an unexplained `CALYX_PANEL_RESIDENT_BINARY_FRAME` mid-ingest.
fn client_timeout() -> CliResult<Duration> {
    let Some(raw) = std::env::var_os(CLIENT_TIMEOUT_SECS_ENV) else {
        return Ok(Duration::from_secs(DEFAULT_CLIENT_TIMEOUT_SECS));
    };
    let raw = raw.to_string_lossy().trim().to_string();
    let secs: u64 = raw.parse().map_err(|_| {
        CliError::usage(format!(
            "{CLIENT_TIMEOUT_SECS_ENV}={raw:?} is not a whole number of seconds"
        ))
    })?;
    if secs == 0 || secs > MAX_CLIENT_TIMEOUT_SECS {
        return Err(CliError::usage(format!(
            "{CLIENT_TIMEOUT_SECS_ENV}={secs} is outside 1..={MAX_CLIENT_TIMEOUT_SECS}"
        )));
    }
    Ok(Duration::from_secs(secs))
}
const CLIENT_TIMEOUT_REMEDIATION: &str =
    "start `calyx panel resident serve` on the requested loopback address";
const RESIDENT_BINARY_MAGIC: &[u8] = b"CALYX_PANEL_RESIDENT_BIN1\n";
const KERNEL_RANK_BINARY_MAGIC: &[u8] = b"CALYX_KERNEL_RANK_BIN1\n";
const SEARCH_BINARY_MAGIC: &[u8] = b"CALYX_RESIDENT_SEARCH_BIN1\n";
const MAX_RESIDENT_SERVICE_FRAME_BYTES: usize = 2 * 1024 * 1024 * 1024;

mod capacity;
mod client;
mod codec;
mod dispatch;
mod input_wire;
mod kernel_rank;
mod parallel;
mod search;
mod server;
mod stream;

#[cfg(test)]
mod tests;

pub(crate) use client::{kernel_rank_at, measure_batch_at, ready_value_at, search_at};
pub(crate) fn run(args: &[String]) -> CliResult {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(CliError::usage(
            "calyx panel resident requires serve, ready, measure, measure-batch, or stop",
        ));
    };
    match command {
        "serve" => {
            let snapshot =
                crate::gpu_commands::require_active_reservation("calyx panel resident serve")?;
            let lease = resolve_resident_lease_identity(&snapshot)?;
            server::serve(&args[1..], lease)
        }
        "ready" => client::client_command(&args[1..], "ready"),
        "measure" => client::client_command(&args[1..], "measure"),
        "measure-batch" => client::client_command(&args[1..], "measure-batch"),
        "stop" => client::client_command(&args[1..], "shutdown"),
        other => Err(CliError::usage(format!(
            "unknown panel resident subcommand {other}; expected serve, ready, measure, measure-batch, or stop"
        ))),
    }
}

/// The live host GPU lease the resident validated at startup and republishes
/// so ingest clients can VALIDATE it instead of minting a second reservation
/// (#1936). Threaded from `run` into `server::serve` → readiness + discovery.
pub(crate) struct ResidentLeaseIdentity {
    pub(crate) reservation_id: String,
    pub(crate) lease_pid: u32,
    pub(crate) requested_mib: u64,
}

/// Resolve the reservation this resident is running under from the same live
/// snapshot `require_active_reservation` already validated: the env-named
/// reservation id plus the pid that owns its store row. Fails closed if the
/// validated row cannot be located (it must exist — the caller succeeded).
fn resolve_resident_lease_identity(
    snapshot: &calyx_forge::vram::HostGpuReservationSnapshot,
) -> CliResult<ResidentLeaseIdentity> {
    let reservation_id = crate::gpu_commands::active_reservation_id("calyx panel resident serve")?;
    let lease = snapshot
        .reservations
        .iter()
        .find(|row| row.reservation_id == reservation_id)
        .ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_RESIDENT_LEASE_ABSENT",
                message: format!(
                    "resident reservation {reservation_id} is not present in validated host store snapshot {}",
                    snapshot.state_path
                ),
                remediation:
                    "launch the resident through `calyx gpu run --mib <measured-peak> --job <stable-id> -- calyx panel resident serve ...` so a live lease backs it",
            })
        })?;
    Ok(ResidentLeaseIdentity {
        reservation_id,
        lease_pid: lease.pid,
        requested_mib: lease.requested_mib,
    })
}

fn write_json_file(path: PathBuf, value: &impl Serialize) -> CliResult {
    // Runtime evidence (readiness/stop/measure JSON) must be owner-only; the
    // private writer creates it 0600 under a 0700 parent, atomically, with a
    // mode/length readback that fails closed (issue #1931).
    crate::private_fs::write_private_json(&path, value)
}

fn cli_error_value(error: &CliError) -> Value {
    error_value(error.code(), error.message(), error.remediation())
}

fn error_value(
    code: impl Into<String>,
    message: impl Into<String>,
    remediation: impl Into<String>,
) -> Value {
    json!({
        "ok": false,
        "code": code.into(),
        "message": message.into(),
        "remediation": remediation.into(),
    })
}
