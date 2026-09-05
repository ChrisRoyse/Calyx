use super::*;

pub(super) fn health(broker: &Broker, request: HealthRequest) -> Result<Reply, BrokerError> {
    let runtimes: Vec<Arc<RunRuntime>> = broker
        .runs
        .lock()
        .map_err(|_| poisoned("runs"))?
        .values()
        .cloned()
        .collect();
    let mut authorities = Vec::new();
    for run in &runtimes {
        if let Some(stage) = run
            .stage
            .0
            .lock()
            .map_err(|_| poisoned("run stage"))?
            .as_ref()
        {
            authorities.push(Arc::clone(&stage.authority));
        }
    }
    match authorities.as_slice() {
        [] => crate::systemd::verify_worker_idle(&broker.worker.name, broker.worker.uid)
            .map_err(|error| systemd_error("verify idle worker boundary health", error))?,
        [authority] => authority
            .verify_contained(&broker.worker.name, broker.worker.uid)
            .map_err(|error| systemd_error("verify live stage worker containment", error))?,
        many => {
            return Err(BrokerError::new(
                StableCode::RecoveryRequired,
                format!(
                    "worker boundary health found {} simultaneous live stage authorities",
                    many.len()
                ),
                "Stop the broker and inspect every held service/fixed-slice cgroup authority.",
            )
            .fatal());
        }
    }
    {
        let journal = broker.journal.lock().map_err(|_| poisoned("journal"))?;
        journal
            .verify_durability_settings()
            .map_err(|error| BrokerError::journal("health durability check", error))?;
        journal
            .integrity_check()
            .map_err(|error| BrokerError::journal("health integrity check", error))?;
    }
    let mut managed_roots = BTreeMap::new();
    for (alias, root) in &broker.roots {
        let configured = broker
            .config
            .root(alias)
            .ok_or_else(|| not_found("managed root", alias.as_str()))?;
        managed_roots.insert(
            alias.clone(),
            ManagedRootHealth {
                shared: absolute_path(&configured.raw().shared)?,
                private: absolute_path(&configured.raw().private)?,
                root_identity: diagnostic(root.root_identity()),
            },
        );
    }
    let unit = UnitName::new("calyx-gatebrokerd.service")
        .map_err(|error| BrokerError::system("construct broker unit", error))?;
    let account = ContextValue::new(broker.worker.name.clone())
        .map_err(|error| BrokerError::system("construct worker health", error))?;
    let active_runs = broker.runs.lock().map_err(|_| poisoned("runs"))?.len();
    Ok(Reply::success(
        request.request_id,
        Response::Health {
            healthy: !broker.fatal.load(Ordering::Acquire),
            broker: BrokerHealth {
                pid: std::process::id(),
                uid: unsafe { libc::geteuid() },
                unit,
                cgroup: broker.broker_cgroup.clone(),
            },
            worker: WorkerHealth {
                uid: broker.worker.uid,
                gid: broker.worker.gid,
                account,
                user_manager_absent: true,
            },
            storage: StorageHealth {
                database: absolute_path(
                    broker
                        .journal
                        .lock()
                        .map_err(|_| poisoned("journal"))?
                        .path(),
                )?,
                managed_roots,
            },
            limits: LimitHealth {
                frame_bytes: MAX_FRAME_BYTES,
                argv_entries: MAX_ARGV_ITEMS,
                environment_entries: MAX_ENV_ITEMS,
                object_name_bytes: 240,
                active_runs,
            },
        },
    ))
}

pub(super) fn inspect(
    broker: &Broker,
    peer: PeerCredentials,
    request: InspectRequest,
) -> Result<Reply, BrokerError> {
    let run_record = match (&request.run_id, &request.run_token) {
        (Some(run_id), Some(token)) => {
            let record = broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .get_run(run_id)
                .map_err(|error| BrokerError::journal("inspect run", error))?
                .ok_or_else(|| not_found("run", run_id.as_str()))?;
            if peer.uid != record.intent.owner_uid
                || !constant_time_eq(token.as_str(), record.intent.run_token.as_str())
            {
                return Err(BrokerError::permission(
                    "inspect capability does not match the run",
                ));
            }
            Some(record)
        }
        (None, None) => {
            if peer.uid != 0 {
                return Err(BrokerError::permission(
                    "unscoped inspection requires the root broker identity",
                ));
            }
            broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .list_active_runs()
                .map_err(|error| BrokerError::journal("inspect active runs", error))?
                .into_iter()
                .next()
        }
        _ => {
            return Err(BrokerError::invalid(
                "inspect requires both run_id and run_token",
            ));
        }
    };
    let Some(run_record) = run_record else {
        return Ok(Reply::success(
            request.request_id,
            Response::Inspection {
                run: None,
                objects: Vec::new(),
                stages: Vec::new(),
                truncated: false,
            },
        ));
    };
    let mut objects = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .list_objects_for_run(&run_record.intent.run_id)
        .map_err(|error| BrokerError::journal("inspect objects", error))?;
    let mut stages = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .list_stages_for_run(&run_record.intent.run_id)
        .map_err(|error| BrokerError::journal("inspect stages", error))?;
    let truncated = objects.len() > INSPECTION_LIMIT || stages.len() > INSPECTION_LIMIT;
    objects.truncate(INSPECTION_LIMIT);
    stages.truncate(INSPECTION_LIMIT);
    let object_views = objects
        .into_iter()
        .map(|record| {
            let configured = broker
                .config
                .root(&record.intent.root_alias)
                .ok_or_else(|| not_found("managed root", record.intent.root_alias.as_str()))?;
            Ok(ObjectInspection {
                id: record.intent.object_id,
                run_id: record.intent.run_id,
                role: record.intent.role,
                root_alias: record.intent.root_alias,
                leaf: record.intent.leaf.clone(),
                path: absolute_path(&configured.raw().shared.join(record.intent.leaf.as_str()))?,
                state: context(transaction_state_name(record.state))?,
                identity: record.identity.as_ref().map(diagnostic),
                quarantine_name: optional_context(record.quarantine_name)?,
                error_code: optional_context(record.error_code)?,
                detail: optional_context(record.detail)?,
                created_ms: record.created_ms,
                updated_ms: record.updated_ms,
            })
        })
        .collect::<Result<Vec<_>, BrokerError>>()?;
    let stage_views = stages
        .into_iter()
        .map(|record| {
            Ok(StageInspection {
                id: record.intent.stage_id,
                run_id: record.intent.run_id,
                label: record.intent.label,
                state: context(stage_state_name(record.state))?,
                unit: Some(record.intent.unit),
                invocation_id: record.invocation_id,
                control_group: record.control_group,
                main_pid: record.main_pid,
                exit_status: record.exit_status,
                created_ms: record.created_ms,
                updated_ms: record.updated_ms,
            })
        })
        .collect::<Result<Vec<_>, BrokerError>>()?;
    let run_view = RunInspection {
        id: run_record.intent.run_id,
        state: context(run_state_name(run_record.state))?,
        profile: run_record.intent.profile,
        owner_uid: run_record.intent.owner_uid,
        owner_pid: run_record.intent.owner_pid,
        owner_starttime: run_record.intent.owner_starttime,
        terminal_reason: optional_context(run_record.detail)?,
        created_ms: run_record.created_ms,
        updated_ms: run_record.updated_ms,
    };
    Ok(Reply::success(
        request.request_id,
        Response::Inspection {
            run: Some(run_view),
            objects: object_views,
            stages: stage_views,
            truncated,
        },
    ))
}
