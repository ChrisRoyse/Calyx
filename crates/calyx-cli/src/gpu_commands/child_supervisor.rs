//! Supervised execution of a GPU-reserved child (`calyx gpu run`), issue #1932.
//!
//! The pre-fix `reservation_run` spawned the child in the wrapper's own process
//! group and blocked in `Command::status()` with no signal handling. A SIGTERM
//! to the wrapper killed it immediately: `status()` was abandoned, the
//! reservation was never released, and the child — still consuming VRAM —
//! reparented to PID 1. The next `prune_stale` then reaped the reservation row
//! while the GPU-consuming child was still alive (accounting erased, device
//! silently oversubscribable).
//!
//! This supervisor closes that gap on Unix. The child leads its own
//! session/process group (`setsid` via `pre_exec`). SIGTERM/SIGINT to the
//! wrapper set an atomic flag (self-pipe-free, via `signal_hook::flag`) which
//! the supervise loop observes and forwards to the whole child group with
//! `kill(-pgid, SIGTERM)`; a bounded grace period then escalates to `SIGKILL`.
//! The lease is released ONLY after the child's exit is confirmed via
//! `try_wait` (reaping the group leader). If the child cannot be confirmed dead
//! even after `SIGKILL`, the wrapper fails closed with a structured error
//! carrying the wrapper PID, child pgid, and reservation id, and does NOT
//! release: the reservation is leaked rather than falsely released, because the
//! true consumer state is unknown and admission must stay conservative.

use calyx_forge::vram::HostGpuReservation;

use crate::error::CliResult;

/// Grace after a forwarded SIGTERM before escalating to SIGKILL.
#[cfg(unix)]
const GRACE_SECS: u64 = 15;
/// Bound on confirming death after SIGKILL before failing closed.
#[cfg(unix)]
const KILL_CONFIRM_SECS: u64 = 10;

/// Spawn, supervise, and (only after confirmed child exit) release the lease.
pub(super) fn supervise(
    child_program: &str,
    child_args: &[String],
    reservation: HostGpuReservation,
) -> CliResult<()> {
    imp::supervise(child_program, child_args, reservation)
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// Emit the released event and map the confirmed child exit to a CLI result.
fn release_and_finish(
    reservation: HostGpuReservation,
    child_program: &str,
    child_pid: u32,
    exit_display: String,
    completion: ChildCompletion,
) -> CliResult<()> {
    let released = reservation.release().map_err(super::reservation_error)?;
    super::print_event(&serde_json::json!({
        "event": "gpu_child_released",
        "at_unix_ms": now_ms(),
        "child_pid": child_pid,
        "child_exit": exit_display,
        "source_of_truth": released,
    }))?;
    match completion {
        // Natural exit: propagate the child's own success/failure.
        ChildCompletion::Natural(true) => Ok(()),
        ChildCompletion::Natural(false) => {
            Err(crate::error::CliError::from(calyx_core::CalyxError {
                code: "CALYX_GPU_RESERVED_CHILD_FAILED",
                message: format!("GPU-reserved child {child_program:?} exited with {exit_display}"),
                remediation: "inspect the child structured error and its source-of-truth state; repair the named failure before retrying the same immutable job",
            }))
        }
        // Forwarded termination we requested: the teardown and lease release
        // are the wrapper's own hygiene, not job success — the child never
        // completed its work. Exiting 0 here lets a driver read an aborted run
        // as success (issue #1995: a SIGTERM'd weave window logged WEAVE_RC=0
        // and proceeded to kernel-build on a half-woven vault), so fail closed
        // with a code distinct from a child's own failure.
        #[cfg(unix)]
        ChildCompletion::Forwarded(signal) => Err(crate::error::CliError::interrupted(
            calyx_core::CalyxError {
                code: "CALYX_GPU_RESERVED_CHILD_ABORTED",
                message: format!(
                    "GPU-reserved child {child_program:?} was terminated by a forwarded signal before completing: {exit_display}"
                ),
                remediation: "the run was aborted by an operator/system signal after a clean lease release; re-run the job when intended — never treat this exit as job completion",
            },
            signal,
        )),
    }
}

enum ChildCompletion {
    Natural(bool),
    #[cfg(unix)]
    Forwarded(i32),
}

#[cfg(unix)]
mod imp {
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use calyx_forge::vram::HostGpuReservation;
    use signal_hook::consts::{SIGINT, SIGTERM};

    use super::{ChildCompletion, GRACE_SECS, KILL_CONFIRM_SECS, now_ms, release_and_finish};
    use crate::error::{CliError, CliResult};

    const POLL: Duration = Duration::from_millis(100);

    pub(super) fn supervise(
        child_program: &str,
        child_args: &[String],
        reservation: HostGpuReservation,
    ) -> CliResult<()> {
        let signal = Arc::new(AtomicUsize::new(0));
        for signum in [SIGTERM, SIGINT] {
            signal_hook::flag::register_usize(signum, Arc::clone(&signal), signum as usize)
                .map_err(|error| {
                    CliError::io(format!(
                        "install GPU-run termination handler for signal {signum}: {error}"
                    ))
                })?;
        }

        let reservation_id = reservation.reservation_id().to_string();
        let wrapper_pid = std::process::id();

        let mut command = Command::new(child_program);
        command
            .args(child_args)
            .env(super::super::ACTIVE_RESERVATION_ID_ENV, &reservation_id);
        // The child leads its own session/process group so the wrapper can
        // forward a termination request to the entire group and never abandon
        // it. `setsid` in the forked child (pre-exec) is async-signal-safe.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|error| {
            CliError::io(format!(
                "start GPU-reserved child {child_program:?} with {} args: {error}",
                child_args.len()
            ))
        })?;
        let child_pid = child.id();
        // The child is its own group leader, so pgid == pid.
        let pgid = child_pid as i32;

        loop {
            if let Some(status) = child.try_wait().map_err(wait_error)? {
                // Natural exit before any termination request.
                let success = status.success();
                return release_and_finish(
                    reservation,
                    child_program,
                    child_pid,
                    format!("{status}"),
                    ChildCompletion::Natural(success),
                );
            }
            let pending = signal.load(Ordering::SeqCst);
            if pending != 0 {
                match terminate_group(&mut child, pgid, wrapper_pid, &reservation_id, pending) {
                    Ok(status) => {
                        return release_and_finish(
                            reservation,
                            child_program,
                            child_pid,
                            format!("{status}"),
                            ChildCompletion::Forwarded(pending as i32),
                        );
                    }
                    Err(error) => {
                        // Cannot confirm the child dead: do NOT release. Leak
                        // the handle so `Drop` does not remove the row / lease
                        // file and falsely free the accounting for a possibly
                        // live consumer.
                        std::mem::forget(reservation);
                        return Err(error);
                    }
                }
            }
            std::thread::sleep(POLL);
        }
    }

    /// Forward the wrapper's termination to the child's whole process group,
    /// escalate to SIGKILL after a bounded grace, and return the confirmed exit
    /// status. Fails closed if the child cannot be confirmed dead.
    fn terminate_group(
        child: &mut Child,
        pgid: i32,
        wrapper_pid: u32,
        reservation_id: &str,
        pending_signum: usize,
    ) -> CliResult<std::process::ExitStatus> {
        super::super::print_event(&serde_json::json!({
            "event": "gpu_wrapper_termination_forwarded",
            "at_unix_ms": now_ms(),
            "wrapper_pid": wrapper_pid,
            "child_pgid": pgid,
            "received_signal": signal_name(pending_signum),
            "forwarded_signal": "SIGTERM",
            "reservation_id": reservation_id,
        }))?;
        send_group(pgid, SIGTERM);
        if let Some(status) = wait_until(child, Duration::from_secs(GRACE_SECS))? {
            return Ok(status);
        }
        // Child ignored SIGTERM for the whole grace: escalate.
        super::super::print_event(&serde_json::json!({
            "event": "gpu_child_sigkill_escalation",
            "at_unix_ms": now_ms(),
            "wrapper_pid": wrapper_pid,
            "child_pgid": pgid,
            "grace_secs": GRACE_SECS,
            "reservation_id": reservation_id,
        }))?;
        send_group(pgid, libc::SIGKILL);
        if let Some(status) = wait_until(child, Duration::from_secs(KILL_CONFIRM_SECS))? {
            return Ok(status);
        }
        Err(CliError::from(calyx_core::CalyxError {
            code: "CALYX_GPU_CHILD_UNREAPED",
            message: format!(
                "GPU-reserved child could not be confirmed dead after SIGKILL: wrapper_pid={wrapper_pid} child_pgid={pgid} reservation_id={reservation_id}; the reservation was NOT released because a live VRAM consumer may remain"
            ),
            remediation: "inspect the child process group (it may be in uninterruptible sleep); do not reuse the device until the group is confirmed gone and the reservation reaped",
        }))
    }

    fn wait_until(
        child: &mut Child,
        budget: Duration,
    ) -> CliResult<Option<std::process::ExitStatus>> {
        let deadline = Instant::now() + budget;
        loop {
            if let Some(status) = child.try_wait().map_err(wait_error)? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(POLL);
        }
    }

    fn send_group(pgid: i32, signum: i32) {
        // Negative pid targets the process group led by `pgid`.
        unsafe {
            libc::kill(-pgid, signum);
        }
    }

    fn wait_error(error: std::io::Error) -> CliError {
        CliError::io(format!("reap GPU-reserved child: {error}"))
    }

    fn signal_name(signum: usize) -> &'static str {
        match signum as i32 {
            SIGINT => "SIGINT",
            SIGTERM => "SIGTERM",
            _ => "signal",
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use std::process::Command;

    use calyx_forge::vram::HostGpuReservation;

    use super::{ChildCompletion, release_and_finish};
    use crate::error::{CliError, CliResult};

    // Non-Unix hosts have no POSIX process-group/signal-forwarding semantics;
    // the GPU/CUDA surface only runs on Linux. Retain the direct spawn/wait/
    // release lifecycle so the crate builds and behaves as before off-Unix.
    pub(super) fn supervise(
        child_program: &str,
        child_args: &[String],
        reservation: HostGpuReservation,
    ) -> CliResult<()> {
        let child_pid;
        let status = {
            let mut child = Command::new(child_program)
                .args(child_args)
                .env(
                    super::super::ACTIVE_RESERVATION_ID_ENV,
                    reservation.reservation_id(),
                )
                .spawn()
                .map_err(|error| {
                    CliError::io(format!(
                        "start GPU-reserved child {child_program:?} with {} args: {error}",
                        child_args.len()
                    ))
                })?;
            child_pid = child.id();
            child
                .wait()
                .map_err(|error| CliError::io(format!("reap GPU-reserved child: {error}")))?
        };
        release_and_finish(
            reservation,
            child_program,
            child_pid,
            format!("{status}"),
            ChildCompletion::Natural(status.success()),
        )
    }
}
