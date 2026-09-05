use super::*;

pub(super) fn begin_run(
    broker: &Arc<Broker>,
    peer: ProcessIdentity,
    request: crate::protocol::BeginRunRequest,
) -> Result<Response, BrokerError> {
    if request.owner_pid != peer.pid {
        // Descendant gatectl processes are accepted at the socket boundary,
        // but the durable owner itself must still have the authenticated UID.
        let owner = crate::accounts::process_identity(request.owner_pid)
            .map_err(|error| BrokerError::permission(error.to_string()))?;
        if owner.uid != peer.uid || owner.starttime != request.owner_starttime {
            return Err(BrokerError::permission(
                "declared owner identity does not match the authenticated peer ancestry",
            ));
        }
    }
    let owner = Arc::new(
        OwnerLease::open(request.owner_pid, peer.uid, request.owner_starttime)
            .map_err(|error| BrokerError::permission(error.to_string()))?,
    );
    let run_id = ids::run_id().map_err(|error| BrokerError::system("generate run id", error))?;
    let run_token =
        ids::run_token().map_err(|error| BrokerError::system("generate run token", error))?;
    let runtime = Arc::new(RunRuntime {
        id: run_id.clone(),
        token: run_token.clone(),
        owner_uid: peer.uid,
        owner: Arc::clone(&owner),
        lifecycle: RunLifecycle {
            sequence: std::sync::Mutex::new(()),
            state: std::sync::Mutex::new(RunLifecycleState::Active),
            changed: std::sync::Condvar::new(),
            abort_signal: std::sync::atomic::AtomicBool::new(false),
        },
        objects: std::sync::Mutex::new(BTreeMap::new()),
        stage: (std::sync::Mutex::new(None), std::sync::Condvar::new()),
    });
    {
        let mut runs = broker.runs.lock().map_err(|_| poisoned("runs"))?;
        if !runs.is_empty() {
            return Err(BrokerError::new(
                StableCode::Busy,
                "the broker already has an active run",
                "Finish or abort the current run before beginning another one.",
            ));
        }
        broker
            .journal
            .lock()
            .map_err(|_| poisoned("journal"))?
            .begin_run(&RunIntent {
                run_id: run_id.clone(),
                request_id: request.request_id,
                run_token: run_token.clone(),
                profile: request.profile,
                owner_uid: peer.uid,
                owner_pid: owner.pid(),
                owner_starttime: owner.starttime(),
            })
            .map_err(|error| BrokerError::journal("begin run", error))?;
        runs.insert(run_id.clone(), Arc::clone(&runtime));
    }
    spawn_owner_watcher(Arc::downgrade(broker), runtime);
    Ok(Response::RunBegun { run_id, run_token })
}

pub(super) fn spawn_owner_watcher(broker: Weak<Broker>, run: Arc<RunRuntime>) {
    std::thread::spawn(move || {
        let result = run.owner.wait_for_exit();
        let Some(broker) = broker.upgrade() else {
            return;
        };
        if let Err(error) = result {
            let cleanup = abort_run_internal(&broker, Arc::clone(&run), "OWNER_LIVENESS_UNPROVEN");
            let failure = BrokerError::new(
                StableCode::RecoveryRequired,
                format!(
                    "owner pidfd monitoring failed for run {}: {error}; abort_cleanup={cleanup:?}",
                    run.id
                ),
                "Inspect pidfd poll events and drain the recorded stage and objects before restart.",
            )
            .fatal();
            failure.log("owner_monitor_failed");
            broker.fatal.store(true, Ordering::Release);
            std::process::exit(125);
        }
        match abort_run_internal(&broker, Arc::clone(&run), "OWNER_DIED") {
            Ok(AbortOutcome::Aborted) => {
                if let Err(error) = finalize_terminal_runtime(&broker, &run, false) {
                    error.log("owner_death_terminal_publish_failed");
                    broker.fatal.store(true, Ordering::Release);
                    std::process::exit(125);
                }
            }
            Ok(AbortOutcome::AlreadyAborted | AbortOutcome::Terminal(_)) => {}
            Err(error) => {
                error.log("owner_death_cleanup_failed");
                broker.fatal.store(true, Ordering::Release);
                std::process::exit(125);
            }
        }
    });
}

pub(super) fn authorize_run(
    broker: &Broker,
    peer: PeerCredentials,
    run_id: &RunId,
    token: &RunToken,
) -> Result<Option<Arc<RunRuntime>>, BrokerError> {
    let runtime = broker
        .runs
        .lock()
        .map_err(|_| poisoned("runs"))?
        .get(run_id)
        .cloned();
    if let Some(run) = runtime {
        if peer.uid != run.owner_uid || !constant_time_eq(token.as_str(), run.token.as_str()) {
            return Err(BrokerError::permission(
                "run capability does not match the peer",
            ));
        }
        return Ok(Some(run));
    }

    let record = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .get_run(run_id)
        .map_err(|error| BrokerError::journal("read run authorization", error))?
        .ok_or_else(|| not_found("run", run_id.as_str()))?;
    if peer.uid != record.intent.owner_uid
        || !constant_time_eq(token.as_str(), record.intent.run_token.as_str())
    {
        return Err(BrokerError::permission(
            "run capability does not match the peer",
        ));
    }
    Ok(None)
}

pub(super) fn runtime_is_current(
    broker: &Broker,
    run: &Arc<RunRuntime>,
) -> Result<bool, BrokerError> {
    let runs = broker.runs.lock().map_err(|_| poisoned("runs"))?;
    match runs.get(&run.id) {
        Some(current) if Arc::ptr_eq(current, run) => Ok(true),
        Some(_) => Err(lifecycle_mismatch(
            &run.id,
            "run id maps to a different in-memory authority",
        )),
        None => Ok(false),
    }
}

pub(super) fn revalidate_run_authority(
    run: &RunRuntime,
    peer: PeerCredentials,
    token: &RunToken,
    require_live_owner: bool,
) -> Result<(), BrokerError> {
    if peer.uid != run.owner_uid || !constant_time_eq(token.as_str(), run.token.as_str()) {
        return Err(BrokerError::permission(
            "run capability does not match the peer",
        ));
    }
    require_owner_ancestry(
        peer.pid,
        run.owner_uid,
        run.owner.pid(),
        run.owner.starttime(),
    )
    .map_err(|error| BrokerError::permission(error.to_string()))?;
    if require_live_owner && poll_owner(run, "revalidate run authority")? {
        return Err(BrokerError::new(
            StableCode::OwnerDied,
            format!("run {} owner has exited", run.id),
            "Wait for automatic abort cleanup and inspect the durable run state.",
        ));
    }
    Ok(())
}

pub(super) fn poll_owner(run: &RunRuntime, operation: &str) -> Result<bool, BrokerError> {
    match run.owner.has_exited() {
        Ok(exited) => Ok(exited),
        Err(error) => {
            request_abort(run)?;
            Err(BrokerError::new(
                StableCode::RecoveryRequired,
                format!(
                    "owner liveness is unproven during {operation} for run {}: {error}",
                    run.id
                ),
                "Drain the exact recorded stage and registered objects before restarting the broker.",
            )
            .fatal())
        }
    }
}

pub(super) fn lifecycle_mismatch(run_id: &RunId, detail: &str) -> BrokerError {
    BrokerError::new(
        StableCode::RecoveryRequired,
        format!("run {run_id} lifecycle source-of-truth mismatch: {detail}"),
        "Stop the broker and inspect the runtime authority, journal row, and terminal operation response.",
    )
    .fatal()
}
