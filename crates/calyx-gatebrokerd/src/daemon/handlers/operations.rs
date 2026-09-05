use super::*;

pub(super) fn durable_operation<F>(
    broker: &Broker,
    frame: &[u8],
    verb: &str,
    run_id: Option<RunId>,
    request_id: RequestId,
    execution_allowed: bool,
    action: F,
) -> Result<Reply, BrokerError>
where
    F: FnOnce() -> Result<Response, BrokerError>,
{
    let intent = OperationIntent {
        request_id: request_id.clone(),
        request_hash: *blake3::hash(frame).as_bytes(),
        request_json: frame.to_vec(),
        verb: verb.into(),
        run_id,
    };
    if !execution_allowed {
        let existing = broker
            .journal
            .lock()
            .map_err(|_| poisoned("journal"))?
            .get_operation(&request_id)
            .map_err(|error| BrokerError::journal("read terminal request operation", error))?;
        let existing = existing.ok_or_else(|| {
            BrokerError::new(
                StableCode::InvalidTransition,
                format!("run for request {request_id} is terminal"),
                "Only an identical previously completed request may be replayed for a terminal run.",
            )
        })?;
        if existing.intent.request_hash != intent.request_hash
            || existing.intent.verb != intent.verb
            || existing.intent.run_id != intent.run_id
        {
            return Err(BrokerError::new(
                StableCode::InvalidRequest,
                format!("request id {request_id} was reused with different bytes or authority"),
                "Use the original exact frame for replay or a new request id for a new operation.",
            ));
        }
        return replay_operation(request_id, existing);
    }
    let begun = begin_operation_checked(broker, &intent)?;
    if let BeginOperation::Existing(existing) = begun {
        return match existing.state {
            OperationState::Pending => Err(BrokerError::new(
                StableCode::Busy,
                format!("request {request_id} is already executing"),
                "Wait for the in-flight operation and retry the identical frame with the same request id.",
            )),
            OperationState::Succeeded | OperationState::Failed => {
                replay_operation(request_id, existing)
            }
        };
    }

    terminalize_operation(broker, request_id, action())
}

pub(super) fn terminalize_operation(
    broker: &Broker,
    request_id: RequestId,
    outcome: Result<Response, BrokerError>,
) -> Result<Reply, BrokerError> {
    let (envelope, operation_state, error_code, error) = match outcome {
        Ok(response) => (
            Reply::success(request_id.clone(), response).envelope,
            OperationState::Succeeded,
            None,
            None,
        ),
        Err(error) => (
            error.response(request_id.clone()),
            OperationState::Failed,
            Some(code_name(error.code)),
            Some(error),
        ),
    };
    let encoded = encode_response(&envelope)
        .map_err(|error| BrokerError::journal("encode durable operation response", error))?;
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .finish_operation(&request_id, operation_state, &encoded, error_code)
        .map_err(|error| BrokerError::journal("finish request operation", error))?;
    Ok(Reply { envelope, error })
}

pub(super) fn durable_run_operation<F>(
    broker: &Broker,
    run: Option<Arc<RunRuntime>>,
    operation: RunOperation<'_>,
    action: F,
) -> Result<Reply, BrokerError>
where
    F: FnOnce(Arc<RunRuntime>) -> Result<Response, BrokerError>,
{
    let RunOperation {
        peer,
        frame,
        verb,
        run_id,
        run_token,
        request_id,
    } = operation;
    let Some(run) = run else {
        return durable_operation(broker, frame, verb, Some(run_id), request_id, false, || {
            unreachable!("terminal operation action is never called")
        });
    };

    // This guard deliberately encloses the second authorization check, the
    // operation row, every domain transition, and the terminal response row.
    // A stale Arc can therefore only replay after Finish/Abort, never execute.
    let _sequence = run
        .lifecycle
        .sequence
        .lock()
        .map_err(|_| poisoned("run lifecycle sequence"))?;
    if !runtime_is_current(broker, &run)? {
        return durable_operation(broker, frame, verb, Some(run_id), request_id, false, || {
            unreachable!("removed runtime action is never called")
        });
    }
    revalidate_run_authority(&run, peer, &run_token, true)?;
    {
        let mut state = run
            .lifecycle
            .state
            .lock()
            .map_err(|_| poisoned("run lifecycle state"))?;
        if matches!(*state, RunLifecycleState::Terminal { .. }) {
            while runtime_is_current(broker, &run)? {
                state = run
                    .lifecycle
                    .changed
                    .wait(state)
                    .map_err(|_| poisoned("run lifecycle state"))?;
            }
        } else if *state != RunLifecycleState::Active {
            drop(state);
            return durable_operation(broker, frame, verb, Some(run_id), request_id, false, || {
                unreachable!("non-active runtime action is never called")
            });
        }
        if !runtime_is_current(broker, &run)? {
            drop(state);
            return durable_operation(broker, frame, verb, Some(run_id), request_id, false, || {
                unreachable!("terminalized runtime action is never called")
            });
        }
    }
    let journal_state = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .get_run(&run.id)
        .map_err(|error| BrokerError::journal("revalidate live run", error))?
        .ok_or_else(|| lifecycle_mismatch(&run.id, "runtime exists but journal row is absent"))?
        .state;
    if journal_state != RunState::Active {
        return Err(lifecycle_mismatch(
            &run.id,
            &format!("runtime is active but journal state is {journal_state:?}"),
        ));
    }
    let reply = durable_operation(broker, frame, verb, Some(run_id), request_id, true, || {
        action(Arc::clone(&run))
    });
    if reply.is_ok() {
        finalize_terminal_runtime(broker, &run, false)?;
    }
    reply
}

pub(super) fn begin_operation_checked(
    broker: &Broker,
    intent: &OperationIntent,
) -> Result<BeginOperation, BrokerError> {
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .begin_operation(intent)
        .map_err(|error| match &error {
            JournalError::RequestConflict { .. } => BrokerError::new(
                StableCode::InvalidRequest,
                error.to_string(),
                "Use the original exact frame for replay or a new request id for a new operation.",
            ),
            JournalError::RunNotActive { .. } => BrokerError::new(
                StableCode::InvalidTransition,
                error.to_string(),
                "The run became terminal; only an identical completed request may be replayed.",
            ),
            _ => BrokerError::journal("begin request operation", error),
        })
}

pub(super) fn durable_abort_operation(
    broker: &Broker,
    peer: PeerCredentials,
    frame: &[u8],
    run: Option<Arc<RunRuntime>>,
    request: crate::protocol::AbortRunRequest,
) -> Result<Reply, BrokerError> {
    let request_id = request.request_id.clone();
    let run_id = request.run_id.clone();
    let intent = OperationIntent {
        request_id: request_id.clone(),
        request_hash: *blake3::hash(frame).as_bytes(),
        request_json: frame.to_vec(),
        verb: "abort_run".into(),
        run_id: Some(run_id.clone()),
    };
    let Some(run) = run else {
        return terminal_run_replay(broker, &intent);
    };
    let preexisting = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .get_operation(&request_id)
        .map_err(|error| BrokerError::journal("preflight abort replay", error))?
        .is_some();
    if !preexisting {
        revalidate_run_authority(&run, peer, &request.run_token, true)?;
    }

    let begun = {
        // The state lock is the terminal-claim boundary. Finish holds this
        // same lock across its journal CAS, so either this abort intent joins
        // the active run or it observes the exact terminal result.
        let mut state = run
            .lifecycle
            .state
            .lock()
            .map_err(|_| poisoned("run lifecycle state"))?;
        if !runtime_is_current(broker, &run)? {
            drop(state);
            return terminal_run_replay(broker, &intent);
        }
        if matches!(*state, RunLifecycleState::Terminal { .. }) {
            while runtime_is_current(broker, &run)? {
                state = run
                    .lifecycle
                    .changed
                    .wait(state)
                    .map_err(|_| poisoned("run lifecycle state"))?;
            }
            drop(state);
            return terminal_run_replay(broker, &intent);
        }
        if !preexisting {
            revalidate_run_authority(&run, peer, &request.run_token, false)?;
        }
        let begun = begin_operation_checked(broker, &intent)?;
        if matches!(begun, BeginOperation::Inserted) {
            match &mut *state {
                RunLifecycleState::Active => {
                    *state = RunLifecycleState::AbortRequested {
                        pending_operations: 1,
                    };
                    run.lifecycle.abort_signal.store(true, Ordering::Release);
                    run.lifecycle.changed.notify_all();
                }
                RunLifecycleState::AbortRequested { pending_operations } => {
                    *pending_operations = pending_operations.checked_add(1).ok_or_else(|| {
                        lifecycle_mismatch(&run.id, "abort operation waiter count overflowed")
                    })?;
                }
                RunLifecycleState::Terminal { .. } => {
                    return Err(lifecycle_mismatch(
                        &run.id,
                        "abort operation was inserted after terminal claim",
                    ));
                }
            }
        }
        begun
    };

    match begun {
        BeginOperation::Existing(existing) if existing.state != OperationState::Pending => {
            replay_operation(request_id, existing)
        }
        BeginOperation::Existing(_) => wait_for_operation_terminal(broker, &run, &intent),
        BeginOperation::Inserted => {
            let outcome =
                match abort_run_internal(broker, Arc::clone(&run), request.reason.as_str()) {
                    Ok(AbortOutcome::Aborted | AbortOutcome::AlreadyAborted) => {
                        Ok(Response::RunAborted { run_id })
                    }
                    Ok(AbortOutcome::Terminal(actual)) => {
                        Err(terminal_abort_conflict(&run.id, actual))
                    }
                    Err(error) => Err(error),
                };
            let reply = terminalize_operation(broker, request_id, outcome);
            if reply.is_ok() {
                finalize_terminal_runtime(broker, &run, true)?;
            }
            reply
        }
    }
}

pub(super) fn wait_for_operation_terminal(
    broker: &Broker,
    run: &RunRuntime,
    intent: &OperationIntent,
) -> Result<Reply, BrokerError> {
    loop {
        let existing = broker
            .journal
            .lock()
            .map_err(|_| poisoned("journal"))?
            .get_operation(&intent.request_id)
            .map_err(|error| BrokerError::journal("join abort operation", error))?
            .ok_or_else(|| {
                lifecycle_mismatch(&run.id, "pending abort operation disappeared from journal")
            })?;
        if existing.intent.request_hash != intent.request_hash
            || existing.intent.verb != intent.verb
            || existing.intent.run_id != intent.run_id
        {
            return Err(BrokerError::new(
                StableCode::InvalidRequest,
                format!(
                    "request id {} was reused with different bytes or authority",
                    intent.request_id
                ),
                "Use the original exact frame for replay or a new request id for a new operation.",
            ));
        }
        if existing.state != OperationState::Pending {
            return replay_operation(intent.request_id.clone(), existing);
        }

        let state = run
            .lifecycle
            .state
            .lock()
            .map_err(|_| poisoned("run lifecycle state"))?;
        // Re-read while holding the notification mutex so terminalization
        // cannot be missed between the query above and this wait.
        let pending = broker
            .journal
            .lock()
            .map_err(|_| poisoned("journal"))?
            .get_operation(&intent.request_id)
            .map_err(|error| BrokerError::journal("recheck joined abort operation", error))?
            .is_some_and(|record| record.state == OperationState::Pending);
        if !pending {
            drop(state);
            continue;
        }
        drop(
            run.lifecycle
                .changed
                .wait(state)
                .map_err(|_| poisoned("run lifecycle state"))?,
        );
    }
}

pub(super) fn terminal_run_replay(
    broker: &Broker,
    intent: &OperationIntent,
) -> Result<Reply, BrokerError> {
    let existing = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .get_operation(&intent.request_id)
        .map_err(|error| BrokerError::journal("read terminal abort operation", error))?;
    let Some(existing) = existing else {
        let actual = match intent.run_id.as_ref() {
            Some(run_id) => broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .get_run(run_id)
                .map_err(|error| BrokerError::journal("read terminal abort run", error))?
                .map(|record| record.state),
            None => None,
        };
        return Err(BrokerError::new(
            StableCode::InvalidTransition,
            format!(
                "abort request {} cannot start because run state is {}",
                intent.request_id,
                actual
                    .map(|state| format!("{state:?}"))
                    .unwrap_or_else(|| "terminal or unavailable".into())
            ),
            "Only an identical completed abort request may be replayed after terminalization.",
        ));
    };
    if existing.intent.request_hash != intent.request_hash
        || existing.intent.verb != intent.verb
        || existing.intent.run_id != intent.run_id
    {
        return Err(BrokerError::new(
            StableCode::InvalidRequest,
            format!(
                "request id {} was reused with different bytes or authority",
                intent.request_id
            ),
            "Use the original exact frame for replay or a new request id for a new operation.",
        ));
    }
    replay_operation(intent.request_id.clone(), existing)
}

pub(super) fn replay_operation(
    request_id: RequestId,
    existing: crate::journal::OperationRecord,
) -> Result<Reply, BrokerError> {
    if existing.state == OperationState::Pending {
        return Err(BrokerError::new(
            StableCode::Busy,
            format!("request {request_id} is already executing"),
            "Wait for the in-flight operation and retry the identical frame with the same request id.",
        ));
    }
    let encoded = existing.response_json.ok_or_else(|| {
        BrokerError::journal(
            "replay request operation",
            "terminal operation has no response payload",
        )
    })?;
    let response = decode_response(&encoded)
        .map_err(|error| BrokerError::journal("decode replayed response", error))?;
    if response.request_id != request_id {
        return Err(BrokerError::journal(
            "verify replayed response",
            "response request id does not match operation key",
        ));
    }
    Ok(Reply {
        envelope: response,
        error: None,
    })
}
