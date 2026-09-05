use super::account::{
    cgroup_contains, proc_control_group, proc_uids, processes_in_cgroup,
    verify_worker_account_identity, verify_worker_manager_absent, worker_process_locations,
};
use super::capture::{cleanup_bound, cleanup_unbound};
use super::validation::io_error;
use super::*;

pub(super) fn duplicate_abort(
    evidence: &UnitEvidence,
    pidfd: Option<&ProcessFd>,
    service: Option<&CgroupAuthority>,
    slice: Option<&CgroupAuthority>,
) -> Result<AbortAuthority, SystemdError> {
    Ok(AbortAuthority {
        evidence: evidence.clone(),
        stage_pidfd: pidfd
            .ok_or_else(|| SystemdError::Release("stage pidfd was consumed".into()))?
            .duplicate()?,
        service: service
            .ok_or_else(|| SystemdError::Release("service cgroup was consumed".into()))?
            .duplicate()?,
        slice: slice
            .ok_or_else(|| SystemdError::Release("slice cgroup was consumed".into()))?
            .duplicate()?,
    })
}

impl RunningStage {
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

    pub fn wait(mut self) -> Result<StageResult, SystemdError> {
        let status = self
            .child
            .as_mut()
            .expect("running systemd-run child")
            .wait()
            .map_err(|source| io_error("wait for /usr/bin/systemd-run", source))?;
        let pidfd = self.stage_pidfd.as_ref().expect("running stage pidfd");
        let service = self.service.as_ref().expect("running service cgroup");
        let slice = self.slice.as_ref().expect("running slice cgroup");
        let proof = (|| {
            if !pidfd.wait_timeout(DRAIN_TIMEOUT)? {
                return Err(SystemdError::ProcessIdentity {
                    pid: pidfd.pid,
                    detail: "pidfd did not report exit after systemd-run completed".into(),
                });
            }
            service.prove_empty(DRAIN_TIMEOUT)?;
            slice.prove_empty(DRAIN_TIMEOUT)?;
            Ok(())
        })();
        if let Err(error) = proof {
            let cleanup = cleanup_bound(
                self.child.as_mut().expect("running child"),
                pidfd,
                service,
                slice,
            );
            self.child.take();
            return Err(SystemdError::Cleanup {
                primary: error.to_string(),
                cleanup,
            });
        }
        let systemd_run_status = status
            .code()
            .unwrap_or_else(|| status.signal().map(|signal| 128 + signal).unwrap_or(125));
        let (main_code, main_status) = match (status.code(), status.signal()) {
            (Some(code), _) => ("exited".into(), code),
            (_, Some(signal)) => ("killed".into(), signal),
            _ => ("unknown".into(), 125),
        };
        self.child.take();
        self.stage_pidfd.take();
        self.service.take();
        self.slice.take();
        self.cwd_guard.take();
        self.owner_guard.take();
        Ok(StageResult {
            evidence: self.evidence.clone(),
            exit_status: systemd_run_status,
            main_code,
            main_status,
            systemd_run_status,
        })
    }

    fn cleanup(&mut self) -> CleanupProof {
        match (
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
        }
    }
}

impl Drop for RunningStage {
    fn drop(&mut self) {
        if self.child.is_some() {
            let cleanup = self.cleanup();
            eprintln!("CALYX_GATEBROKER_RUNNING_DROP_CLEANUP: {cleanup}");
        }
    }
}

impl AbortAuthority {
    pub fn evidence(&self) -> &UnitEvidence {
        &self.evidence
    }

    pub fn kill_and_prove_empty(&self) -> Result<(), SystemdError> {
        self.service.kill_if_populated()?;
        self.slice.kill_if_populated()?;
        if !self.stage_pidfd.wait_timeout(DRAIN_TIMEOUT)? {
            self.stage_pidfd.kill()?;
            if !self.stage_pidfd.wait_timeout(DRAIN_TIMEOUT)? {
                return Err(SystemdError::ProcessIdentity {
                    pid: self.stage_pidfd.pid,
                    detail: "pidfd remained live after cgroup.kill".into(),
                });
            }
        }
        self.service.prove_empty(DRAIN_TIMEOUT)?;
        self.slice.prove_empty(DRAIN_TIMEOUT)
    }

    pub fn verify_contained(&self, worker_user: &str, worker_uid: u32) -> Result<(), SystemdError> {
        if self.evidence.worker_user != worker_user || self.evidence.worker_uid != worker_uid {
            return Err(SystemdError::WorkerPolicy {
                user: worker_user.into(),
                detail: format!(
                    "live authority identity differs from requested health identity: evidence={}:{} requested={worker_user}:{worker_uid}",
                    self.evidence.worker_user, self.evidence.worker_uid
                ),
            });
        }
        let account = verify_worker_account_identity(worker_user, worker_uid)?;
        verify_worker_manager_absent(&account, worker_user)?;

        for (authority, expected_path, expected_device, expected_inode) in [
            (
                &self.service,
                self.evidence.control_group.as_str(),
                self.evidence.control_group_device,
                self.evidence.control_group_inode,
            ),
            (
                &self.slice,
                self.evidence.slice_control_group.as_str(),
                self.evidence.slice_control_group_device,
                self.evidence.slice_control_group_inode,
            ),
        ] {
            let (device, inode) = authority.identity()?;
            if authority.control_group != expected_path
                || device != expected_device
                || inode != expected_inode
            {
                return Err(SystemdError::Cgroup {
                    control: authority.control_group.clone(),
                    detail: format!(
                        "held authority identity changed: expected_path={expected_path} expected_dev={expected_device} expected_ino={expected_inode} actual_dev={device} actual_ino={inode}"
                    ),
                });
            }
        }
        if self.slice.control_group != STAGE_SLICE_CONTROL_GROUP
            || !cgroup_contains(&self.slice.control_group, &self.service.control_group)
        {
            return Err(SystemdError::Cgroup {
                control: self.slice.control_group.clone(),
                detail: format!(
                    "held service is outside fixed slice: service={}",
                    self.service.control_group
                ),
            });
        }

        let workers = worker_process_locations(worker_uid)?;
        let outside: Vec<_> = workers
            .iter()
            .filter(|(_, control_group)| {
                !cgroup_contains(&self.service.control_group, control_group)
            })
            .cloned()
            .collect();
        if !outside.is_empty() {
            return Err(SystemdError::WorkerProcessPresent {
                uid: worker_uid,
                pids: outside.iter().map(|(pid, _)| *pid).collect(),
            });
        }
        for pid in processes_in_cgroup(&self.slice.control_group)? {
            let Some(control_group) = proc_control_group(pid)? else {
                continue;
            };
            if !cgroup_contains(&self.slice.control_group, &control_group) {
                continue;
            }
            if !cgroup_contains(&self.service.control_group, &control_group) {
                return Err(SystemdError::Cgroup {
                    control: self.slice.control_group.clone(),
                    detail: format!(
                        "pid {pid} occupies an unexpected fixed-slice cgroup {control_group}"
                    ),
                });
            }
            if let Some(uids) = proc_uids(pid)?
                && uids != [worker_uid; 4]
            {
                return Err(SystemdError::ProcessIdentity {
                    pid,
                    detail: format!(
                        "fixed-slice process is not the dedicated worker uid {worker_uid}: {uids:?}"
                    ),
                });
            }
        }
        Ok(())
    }
}
