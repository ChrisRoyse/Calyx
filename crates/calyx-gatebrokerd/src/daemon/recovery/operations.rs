use super::*;

pub(super) fn finish_pending_operations(broker: &Broker) -> Result<(), BrokerError> {
    let pending = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .list_pending_operations()
        .map_err(|error| BrokerError::journal("list pending operations", error))?;
    for operation in pending {
        if let Some(response) = recovered_operation_response(broker, &operation)? {
            let envelope = ResponseEnvelope {
                version: PROTOCOL_VERSION,
                request_id: operation.intent.request_id.clone(),
                outcome: ResponseOutcome::Ok(response),
            };
            let bytes = encode_response(&envelope)
                .map_err(|encode| BrokerError::journal("encode recovered operation", encode))?;
            broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .finish_operation(
                    &operation.intent.request_id,
                    OperationState::Succeeded,
                    &bytes,
                    None,
                )
                .map_err(|journal| {
                    BrokerError::journal("finish recovered successful operation", journal)
                })?;
            continue;
        }
        let error = BrokerError::new(
            StableCode::RecoveryRequired,
            format!(
                "operation {} was interrupted by broker restart and its run was aborted",
                operation.intent.request_id
            ),
            "Inspect the run/object/stage rows, then start a new run; terminal operation replay never invents success.",
        );
        let response = error.response(operation.intent.request_id.clone());
        debug_assert!(matches!(response.outcome, ResponseOutcome::Error(_)));
        let bytes = encode_response(&response)
            .map_err(|encode| BrokerError::journal("encode recovered operation", encode))?;
        broker
            .journal
            .lock()
            .map_err(|_| poisoned("journal"))?
            .finish_operation(
                &operation.intent.request_id,
                OperationState::Failed,
                &bytes,
                Some("RECOVERY_REQUIRED"),
            )
            .map_err(|journal| BrokerError::journal("finish recovered operation", journal))?;
    }
    Ok(())
}

pub(super) fn recovered_operation_response(
    broker: &Broker,
    operation: &OperationRecord,
) -> Result<Option<Response>, BrokerError> {
    let envelope = decode_request(&operation.intent.request_json)
        .map_err(|error| BrokerError::journal("decode recorded operation request", error))?;
    let response = match envelope.request {
        Request::FinishRun(request) => {
            let run = broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .get_run(&request.run_id)
                .map_err(|error| BrokerError::journal("read recovered finish run", error))?;
            run.filter(|record| record.state == RunState::from(request.intended_status))
                .map(|_| Response::RunFinished {
                    run_id: request.run_id,
                    status: request.intended_status,
                })
        }
        Request::AbortRun(request) => {
            let run = broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .get_run(&request.run_id)
                .map_err(|error| BrokerError::journal("read recovered aborted run", error))?;
            run.filter(|record| record.state == RunState::Aborted)
                .map(|_| Response::RunAborted {
                    run_id: request.run_id,
                })
        }
        Request::DeleteObject(request) => {
            let object = broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .get(&request.object_id)
                .map_err(|error| BrokerError::journal("read recovered deleted object", error))?;
            object
                .filter(|record| record.state == TransactionState::Deleted)
                .map(|_| Response::ObjectDeleted {
                    object_id: request.object_id,
                })
        }
        Request::ExecStage(_) => {
            let stage = broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .get_stage_by_request(&operation.intent.request_id)
                .map_err(|error| BrokerError::journal("read recovered stage", error))?;
            stage.and_then(|record| {
                if !matches!(record.state, StageState::Succeeded | StageState::Failed) {
                    return None;
                }
                Some(Response::StageFinished {
                    stage_id: record.intent.stage_id,
                    unit: record.intent.unit,
                    invocation_id: record.invocation_id?,
                    control_group: record.control_group?,
                    exit_status: record.exit_status?,
                })
            })
        }
        Request::BeginRun(_) | Request::CreateObject(_) => None,
        Request::Health(_) | Request::Inspect(_) => {
            return Err(recovery_required(format!(
                "pending operation {} contains a non-durable request",
                operation.intent.request_id
            )));
        }
    };
    Ok(response)
}

pub(super) fn recovery_fs(error: crate::fs_tx::FsTxError) -> BrokerError {
    recovery_required(error.to_string())
}

pub(super) fn recovery_required(message: impl Into<String>) -> BrokerError {
    BrokerError::new(
        StableCode::RecoveryRequired,
        message,
        "Do not delete or rename recovery entries manually; inspect Turso, opaque identities, public/private namespaces, units, and cgroups.",
    )
    .fatal()
}
