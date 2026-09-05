use super::*;

pub(super) fn exec_stage(
    broker: &Broker,
    run: Arc<RunRuntime>,
    request: ExecStageRequest,
    rights: Vec<OwnedFd>,
) -> Result<Response, BrokerError> {
    ensure_run_mutable(&run)?;
    ensure_no_stage(&run)?;
    if rights.len() != 2 {
        return Err(BrokerError::invalid(
            "exec_stage lost its two validated output descriptors",
        ));
    }
    let executable = Path::new(request.argv[0].as_str());
    super::super::policy::require_absolute_regular_executable(executable)?;
    let execution_root = broker
        .execution_roots
        .get(&request.cwd_root)
        .cloned()
        .ok_or_else(|| not_found("execution root", request.cwd_root.as_str()))?;
    let resolved = execution_root.resolve(&request.cwd).map_err(|error| {
        BrokerError::new(
            StableCode::InvalidRequest,
            format!("resolve execution cwd: {error}"),
            "Use a real traversal-free directory beneath the configured execution root.",
        )
    })?;

    let objects: Vec<(ObjectId, LiveObject)> = run
        .objects
        .lock()
        .map_err(|_| poisoned("run objects"))?
        .iter()
        .map(|(id, object)| (id.clone(), object.clone()))
        .collect();
    let mut writable_paths = Vec::with_capacity(objects.len());
    for (object_id, object) in &objects {
        let root = broker
            .roots
            .get(&object.root_alias)
            .ok_or_else(|| not_found("managed root", object.root_alias.as_str()))?;
        prove_published(root, object_id, &object.published)?;
        let configured = broker
            .config
            .root(&object.root_alias)
            .ok_or_else(|| not_found("managed root", object.root_alias.as_str()))?;
        writable_paths.push(configured.raw().shared.join(object.leaf.as_str()));
    }

    let stage_id =
        ids::stage_id().map_err(|error| BrokerError::system("generate stage id", error))?;
    let prefix = &broker.config.raw().unit_prefix;
    let unit_name = format!("{prefix}-{stage_id}.service");
    let planned_unit = UnitName::new(unit_name.clone())
        .map_err(|error| BrokerError::invalid(format!("construct stage unit: {error}")))?;
    let planned_slice = UnitName::new(STAGE_SLICE_NAME)
        .map_err(|error| fatal_system("construct fixed stage slice", error))?;
    let spec = StageSpec {
        unit_name,
        worker_user: broker.worker.name.clone(),
        worker_uid: broker.worker.uid,
        execution_root: execution_root.path().to_path_buf(),
        relative_cwd: PathBuf::from(request.cwd.as_str()),
        execution_root_uid: execution_root.expected_uid(),
        execution_root_mode: execution_root.expected_mode(),
        cwd_fd: resolved.raw_fd(),
        argv: request
            .argv
            .iter()
            .map(|value| OsString::from(value.as_str()))
            .collect(),
        environment: request
            .env
            .iter()
            .map(|entry| {
                (
                    OsString::from(entry.name.as_str()),
                    OsString::from(entry.value.as_str()),
                )
            })
            .collect(),
        writable_paths,
    };
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .begin_stage(&StageIntent {
            stage_id: stage_id.clone(),
            request_id: request.request_id,
            run_id: run.id.clone(),
            label: request.label,
            unit: planned_unit.clone(),
            slice_unit: planned_slice.clone(),
            worker_user: broker.worker.name.clone(),
            worker_uid: broker.worker.uid,
        })
        .map_err(|error| BrokerError::journal("record stage intent", error))?;

    let captured = match CapturedStage::capture(&spec, rights[0].as_raw_fd(), rights[1].as_raw_fd())
    {
        Ok(value) => value,
        Err(error) => {
            if error.cleanup_proved_drained() {
                broker
                    .journal
                    .lock()
                    .map_err(|_| poisoned("journal"))?
                    .fail_stage_intent(&stage_id, 125, &error.to_string())
                    .map_err(|journal| BrokerError::journal("fail stage capture", journal))?;
                return Err(systemd_error("capture stage", error));
            }
            return Err(unproven_stage_error("capture stage", &stage_id, &error));
        }
    };
    let evidence = captured.evidence().clone();
    let unit = UnitName::new(evidence.unit_name.clone())
        .map_err(|error| fatal_system("parse captured unit", error))?;
    if unit != planned_unit {
        return Err(BrokerError::new(
            StableCode::RecoveryRequired,
            format!("captured unit changed: planned={planned_unit} actual={unit}"),
            "Leave the captured service and fixed-slice authorities intact for exact recovery.",
        )
        .fatal());
    }
    let invocation = crate::protocol::InvocationId::new(evidence.invocation_id.clone())
        .map_err(|error| fatal_system("parse captured invocation", error))?;
    let control_group = AbsolutePath::new(evidence.control_group.clone())
        .map_err(|error| fatal_system("parse captured cgroup", error))?;
    let slice_control_group = AbsolutePath::new(evidence.slice_control_group.clone())
        .map_err(|error| fatal_system("parse captured slice cgroup", error))?;
    if evidence.worker_user != broker.worker.name || evidence.worker_uid != broker.worker.uid {
        return Err(lifecycle_mismatch(
            &run.id,
            &format!(
                "captured worker identity changed: expected={}:{} actual={}:{}",
                broker.worker.name, broker.worker.uid, evidence.worker_user, evidence.worker_uid
            ),
        ));
    }
    let authority = Arc::new(
        captured
            .abort_authority()
            .map_err(|error| systemd_error("duplicate captured authority", error))?,
    );
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .mark_stage_running(&StageRunningEvidence {
            stage_id: &stage_id,
            expected_unit: &planned_unit,
            expected_slice_unit: &planned_slice,
            invocation_id: &invocation,
            control_group: &control_group,
            slice_control_group: &slice_control_group,
            control_group_identity: RecordedCgroupIdentity {
                device: evidence.control_group_device,
                inode: evidence.control_group_inode,
            },
            slice_control_group_identity: RecordedCgroupIdentity {
                device: evidence.slice_control_group_device,
                inode: evidence.slice_control_group_inode,
            },
            main_pid: evidence.main_pid,
        })
        .map_err(|error| BrokerError::journal("publish running stage", error))?;
    {
        let mut slot = run.stage.0.lock().map_err(|_| poisoned("run stage"))?;
        if slot.is_some() {
            return Err(BrokerError::new(
                StableCode::RecoveryRequired,
                format!("run {} acquired two live stage authorities", run.id),
                "Stop the broker and inspect both recorded stage/cgroup identities.",
            )
            .fatal());
        }
        *slot = Some(LiveStage {
            id: stage_id.clone(),
            authority,
        });
    }

    let mut captured = Some(captured);
    let release = {
        let slot = run.stage.0.lock().map_err(|_| poisoned("run stage"))?;
        let live = slot.as_ref().ok_or_else(|| {
            BrokerError::new(
                StableCode::RecoveryRequired,
                format!("live authority for captured stage {stage_id} disappeared"),
                "Inspect abort/launch serialization before restart.",
            )
            .fatal()
        })?;
        if live.id != stage_id {
            return Err(BrokerError::new(
                StableCode::RecoveryRequired,
                format!(
                    "captured stage identity changed: expected={stage_id} actual={}",
                    live.id
                ),
                "Inspect abort/launch serialization before restart.",
            )
            .fatal());
        }
        if run.lifecycle.abort_signal.load(Ordering::Acquire) {
            None
        } else {
            // The abort transition is serialized by this same mutex. After
            // release starts, an abort can only acquire this exact authority
            // and kill the now-running stage.
            Some(
                captured
                    .take()
                    .expect("captured stage is consumed exactly once")
                    .release(run.owner.raw_fd()),
            )
        }
    };
    let Some(release) = release else {
        drop(captured);
        record_stage_terminal(broker, &run, &stage_id, 125)?;
        return Err(BrokerError::new(
            StableCode::OwnerDied,
            format!(
                "run {} began aborting before stage {stage_id} release",
                run.id
            ),
            "The blocked shim was drained without executing the payload.",
        ));
    };
    let running = match release {
        Ok(value) => value,
        Err(error) => {
            if error.cleanup_proved_drained() {
                record_stage_terminal(broker, &run, &stage_id, 125)?;
            } else {
                return Err(unproven_stage_error("release stage", &stage_id, &error));
            }
            match run.owner.has_exited() {
                Ok(true) => {
                    return Err(BrokerError::new(
                        StableCode::OwnerDied,
                        format!("owner exited before stage {} release: {error}", stage_id),
                        "Automatic abort will clean every registered object after exact stage drain.",
                    ));
                }
                Ok(false) => {}
                Err(owner_error) => {
                    request_abort(&run)?;
                    return Err(BrokerError::new(
                        StableCode::RecoveryRequired,
                        format!(
                            "owner liveness is unproven after failed stage release for run {}: {owner_error}",
                            run.id
                        ),
                        "Leave the recorded authority intact and drain it during broker recovery.",
                    )
                    .fatal());
                }
            }
            return Err(systemd_error("release stage", error));
        }
    };
    let result = match running.wait() {
        Ok(value) => value,
        Err(error) => {
            if error.cleanup_proved_drained() {
                record_stage_terminal(broker, &run, &stage_id, 125)?;
                return Err(systemd_error("wait for stage drain", error));
            }
            return Err(unproven_stage_error(
                "wait for stage drain",
                &stage_id,
                &error,
            ));
        }
    };
    record_stage_terminal(broker, &run, &stage_id, result.exit_status)?;
    Ok(Response::StageFinished {
        stage_id,
        unit,
        invocation_id: invocation,
        control_group,
        exit_status: result.exit_status,
    })
}

pub(super) fn record_stage_terminal(
    broker: &Broker,
    run: &RunRuntime,
    stage_id: &crate::protocol::StageId,
    exit_status: i32,
) -> Result<(), BrokerError> {
    let mut slot = run.stage.0.lock().map_err(|_| poisoned("run stage"))?;
    let live = slot.as_ref().ok_or_else(|| {
        BrokerError::new(
            StableCode::RecoveryRequired,
            format!("live authority for stage {stage_id} disappeared"),
            "Inspect concurrent abort and stage journal ordering before restart.",
        )
        .fatal()
    })?;
    if live.id != *stage_id {
        return Err(BrokerError::new(
            StableCode::RecoveryRequired,
            format!(
                "live stage identity changed: expected={stage_id} actual={}",
                live.id
            ),
            "Inspect concurrent stage state before restart.",
        )
        .fatal());
    }
    live.authority
        .kill_and_prove_empty()
        .map_err(|error| unproven_stage_error("prove stage drained", stage_id, &error))?;
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .finish_stage(stage_id, exit_status)
        .map_err(|error| BrokerError::journal("finish stage", error))?;
    let removed = slot.take().ok_or_else(|| {
        BrokerError::new(
            StableCode::RecoveryRequired,
            format!("live authority for stage {stage_id} disappeared"),
            "Inspect concurrent abort and stage journal ordering before restart.",
        )
        .fatal()
    })?;
    if removed.id != *stage_id {
        return Err(lifecycle_mismatch(
            &run.id,
            "stage authority changed during terminal removal",
        ));
    }
    run.stage.1.notify_all();
    Ok(())
}

pub(super) fn unproven_stage_error(
    operation: &str,
    stage_id: &crate::protocol::StageId,
    error: &SystemdError,
) -> BrokerError {
    BrokerError::new(
        StableCode::RecoveryRequired,
        format!("{operation} for {stage_id} did not prove both cgroups drained: {error}"),
        "Preserve the stage row and held service/fixed-slice identities for broker restart recovery.",
    )
    .fatal()
}

pub(super) fn systemd_error(operation: &str, error: SystemdError) -> BrokerError {
    match error {
        SystemdError::InvalidSpec(detail) => BrokerError::new(
            StableCode::InvalidRequest,
            format!("{operation}: {detail}"),
            "Correct the stage command/environment/path policy and retry with a new request id.",
        ),
        SystemdError::OwnerExited => BrokerError::new(
            StableCode::OwnerDied,
            format!("{operation}: owner exited"),
            "Wait for automatic exact stage/object cleanup.",
        ),
        other => BrokerError::new(
            StableCode::StageFailed,
            format!("{operation}: {other}"),
            "Inspect the recorded unit, invocation, main PID, service/slice cgroup, and cleanup evidence.",
        )
        .fatal(),
    }
}
