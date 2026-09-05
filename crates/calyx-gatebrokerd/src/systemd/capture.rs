use super::account::{
    processes_in_cgroup, verify_worker_idle_account, verify_worker_manager_absent, worker_processes,
};
use super::builder::{build_systemd_run, hex_token, kernel_random_token, shim_argv};
use super::cgroup::poll_pidfd;
use super::evidence::{parse_unit_evidence, verify_process_identity};
use super::manager::{verify_broker_unit, verify_fixed_slice_contract, verify_systemd_contract};
use super::running::duplicate_abort;
use super::validation::{duplicate_fd, fstat, io_error, trusted_executable, validate_spec};
use super::*;

#[derive(Debug)]
struct ChildCleanup {
    reaped: bool,
    detail: String,
}

fn cleanup_child(child: &mut Child) -> ChildCleanup {
    let deadline = Instant::now() + DRAIN_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return ChildCleanup {
                    reaped: true,
                    detail: format!("systemd-run reaped status={status}"),
                };
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                let kill = child.kill();
                let wait = child.wait();
                return ChildCleanup {
                    reaped: wait.is_ok(),
                    detail: format!("systemd-run timeout kill={kill:?} wait={wait:?}"),
                };
            }
            Err(error) => {
                return ChildCleanup {
                    reaped: false,
                    detail: format!("systemd-run try_wait failed: {error}"),
                };
            }
        }
    }
}

pub(super) fn cleanup_unbound(child: &mut Child, prefix: &str) -> CleanupProof {
    let child = cleanup_child(child);
    CleanupProof::unbound(format!("{prefix}{}", child.detail), child.reaped)
}

pub(super) fn cleanup_bound(
    child: &mut Child,
    stage_pidfd: &ProcessFd,
    service: &CgroupAuthority,
    slice: &CgroupAuthority,
) -> CleanupProof {
    let service_kill = service.kill_if_populated();
    let slice_kill = slice.kill_if_populated();
    let first_pid_wait = stage_pidfd.wait_timeout(DRAIN_TIMEOUT);
    let (pidfd_exited, pid) = if matches!(first_pid_wait, Ok(true)) {
        (true, format!("initial_wait={first_pid_wait:?}"))
    } else {
        let kill = stage_pidfd.kill();
        let wait = stage_pidfd.wait_timeout(DRAIN_TIMEOUT);
        (
            matches!(wait, Ok(true)),
            format!("initial_wait={first_pid_wait:?} forced_kill={kill:?} final_wait={wait:?}"),
        )
    };
    let service_empty = service.prove_empty(DRAIN_TIMEOUT);
    let slice_empty = slice.prove_empty(DRAIN_TIMEOUT);
    let child = cleanup_child(child);
    CleanupProof {
        pidfd_exited,
        service_cgroup_empty: service_empty.is_ok(),
        slice_cgroup_empty: slice_empty.is_ok(),
        systemd_run_reaped: child.reaped,
        detail: format!(
            "service_kill={service_kill:?} slice_kill={slice_kill:?} pid={pid} service_empty={service_empty:?} slice_empty={slice_empty:?} {}",
            child.detail
        ),
    }
}

impl CapturedStage {
    pub fn capture(
        spec: &StageSpec,
        stdout_fd: RawFd,
        stderr_fd: RawFd,
    ) -> Result<Self, SystemdError> {
        validate_spec(spec)?;
        let real_uid = unsafe { libc::getuid() };
        let effective_uid = unsafe { libc::geteuid() };
        if real_uid != 0 || effective_uid != 0 {
            return Err(SystemdError::BrokerIdentity {
                real_uid,
                effective_uid,
            });
        }
        verify_systemd_contract()?;
        verify_broker_unit(BROKER_UNIT_NAME)?;
        verify_fixed_slice_contract()?;
        let account = verify_worker_idle_account(&spec.worker_user, spec.worker_uid)?;
        let shim = trusted_executable(STAGE_SHIM, true)?;
        let cgroup_root = CgroupRoot::open()?;
        let cwd_guard = duplicate_fd(spec.cwd_fd, "duplicate resolved cwd")?;
        let cwd_stat = fstat(cwd_guard.as_raw_fd(), "fstat resolved cwd")?;
        if cwd_stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(SystemdError::InvalidSpec(
                "cwd fd is not a directory".into(),
            ));
        }
        let mut release_token = kernel_random_token()?;
        let token_hex = hex_token(&release_token);
        let shim_arguments = shim_argv(spec, &token_hex, &cwd_stat);
        let mut command = build_systemd_run(spec, &account, &shim_arguments, stdout_fd, stderr_fd)?;
        let mut child = command
            .spawn()
            .map_err(|source| io_error("launch /usr/bin/systemd-run", source))?;
        let release_pipe = match child.stdin.take() {
            Some(pipe) => pipe,
            None => {
                release_token.fill(0);
                let cleanup = cleanup_unbound(&mut child, "");
                return Err(SystemdError::Cleanup {
                    primary: SystemdError::Release(
                        "systemd-run did not expose the release pipe".into(),
                    )
                    .to_string(),
                    cleanup,
                });
            }
        };
        let deadline = Instant::now() + START_TIMEOUT;
        let (evidence, stage_pidfd, service, slice) = loop {
            let mut evidence =
                match parse_unit_evidence(&spec.unit_name, &spec.worker_user, account.uid) {
                    Ok(evidence) => evidence,
                    Err(error) if Instant::now() < deadline => {
                        let _ = error;
                        thread::sleep(POLL_INTERVAL);
                        continue;
                    }
                    Err(error) => {
                        drop(release_pipe);
                        release_token.fill(0);
                        let cleanup = cleanup_unbound(&mut child, "");
                        return Err(SystemdError::Cleanup {
                            primary: error.to_string(),
                            cleanup,
                        });
                    }
                };
            let stage_pidfd = match ProcessFd::open(evidence.main_pid) {
                Ok(pidfd) => pidfd,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(POLL_INTERVAL);
                    continue;
                }
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_unbound(&mut child, "");
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            let service = match cgroup_root.open_group(&evidence.control_group) {
                Ok(value) => value,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(POLL_INTERVAL);
                    continue;
                }
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_unbound(&mut child, "");
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            let slice = match cgroup_root.open_group(&evidence.slice_control_group) {
                Ok(value) => value,
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let service_kill = service.kill_if_populated();
                    let child_cleanup = cleanup_child(&mut child);
                    let cleanup = CleanupProof::unbound(
                        format!("service_kill={service_kill:?} {}", child_cleanup.detail),
                        child_cleanup.reaped,
                    );
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            let (service_device, service_inode) = match service.identity() {
                Ok(identity) => identity,
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            let (slice_device, slice_inode) = match slice.identity() {
                Ok(identity) => identity,
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            evidence.control_group_device = service_device;
            evidence.control_group_inode = service_inode;
            evidence.slice_control_group_device = slice_device;
            evidence.slice_control_group_inode = slice_inode;
            let shim_exited = match stage_pidfd.exited() {
                Ok(value) => value,
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            if shim_exited {
                drop(release_pipe);
                release_token.fill(0);
                let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                return Err(SystemdError::Cleanup {
                    primary: "release shim exited before identity binding".into(),
                    cleanup,
                });
            }
            match verify_process_identity(&evidence, &account, &shim, &shim_arguments, &cwd_stat) {
                Ok(()) => break (evidence, stage_pidfd, service, slice),
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(POLL_INTERVAL);
                }
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            }
        };
        if let Err(error) = verify_worker_manager_absent(&account, &spec.worker_user) {
            drop(release_pipe);
            release_token.fill(0);
            let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
            return Err(SystemdError::Cleanup {
                primary: error.to_string(),
                cleanup,
            });
        }
        let pids = match worker_processes(account.uid) {
            Ok(pids) => pids,
            Err(error) => {
                drop(release_pipe);
                release_token.fill(0);
                let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                return Err(SystemdError::Cleanup {
                    primary: error.to_string(),
                    cleanup,
                });
            }
        };
        if pids != [evidence.main_pid] {
            drop(release_pipe);
            release_token.fill(0);
            let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
            return Err(SystemdError::Cleanup {
                primary: SystemdError::WorkerProcessPresent {
                    uid: account.uid,
                    pids,
                }
                .to_string(),
                cleanup,
            });
        }
        let slice_pids = match processes_in_cgroup(&evidence.slice_control_group) {
            Ok(pids) => pids,
            Err(error) => {
                drop(release_pipe);
                release_token.fill(0);
                let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                return Err(SystemdError::Cleanup {
                    primary: error.to_string(),
                    cleanup,
                });
            }
        };
        if slice_pids != [evidence.main_pid] {
            drop(release_pipe);
            release_token.fill(0);
            let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
            return Err(SystemdError::Cleanup {
                primary: SystemdError::RecoveryRequired {
                    detail: format!(
                        "fixed slice contains processes other than the captured shim: expected=[{}] actual={slice_pids:?}",
                        evidence.main_pid
                    ),
                }
                .to_string(),
                cleanup,
            });
        }
        Ok(Self {
            child: Some(child),
            release_pipe: Some(release_pipe),
            release_token,
            evidence,
            stage_pidfd: Some(stage_pidfd),
            service: Some(service),
            slice: Some(slice),
            cwd_guard: Some(cwd_guard),
        })
    }

    pub fn evidence(&self) -> &UnitEvidence {
        &self.evidence
    }

    pub fn abort_authority(&self) -> Result<AbortAuthority, SystemdError> {
        duplicate_abort(
            &self.evidence,
            self.stage_pidfd.as_ref(),
            self.service.as_ref(),
            self.slice.as_ref(),
        )
    }

    pub fn release(mut self, owner_pidfd: RawFd) -> Result<RunningStage, SystemdError> {
        let owner_guard = duplicate_fd(owner_pidfd, "duplicate owner pidfd")?;
        let stage_pidfd = self.stage_pidfd.as_ref().expect("captured pidfd");
        if stage_pidfd.exited()? {
            return Err(self.fail_release(SystemdError::Release(
                "release shim exited before release".into(),
            )));
        }
        // This poll is intentionally the final observation before write_all.
        if poll_pidfd(owner_guard.as_raw_fd(), Duration::ZERO)? {
            return Err(self.fail_release(SystemdError::OwnerExited));
        }
        let mut release_pipe = self.release_pipe.take().expect("captured release pipe");
        if let Err(error) = release_pipe.write_all(&self.release_token) {
            drop(release_pipe);
            return Err(self.fail_release(SystemdError::Release(format!(
                "write release token: {error}"
            ))));
        }
        drop(release_pipe);
        self.release_token.fill(0);
        Ok(RunningStage {
            child: self.child.take(),
            evidence: self.evidence.clone(),
            stage_pidfd: self.stage_pidfd.take(),
            service: self.service.take(),
            slice: self.slice.take(),
            cwd_guard: self.cwd_guard.take(),
            owner_guard: Some(owner_guard),
        })
    }

    fn fail_release(&mut self, primary: SystemdError) -> SystemdError {
        self.release_pipe.take();
        self.release_token.fill(0);
        let cleanup = self.cleanup();
        SystemdError::Cleanup {
            primary: primary.to_string(),
            cleanup,
        }
    }

    fn cleanup(&mut self) -> CleanupProof {
        self.release_pipe.take();
        self.release_token.fill(0);
        let result = match (
            self.child.as_mut(),
            self.stage_pidfd.as_ref(),
            self.service.as_ref(),
            self.slice.as_ref(),
        ) {
            (Some(child), Some(pidfd), Some(service), Some(slice)) => {
                cleanup_bound(child, pidfd, service, slice)
            }
            (Some(child), _, _, _) => cleanup_unbound(child, ""),
            _ => CleanupProof::unbound("nothing remained to clean".into(), true),
        };
        self.child.take();
        self.stage_pidfd.take();
        self.service.take();
        self.slice.take();
        self.cwd_guard.take();
        result
    }
}

impl Drop for CapturedStage {
    fn drop(&mut self) {
        if self.child.is_some() {
            let cleanup = self.cleanup();
            eprintln!("CALYX_GATEBROKER_CAPTURE_DROP_CLEANUP: {cleanup}");
        }
    }
}
