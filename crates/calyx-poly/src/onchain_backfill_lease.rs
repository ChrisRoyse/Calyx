//! Cross-session lease, heartbeat, and max-runtime guard for the on-chain
//! backfill binary (issue #217).
//!
//! A backgrounded backfill that outlived its launching session used to pin its
//! own `.exe` open on Windows (blocking every rebuild) and could keep hitting
//! live endpoints unattended. This module makes such a runner:
//!   * **detectable** — it writes a lease file carrying `pid` + `started_at`
//!     (so a human or a future session can find and reap it), and
//!   * **self-terminating** — a watchdog enforces a hard `--max-runtime`
//!     ceiling, and
//!   * **reapable** — a fresh run auto-reaps a *stale* lease (heartbeat older
//!     than the staleness window, or past its own deadline) instead of
//!     colliding with it, while refusing to trample a *live* peer.
//!
//! Staleness is decided by **heartbeat age**, not by probing the OS for a live
//! PID: the std library has no portable "is this PID alive" primitive, and PID
//! reuse would make such a probe unsound anyway. The `pid`/`started_at`/
//! `hostname` fields are recorded so an operator can reap manually.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use calyx_core::Clock;
use serde::{Deserialize, Serialize};

use crate::raw_source_support::now_unix_ms;
use crate::{PolyError, PolyLogEvent, Result, StructuredLogSink, log_context};

pub const ONCHAIN_BACKFILL_LEASE_FILE: &str = "onchain-backfill.lease.json";
pub const ONCHAIN_BACKFILL_LEASE_SCHEMA_VERSION: &str = "poly.onchain_backfill.lease.v1";

/// Default wall-clock ceiling for one process. A backfill run is bounded and
/// checkpoint-resumable, so a hard stop here is safe and simply resumes next
/// invocation.
pub const DEFAULT_MAX_RUNTIME_SECS: u64 = 1800;
/// How often the heartbeat timestamp is refreshed while the run is alive.
pub const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 10;
/// A lease whose heartbeat is older than `interval * this` is considered
/// abandoned and is reaped by the next run.
pub const LEASE_STALE_AFTER_MISSED_HEARTBEATS: u64 = 6;
/// Exit code used when the watchdog force-terminates a run past its deadline.
pub const MAX_RUNTIME_EXIT_CODE: i32 = 3;

pub const CODE_LEASE_HELD: &str = "POLY_ONCHAIN_BACKFILL_LEASE_HELD";
pub const CODE_LEASE_REAPED: &str = "POLY_ONCHAIN_BACKFILL_LEASE_REAPED";
pub const CODE_LEASE_ACQUIRED: &str = "POLY_ONCHAIN_BACKFILL_LEASE_ACQUIRED";
pub const CODE_MAX_RUNTIME_EXCEEDED: &str = "POLY_ONCHAIN_BACKFILL_MAX_RUNTIME_EXCEEDED";

/// Physical, machine-written record of a running backfill process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillLease {
    pub schema_version: String,
    pub pid: u32,
    pub hostname: String,
    pub command: String,
    pub started_at_unix_ms: u128,
    pub heartbeat_at_unix_ms: u128,
    pub heartbeat_interval_secs: u64,
    pub max_runtime_secs: u64,
    pub deadline_unix_ms: u128,
}

impl BackfillLease {
    fn new(
        command: String,
        started_at_unix_ms: u128,
        heartbeat_interval_secs: u64,
        max_runtime_secs: u64,
    ) -> Self {
        Self {
            schema_version: ONCHAIN_BACKFILL_LEASE_SCHEMA_VERSION.to_string(),
            pid: std::process::id(),
            hostname: hostname(),
            command,
            started_at_unix_ms,
            heartbeat_at_unix_ms: started_at_unix_ms,
            heartbeat_interval_secs,
            max_runtime_secs,
            deadline_unix_ms: started_at_unix_ms
                .saturating_add(u128::from(max_runtime_secs).saturating_mul(1_000)),
        }
    }

    /// The staleness window in milliseconds derived from this lease's own
    /// declared heartbeat cadence.
    fn stale_threshold_ms(&self) -> u128 {
        u128::from(self.heartbeat_interval_secs.max(1))
            .saturating_mul(u128::from(LEASE_STALE_AFTER_MISSED_HEARTBEATS))
            .saturating_mul(1_000)
    }
}

/// What to do when a lease file already exists at acquisition time.
#[derive(Debug, PartialEq, Eq)]
pub enum AcquireDecision {
    /// No live peer — take the lease.
    Proceed,
    /// A prior lease is abandoned; reap it (with a human-readable reason) then
    /// take the lease.
    Reap(String),
    /// A live peer holds the lease; refuse unless `--takeover` was given.
    Blocked { pid: u32, age_ms: u128 },
}

/// Pure decision function (unit-tested): given the existing lease (if any) and
/// the current wall clock, decide whether we may acquire.
pub fn acquire_decision(
    existing: Option<&BackfillLease>,
    now_unix_ms: u128,
    takeover: bool,
) -> AcquireDecision {
    let Some(existing) = existing else {
        return AcquireDecision::Proceed;
    };
    let heartbeat_age_ms = now_unix_ms.saturating_sub(existing.heartbeat_at_unix_ms);
    let past_deadline = now_unix_ms >= existing.deadline_unix_ms;
    let heartbeat_stale = heartbeat_age_ms > existing.stale_threshold_ms();
    if heartbeat_stale || past_deadline {
        let reason = if past_deadline {
            format!(
                "prior lease pid={} exceeded its own deadline ({}ms past)",
                existing.pid,
                now_unix_ms.saturating_sub(existing.deadline_unix_ms)
            )
        } else {
            format!(
                "prior lease pid={} heartbeat stale ({heartbeat_age_ms}ms > {}ms window)",
                existing.pid,
                existing.stale_threshold_ms()
            )
        };
        return AcquireDecision::Reap(reason);
    }
    if takeover {
        return AcquireDecision::Reap(format!(
            "operator --takeover of live lease pid={} (heartbeat age {heartbeat_age_ms}ms)",
            existing.pid
        ));
    }
    AcquireDecision::Blocked {
        pid: existing.pid,
        age_ms: heartbeat_age_ms,
    }
}

/// RAII guard: holds the lease, runs the heartbeat + watchdog threads, and
/// removes the lease file on drop. If the watchdog force-exits the process,
/// it removes the lease itself before exiting so it does not linger.
#[derive(Debug)]
pub struct LeaseGuard {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl LeaseGuard {
    pub fn lease_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
        // Best-effort: the run finished cleanly, so release the lease.
        let _ = fs::remove_file(&self.path);
    }
}

/// Acquire the lease for the current process, reaping a stale prior lease or
/// refusing a live one. On success spawns the heartbeat + max-runtime watchdog
/// and returns a guard that releases the lease on drop.
pub fn acquire<C>(
    lease_path: &Path,
    command: String,
    max_runtime_secs: u64,
    heartbeat_interval_secs: u64,
    takeover: bool,
    sink: &StructuredLogSink,
    clock: C,
) -> Result<LeaseGuard>
where
    C: Clock + Clone + 'static,
{
    let now = now_unix_ms(&clock);
    let existing = read_lease(lease_path)?;
    match acquire_decision(existing.as_ref(), now, takeover) {
        AcquireDecision::Proceed => {}
        AcquireDecision::Reap(reason) => {
            let prior = existing.as_ref();
            sink.append_event(&PolyLogEvent::new(
                &clock,
                crate::PolyLogLevel::Warn,
                "onchain_backfill",
                "lease_reap",
                CODE_LEASE_REAPED,
                format!("reaping abandoned backfill lease: {reason}"),
                log_context(&[
                    ("lease_path", lease_path.display().to_string()),
                    (
                        "prior_pid",
                        prior.map(|l| l.pid.to_string()).unwrap_or_default(),
                    ),
                    (
                        "prior_started_at_unix_ms",
                        prior
                            .map(|l| l.started_at_unix_ms.to_string())
                            .unwrap_or_default(),
                    ),
                    (
                        "prior_command",
                        prior.map(|l| l.command.clone()).unwrap_or_default(),
                    ),
                ]),
            )?)?;
        }
        AcquireDecision::Blocked { pid, age_ms } => {
            return Err(PolyError::raw_source(
                CODE_LEASE_HELD,
                format!(
                    "a live on-chain backfill lease is held by pid={pid} (heartbeat age {age_ms}ms) at {}; \
                     wait for it to finish, reap it, or re-run with --takeover",
                    lease_path.display()
                ),
            ));
        }
    }

    let lease = BackfillLease::new(command, now, heartbeat_interval_secs, max_runtime_secs);
    write_lease_atomic(lease_path, &lease)?;
    sink.append_event(&PolyLogEvent::info(
        &clock,
        "onchain_backfill",
        "lease_acquire",
        CODE_LEASE_ACQUIRED,
        "acquired on-chain backfill lease",
        log_context(&[
            ("lease_path", lease_path.display().to_string()),
            ("pid", lease.pid.to_string()),
            ("started_at_unix_ms", lease.started_at_unix_ms.to_string()),
            ("max_runtime_secs", lease.max_runtime_secs.to_string()),
            ("deadline_unix_ms", lease.deadline_unix_ms.to_string()),
        ]),
    )?)?;

    let stop = Arc::new(AtomicBool::new(false));
    let handles = vec![
        spawn_heartbeat(
            lease_path.to_path_buf(),
            lease.clone(),
            Arc::clone(&stop),
            clock.clone(),
        ),
        spawn_watchdog(
            lease_path.to_path_buf(),
            lease.clone(),
            Arc::clone(&stop),
            sink.clone(),
            clock,
        ),
    ];

    Ok(LeaseGuard {
        path: lease_path.to_path_buf(),
        stop,
        handles,
    })
}

fn spawn_heartbeat<C>(
    path: PathBuf,
    mut lease: BackfillLease,
    stop: Arc<AtomicBool>,
    clock: C,
) -> JoinHandle<()>
where
    C: Clock + 'static,
{
    let tick = Duration::from_secs(lease.heartbeat_interval_secs.max(1));
    thread::spawn(move || {
        loop {
            if sleep_until_stop(&stop, tick) {
                return;
            }
            lease.heartbeat_at_unix_ms = now_unix_ms(&clock);
            // A failed heartbeat write is non-fatal for this tick; the
            // next tick retries. Persistent failure simply makes the
            // lease look stale, which is the safe direction.
            let _ = write_lease_atomic(&path, &lease);
        }
    })
}

fn spawn_watchdog<C>(
    path: PathBuf,
    lease: BackfillLease,
    stop: Arc<AtomicBool>,
    sink: StructuredLogSink,
    clock: C,
) -> JoinHandle<()>
where
    C: Clock + 'static,
{
    // Poll frequently so a clean finish stops the watchdog promptly rather than
    // sleeping until the full deadline.
    let poll = Duration::from_millis(250);
    thread::spawn(move || {
        loop {
            if sleep_until_stop(&stop, poll) {
                return;
            }
            let now = now_unix_ms(&clock);
            if now >= lease.deadline_unix_ms {
                let elapsed_ms = now.saturating_sub(lease.started_at_unix_ms);
                let error = PolyError::raw_source(
                    CODE_MAX_RUNTIME_EXCEEDED,
                    format!(
                        "on-chain backfill exceeded --max-runtime {}s (elapsed {elapsed_ms}ms); \
                         force-terminating (safe: resumable from checkpoint on next run)",
                        lease.max_runtime_secs
                    ),
                );
                let _ = sink.append_error(
                    &clock,
                    "onchain_backfill",
                    "max_runtime_watchdog",
                    &error,
                    log_context(&[
                        ("pid", lease.pid.to_string()),
                        ("started_at_unix_ms", lease.started_at_unix_ms.to_string()),
                        ("deadline_unix_ms", lease.deadline_unix_ms.to_string()),
                        ("elapsed_ms", elapsed_ms.to_string()),
                    ]),
                );
                // Release the lease before the hard exit so it does not linger
                // as a phantom live lease.
                let _ = fs::remove_file(&path);
                std::process::exit(MAX_RUNTIME_EXIT_CODE);
            }
        }
    })
}

/// Sleep for `dur`, waking early if `stop` is set. Returns `true` if a stop was
/// observed (caller should exit), `false` on a normal timeout.
fn sleep_until_stop(stop: &AtomicBool, dur: Duration) -> bool {
    let step = Duration::from_millis(100);
    let mut remaining = dur;
    loop {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        let nap = remaining.min(step);
        if nap.is_zero() {
            return false;
        }
        thread::sleep(nap);
        remaining = remaining.saturating_sub(nap);
    }
}

pub fn read_lease(path: &Path) -> Result<Option<BackfillLease>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_LEASE_READ_FAILED",
                format!("read backfill lease {}: {err}", path.display()),
            ));
        }
    };
    let lease: BackfillLease = serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_LEASE_PARSE_FAILED",
            format!("parse backfill lease {}: {err}", path.display()),
        )
    })?;
    Ok(Some(lease))
}

pub fn write_lease_atomic(path: &Path, lease: &BackfillLease) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(lease).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_LEASE_ENCODE_FAILED",
            format!("encode backfill lease: {err}"),
        )
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_LEASE_DIR_FAILED",
                format!("create lease dir {}: {err}", parent.display()),
            )
        })?;
    }
    let tmp = path.with_extension("lease.tmp");
    fs::write(&tmp, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_LEASE_WRITE_FAILED",
            format!("write backfill lease tmp {}: {err}", tmp.display()),
        )
    })?;
    fs::rename(&tmp, path).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_LEASE_RENAME_FAILED",
            format!("rename backfill lease into place {}: {err}", path.display()),
        )
    })?;
    Ok(())
}

/// The lease file path for a given backfill output root.
pub fn lease_path_for_output_root(output_root: &Path) -> PathBuf {
    output_root.join(ONCHAIN_BACKFILL_LEASE_FILE)
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
#[path = "onchain_backfill_lease_tests.rs"]
mod tests;
