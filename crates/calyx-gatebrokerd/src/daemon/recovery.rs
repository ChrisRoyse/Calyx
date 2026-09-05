use std::sync::Arc;

use crate::broker_error::BrokerError;
use crate::fs_tx::ObjectIdentity;
use crate::journal::{
    OperationRecord, OperationState, RunState, StageState, TransactionRecord, TransactionState,
    TransitionUpdate,
};
use crate::protocol::{
    PROTOCOL_VERSION, Request, Response, ResponseEnvelope, ResponseOutcome, StableCode,
    decode_request, encode_response,
};
use crate::systemd::{CgroupIdentity, RecoveryOutcome, WorkerIdentity};

use super::{Broker, poisoned};
mod object;
mod operations;

use object::*;
use operations::*;

pub(super) fn replay(broker: &Arc<Broker>) -> Result<(), BrokerError> {
    // Process authority is drained before any writable object is touched.
    let incomplete_stages = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .list_incomplete_stages()
        .map_err(|error| BrokerError::journal("list incomplete stages", error))?;
    for stage in incomplete_stages {
        match stage.state {
            StageState::Intent => {
                let worker = WorkerIdentity {
                    user: stage.intent.worker_user.clone(),
                    uid: stage.intent.worker_uid,
                };
                crate::systemd::recover_worker_boundary(&worker).map_err(|error| {
                    recovery_required(format!(
                        "cannot drain worker/fixed-slice authority for intent stage {}: {error}",
                        stage.intent.stage_id
                    ))
                })?;
                crate::systemd::verify_worker_idle(&worker.user, worker.uid).map_err(|error| {
                    recovery_required(format!(
                        "worker is not idle after intent-stage drain {}: {error}",
                        stage.intent.stage_id
                    ))
                })?;
                broker
                    .journal
                    .lock()
                    .map_err(|_| poisoned("journal"))?
                    .fail_stage_intent(
                        &stage.intent.stage_id,
                        125,
                        "broker restart proved worker and fixed slice idle before intent failure",
                    )
                    .map_err(|error| BrokerError::journal("fail replayed stage intent", error))?;
            }
            StageState::Running => {
                let invocation = stage.invocation_id.as_ref().ok_or_else(|| {
                    recovery_required("running stage has no recorded invocation id")
                })?;
                let control_group = stage
                    .control_group
                    .as_ref()
                    .ok_or_else(|| recovery_required("running stage has no recorded cgroup"))?;
                let slice_control_group = stage.slice_control_group.as_ref().ok_or_else(|| {
                    recovery_required("running stage has no recorded slice cgroup")
                })?;
                let service_identity = stage.control_group_identity.ok_or_else(|| {
                    recovery_required("running stage has no service cgroup identity")
                })?;
                let slice_identity = stage.slice_control_group_identity.ok_or_else(|| {
                    recovery_required("running stage has no slice cgroup identity")
                })?;
                crate::systemd::recover_recorded_stage(
                    &stage.intent.unit,
                    invocation,
                    &CgroupIdentity {
                        control_group: control_group.clone(),
                        device: service_identity.device,
                        inode: service_identity.inode,
                    },
                    &CgroupIdentity {
                        control_group: slice_control_group.clone(),
                        device: slice_identity.device,
                        inode: slice_identity.inode,
                    },
                    &WorkerIdentity {
                        user: stage.intent.worker_user.clone(),
                        uid: stage.intent.worker_uid,
                    },
                )
                .map_err(|error| {
                    recovery_required(format!(
                        "cannot drain recorded stage {}: {error}",
                        stage.intent.stage_id
                    ))
                })?;
                broker
                    .journal
                    .lock()
                    .map_err(|_| poisoned("journal"))?
                    .finish_stage(&stage.intent.stage_id, 125)
                    .map_err(|error| BrokerError::journal("finish replayed stage", error))?;
            }
            StageState::Succeeded | StageState::Failed => {}
        }
    }

    // Terminal rows are audited independently so a prior cleanup failure can
    // never hide live authority from the incomplete-stage query above.
    let terminal_stages = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .list_terminal_stages()
        .map_err(|error| BrokerError::journal("list terminal stages", error))?;
    for stage in terminal_stages {
        let worker = WorkerIdentity {
            user: stage.intent.worker_user.clone(),
            uid: stage.intent.worker_uid,
        };
        match (
            stage.invocation_id.as_ref(),
            stage.control_group.as_ref(),
            stage.slice_control_group.as_ref(),
            stage.control_group_identity,
            stage.slice_control_group_identity,
        ) {
            (
                Some(invocation),
                Some(control_group),
                Some(slice_control_group),
                Some(service_identity),
                Some(slice_identity),
            ) => crate::systemd::audit_terminal_recorded_stage(
                &stage.intent.unit,
                invocation,
                &CgroupIdentity {
                    control_group: control_group.clone(),
                    device: service_identity.device,
                    inode: service_identity.inode,
                },
                &CgroupIdentity {
                    control_group: slice_control_group.clone(),
                    device: slice_identity.device,
                    inode: slice_identity.inode,
                },
                &worker,
            )
            .map_err(|error| {
                recovery_required(format!(
                    "terminal stage {} failed authority audit: {error}",
                    stage.intent.stage_id
                ))
            })?,
            (None, None, None, None, None) => {
                let outcome =
                    crate::systemd::recover_worker_boundary(&worker).map_err(|error| {
                        recovery_required(format!(
                            "terminal pre-publication stage {} failed fixed-slice audit: {error}",
                            stage.intent.stage_id
                        ))
                    })?;
                crate::systemd::verify_worker_idle(&worker.user, worker.uid).map_err(|error| {
                    recovery_required(format!(
                        "terminal pre-publication stage {} left worker authority: {error}",
                        stage.intent.stage_id
                    ))
                })?;
                if outcome == RecoveryOutcome::Killed {
                    return Err(recovery_required(format!(
                        "terminal stage {} concealed live pre-publication authority; fixed slice was drained",
                        stage.intent.stage_id
                    )));
                }
            }
            _ => {
                return Err(recovery_required(format!(
                    "terminal stage {} has partial launch identity",
                    stage.intent.stage_id
                )));
            }
        }
    }

    let incomplete = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .list_incomplete()
        .map_err(|error| BrokerError::journal("list incomplete objects", error))?;
    for record in incomplete {
        recover_object(broker, record)?;
    }

    let active_runs = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .list_active_runs()
        .map_err(|error| BrokerError::journal("list active runs", error))?;
    for run in active_runs {
        let records = broker
            .journal
            .lock()
            .map_err(|_| poisoned("journal"))?
            .list_objects_for_run(&run.intent.run_id)
            .map_err(|error| BrokerError::journal("list active-run objects", error))?;
        for record in records {
            match record.state {
                TransactionState::Committed => {
                    broker
                        .journal
                        .lock()
                        .map_err(|_| poisoned("journal"))?
                        .transition(
                            &record.intent.object_id,
                            TransactionState::Committed,
                            TransactionState::DeleteIntent,
                            TransitionUpdate {
                                detail: Some("broker restart cleanup".into()),
                                ..Default::default()
                            },
                        )
                        .map_err(|error| {
                            BrokerError::journal("record restart delete intent", error)
                        })?;
                    let current = broker
                        .journal
                        .lock()
                        .map_err(|_| poisoned("journal"))?
                        .get(&record.intent.object_id)
                        .map_err(|error| BrokerError::journal("read restart object", error))?
                        .ok_or_else(|| recovery_required("restart object row disappeared"))?;
                    recover_object(broker, current)?;
                }
                TransactionState::Deleted | TransactionState::Failed => {}
                TransactionState::MismatchPreserved => {
                    return Err(recovery_required(format!(
                        "object {} is mismatch_preserved and requires operator recovery",
                        record.intent.object_id
                    )));
                }
                state if state.is_recovery_required() => {
                    // It was already processed from list_incomplete. Require
                    // the independent read to show a terminal state now.
                    let current = broker
                        .journal
                        .lock()
                        .map_err(|_| poisoned("journal"))?
                        .get(&record.intent.object_id)
                        .map_err(|error| BrokerError::journal("verify replayed object", error))?
                        .ok_or_else(|| recovery_required("replayed object row disappeared"))?;
                    if !matches!(
                        current.state,
                        TransactionState::Deleted | TransactionState::Failed
                    ) {
                        return Err(recovery_required(format!(
                            "object {} remained {:?} after replay",
                            current.intent.object_id, current.state
                        )));
                    }
                }
                state => {
                    return Err(recovery_required(format!(
                        "unexpected active-run object state {state:?}"
                    )));
                }
            }
        }
        broker
            .journal
            .lock()
            .map_err(|_| poisoned("journal"))?
            .finish_run(
                &run.intent.run_id,
                RunState::Aborted,
                Some("broker restart invalidated pidfd/control lease"),
            )
            .map_err(|error| BrokerError::journal("abort replayed run", error))?;
    }

    finish_pending_operations(broker)?;
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .checkpoint()
        .map_err(|error| BrokerError::journal("checkpoint after recovery", error))?;
    Ok(())
}
