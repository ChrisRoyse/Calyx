use super::*;

pub(super) fn finish_run(
    broker: &Broker,
    run: Arc<RunRuntime>,
    run_id: RunId,
    status: RunStatus,
) -> Result<Response, BrokerError> {
    ensure_run_mutable(&run)?;
    ensure_no_stage(&run)?;
    let ids: Vec<ObjectId> = run
        .objects
        .lock()
        .map_err(|_| poisoned("run objects"))?
        .keys()
        .cloned()
        .collect();
    for object_id in ids {
        delete_object(broker, &run, &object_id, "run finish cleanup")?;
    }
    if poll_owner(&run, "verify owner before finish")? {
        request_abort(&run)?;
        return Err(BrokerError::new(
            StableCode::OwnerDied,
            format!("run {run_id} owner exited during finish"),
            "The owner-death watcher will record the terminal aborted state.",
        ));
    }
    let intended = RunState::from(status);
    let mut lifecycle = run
        .lifecycle
        .state
        .lock()
        .map_err(|_| poisoned("run lifecycle state"))?;
    match *lifecycle {
        RunLifecycleState::Active => {}
        RunLifecycleState::AbortRequested { .. } => {
            return Err(BrokerError::new(
                StableCode::OwnerDied,
                format!("run {run_id} was claimed for abort during finish"),
                "Wait for exact process drain, object cleanup, and the durable aborted result.",
            ));
        }
        RunLifecycleState::Terminal { state: actual, .. } => {
            return Err(terminal_finish_conflict(&run_id, actual));
        }
    }
    {
        let mut journal = broker.journal.lock().map_err(|_| poisoned("journal"))?;
        journal
            .finish_run(&run_id, intended, Some("controller finish"))
            .map_err(finish_run_transition_error)?;
        let actual = journal
            .get_run(&run_id)
            .map_err(|error| BrokerError::journal("read back finished run", error))?
            .ok_or_else(|| lifecycle_mismatch(&run_id, "finished journal row disappeared"))?
            .state;
        if actual != intended {
            return Err(lifecycle_mismatch(
                &run_id,
                &format!("finish wrote {intended:?} but read back {actual:?}"),
            ));
        }
    }
    *lifecycle = RunLifecycleState::Terminal {
        state: intended,
        pending_operations: 0,
    };
    Ok(Response::RunFinished { run_id, status })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AbortOutcome {
    Aborted,
    AlreadyAborted,
    Terminal(RunState),
}

pub(super) fn abort_run_internal(
    broker: &Broker,
    run: Arc<RunRuntime>,
    reason: &str,
) -> Result<AbortOutcome, BrokerError> {
    let newly_requested = request_abort(&run)?;
    if newly_requested {
        let stage = run.stage.0.lock().map_err(|_| poisoned("run stage"))?;
        if let Some(stage) = stage.as_ref() {
            stage
                .authority
                .kill_and_prove_empty()
                .map_err(|error| fatal_system("drain active stage", error))?;
        }
    }
    let deadline = Instant::now() + ABORT_DRAIN_TIMEOUT;
    let mut stage = run.stage.0.lock().map_err(|_| poisoned("run stage"))?;
    while stage.is_some() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(BrokerError::new(
                StableCode::RecoveryRequired,
                format!(
                    "stage for run {} did not clear after exact cgroup drain",
                    run.id
                ),
                "Inspect the held cgroup and stage journal before touching registered objects.",
            )
            .fatal());
        }
        let waited = run
            .stage
            .1
            .wait_timeout(stage, remaining)
            .map_err(|_| poisoned("run stage"))?;
        stage = waited.0;
    }
    drop(stage);
    let _sequence = run
        .lifecycle
        .sequence
        .lock()
        .map_err(|_| poisoned("run lifecycle sequence"))?;
    {
        let state = run
            .lifecycle
            .state
            .lock()
            .map_err(|_| poisoned("run lifecycle state"))?;
        if let RunLifecycleState::Terminal { state: actual, .. } = *state {
            drop(state);
            verify_terminal_domain(broker, &run, actual)?;
            return Ok(if actual == RunState::Aborted {
                AbortOutcome::AlreadyAborted
            } else {
                AbortOutcome::Terminal(actual)
            });
        }
        if !matches!(*state, RunLifecycleState::AbortRequested { .. }) {
            return Err(lifecycle_mismatch(
                &run.id,
                "abort coordinator acquired an active lifecycle without an abort claim",
            ));
        }
    }
    let ids: Vec<ObjectId> = run
        .objects
        .lock()
        .map_err(|_| poisoned("run objects"))?
        .keys()
        .cloned()
        .collect();
    for object_id in ids {
        delete_object(broker, &run, &object_id, "run abort cleanup")?;
    }
    let mut lifecycle = run
        .lifecycle
        .state
        .lock()
        .map_err(|_| poisoned("run lifecycle state"))?;
    let pending_operations = match *lifecycle {
        RunLifecycleState::AbortRequested { pending_operations } => pending_operations,
        _ => {
            return Err(lifecycle_mismatch(
                &run.id,
                "abort claim changed before durable terminalization",
            ));
        }
    };
    {
        let mut journal = broker.journal.lock().map_err(|_| poisoned("journal"))?;
        let before = journal
            .get_run(&run.id)
            .map_err(|error| BrokerError::journal("read aborting run", error))?
            .ok_or_else(|| lifecycle_mismatch(&run.id, "aborting journal row disappeared"))?;
        if before.state != RunState::Active {
            return Err(lifecycle_mismatch(
                &run.id,
                &format!(
                    "abort claim is live but journal state is {:?}",
                    before.state
                ),
            ));
        }
        journal
            .finish_run(&run.id, RunState::Aborted, Some(reason))
            .map_err(|error| BrokerError::journal("abort run", error))?;
        let after = journal
            .get_run(&run.id)
            .map_err(|error| BrokerError::journal("read back aborted run", error))?
            .ok_or_else(|| lifecycle_mismatch(&run.id, "aborted journal row disappeared"))?;
        if after.state != RunState::Aborted {
            return Err(lifecycle_mismatch(
                &run.id,
                &format!("abort wrote Aborted but read back {:?}", after.state),
            ));
        }
    }
    *lifecycle = RunLifecycleState::Terminal {
        state: RunState::Aborted,
        pending_operations,
    };
    prove_runtime_drained(&run)?;
    Ok(AbortOutcome::Aborted)
}

pub(super) fn request_abort(run: &RunRuntime) -> Result<bool, BrokerError> {
    let mut state = run
        .lifecycle
        .state
        .lock()
        .map_err(|_| poisoned("run lifecycle state"))?;
    match *state {
        RunLifecycleState::Active => {
            *state = RunLifecycleState::AbortRequested {
                pending_operations: 0,
            };
            run.lifecycle.abort_signal.store(true, Ordering::Release);
            run.lifecycle.changed.notify_all();
            Ok(true)
        }
        RunLifecycleState::AbortRequested { .. } | RunLifecycleState::Terminal { .. } => Ok(false),
    }
}

pub(super) fn remove_runtime(broker: &Broker, run: &Arc<RunRuntime>) -> Result<(), BrokerError> {
    {
        let mut runs = broker.runs.lock().map_err(|_| poisoned("runs"))?;
        match runs.get(&run.id) {
            Some(current) if Arc::ptr_eq(current, run) => {}
            Some(_) => {
                return Err(lifecycle_mismatch(
                    &run.id,
                    "terminal removal found a different runtime authority",
                ));
            }
            None => {
                return Err(lifecycle_mismatch(
                    &run.id,
                    "terminal removal found no runtime authority",
                ));
            }
        }
        let removed = runs.remove(&run.id).ok_or_else(|| {
            lifecycle_mismatch(&run.id, "runtime disappeared during exact removal")
        })?;
        if !Arc::ptr_eq(&removed, run) {
            return Err(lifecycle_mismatch(
                &run.id,
                "removed runtime authority did not match the terminal claimant",
            ));
        }
    }
    if broker
        .runs
        .lock()
        .map_err(|_| poisoned("runs"))?
        .contains_key(&run.id)
    {
        return Err(lifecycle_mismatch(
            &run.id,
            "runtime remained present after exact removal readback",
        ));
    }
    Ok(())
}

pub(super) fn finalize_terminal_runtime(
    broker: &Broker,
    run: &Arc<RunRuntime>,
    completed_abort_operation: bool,
) -> Result<(), BrokerError> {
    let mut lifecycle = run
        .lifecycle
        .state
        .lock()
        .map_err(|_| poisoned("run lifecycle state"))?;
    let (terminal, pending_operations) = match &mut *lifecycle {
        RunLifecycleState::Terminal {
            state,
            pending_operations,
        } => (*state, pending_operations),
        RunLifecycleState::Active | RunLifecycleState::AbortRequested { .. } => return Ok(()),
    };
    if completed_abort_operation {
        *pending_operations = pending_operations.checked_sub(1).ok_or_else(|| {
            lifecycle_mismatch(
                &run.id,
                "completed abort operation had no registered pending claim",
            )
        })?;
        run.lifecycle.changed.notify_all();
    }
    if *pending_operations != 0 {
        return Ok(());
    }
    verify_terminal_domain(broker, run, terminal)?;
    remove_runtime(broker, run)?;
    run.lifecycle.changed.notify_all();
    Ok(())
}

pub(super) fn verify_terminal_domain(
    broker: &Broker,
    run: &Arc<RunRuntime>,
    expected: RunState,
) -> Result<(), BrokerError> {
    let actual = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .get_run(&run.id)
        .map_err(|error| BrokerError::journal("verify terminal run", error))?
        .ok_or_else(|| lifecycle_mismatch(&run.id, "terminal journal row is absent"))?
        .state;
    if actual != expected {
        return Err(lifecycle_mismatch(
            &run.id,
            &format!("lifecycle terminal is {expected:?} but journal is {actual:?}"),
        ));
    }
    prove_runtime_drained(run)
}

pub(super) fn prove_runtime_drained(run: &RunRuntime) -> Result<(), BrokerError> {
    let object_count = run
        .objects
        .lock()
        .map_err(|_| poisoned("run objects"))?
        .len();
    let stage_present = run
        .stage
        .0
        .lock()
        .map_err(|_| poisoned("run stage"))?
        .is_some();
    if object_count != 0 || stage_present {
        return Err(lifecycle_mismatch(
            &run.id,
            &format!(
                "terminal runtime is not drained: objects={object_count} stage_present={stage_present}"
            ),
        ));
    }
    Ok(())
}

pub(super) fn finish_run_transition_error(error: JournalError) -> BrokerError {
    match error {
        error @ JournalError::RunUndrained { .. } => BrokerError::new(
            StableCode::InvalidTransition,
            error.to_string(),
            "Finish the run as failed after every stage and object reaches a terminal state.",
        ),
        other => BrokerError::journal("finish run", other),
    }
}

pub(super) fn terminal_finish_conflict(run_id: &RunId, actual: RunState) -> BrokerError {
    BrokerError::new(
        StableCode::InvalidTransition,
        format!("run {run_id} is already terminal with state {actual:?}"),
        "Replay the identical completed finish request or inspect the recorded terminal result.",
    )
}

pub(super) fn terminal_abort_conflict(run_id: &RunId, actual: RunState) -> BrokerError {
    BrokerError::new(
        StableCode::InvalidTransition,
        format!("run {run_id} reached {actual:?}; it was not aborted"),
        "Inspect the durable terminal run and operation records; do not report an aborted result.",
    )
}
