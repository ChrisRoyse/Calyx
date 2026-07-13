use std::sync::Arc;

use crate::broker_error::BrokerError;
use crate::fs_tx::ObjectIdentity;
use crate::journal::{
    OperationState, RunState, StageState, TransactionRecord, TransactionState, TransitionUpdate,
};
use crate::protocol::{ResponseOutcome, StableCode, encode_response};

use super::{Broker, poisoned};

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
            StageState::Intent => broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .fail_stage_intent(
                    &stage.intent.stage_id,
                    125,
                    "broker restart before stage publication",
                )
                .map_err(|error| BrokerError::journal("fail replayed stage intent", error))?,
            StageState::Running => {
                let unit = stage.unit.as_ref().ok_or_else(|| {
                    recovery_required("running stage has no recorded unit identity")
                })?;
                let invocation = stage.invocation_id.as_ref().ok_or_else(|| {
                    recovery_required("running stage has no recorded invocation id")
                })?;
                let control_group = stage
                    .control_group
                    .as_ref()
                    .ok_or_else(|| recovery_required("running stage has no recorded cgroup"))?;
                crate::systemd::recover_recorded_stage(unit, invocation, control_group).map_err(
                    |error| {
                        recovery_required(format!(
                            "cannot drain recorded stage {}: {error}",
                            stage.intent.stage_id
                        ))
                    },
                )?;
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

fn recover_object(broker: &Broker, record: TransactionRecord) -> Result<(), BrokerError> {
    let root = broker.roots.get(&record.intent.root_alias).ok_or_else(|| {
        recovery_required(format!(
            "journal references unknown root {}",
            record.intent.root_alias
        ))
    })?;
    let prepared_name = format!("p-{}", record.intent.object_id);
    let quarantine_name = format!("q-{}", record.intent.object_id);
    let prepared = root.inspect_private(&prepared_name).map_err(recovery_fs)?;
    let quarantined = root
        .inspect_private(&quarantine_name)
        .map_err(recovery_fs)?;
    let published = root
        .inspect_shared(&record.intent.leaf)
        .map_err(recovery_fs)?;
    let present = usize::from(prepared.is_some())
        + usize::from(quarantined.is_some())
        + usize::from(published.is_some());

    if record.state == TransactionState::Intent {
        if quarantined.is_some() || published.is_some() {
            return Err(recovery_required(format!(
                "INTENT object {} crossed an impossible namespace boundary: prepared={} quarantined={} published={}",
                record.intent.object_id,
                prepared.is_some(),
                quarantined.is_some(),
                published.is_some()
            )));
        }
        if let Some(identity) = prepared {
            // begin_intent is committed only after require_namespace_absent,
            // and p-<object_id> lives in the root-only private directory. A
            // matching deterministic entry is therefore the observable kill
            // window after prepare mutated/fsynced the namespace but before
            // its identity was committed. Adopt it only as deletion authority.
            transition(
                broker,
                &record,
                TransactionState::Prepared,
                TransitionUpdate {
                    identity: Some(identity),
                    detail: Some(
                        "restart adopted deterministic prepared identity for exact rollback".into(),
                    ),
                    ..Default::default()
                },
            )?;
            let prepared_record = broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .get(&record.intent.object_id)
                .map_err(|error| BrokerError::journal("read adopted prepared object", error))?
                .ok_or_else(|| recovery_required("adopted prepared object row disappeared"))?;
            return recover_object(broker, prepared_record);
        }
        transition(
            broker,
            &record,
            TransactionState::Failed,
            TransitionUpdate {
                error_code: Some("REPLAYED_INTENT".into()),
                detail: Some("no namespace mutation observed".into()),
                ..Default::default()
            },
        )?;
        return Ok(());
    }

    if record.state == TransactionState::Quarantined && present == 0 {
        // QUARANTINED is the durable delete-start fence. Absence in all three
        // namespaces is the exact crash window after recursive deletion and
        // parent fsync but before the SQLite terminal transition. Re-fsync the
        // held private parent and only then commit Deleted.
        root.sync_private().map_err(recovery_fs)?;
        transition_deleted(broker, &record.intent.object_id)?;
        return Ok(());
    }

    let expected = record.identity.as_ref().ok_or_else(|| {
        recovery_required(format!(
            "object {} state {:?} has no opaque identity",
            record.intent.object_id, record.state
        ))
    })?;
    if present != 1 {
        return Err(recovery_required(format!(
            "object {} state {:?} has {present} namespace entries; expected exactly one",
            record.intent.object_id, record.state
        )));
    }
    verify_observed(expected, prepared.as_ref(), "prepared")?;
    verify_observed(expected, quarantined.as_ref(), "quarantined")?;
    verify_observed(expected, published.as_ref(), "published")?;

    match record.state {
        TransactionState::Prepared => {
            let quarantined = if prepared.is_some() {
                let object = root
                    .reopen_prepared(record.intent.object_id.clone(), expected)
                    .map_err(recovery_fs)?;
                root.quarantine_prepared(&object).map_err(recovery_fs)?
            } else if published.is_some() {
                let object = root
                    .reopen_published(
                        record.intent.object_id.clone(),
                        record.intent.leaf.clone(),
                        expected,
                    )
                    .map_err(recovery_fs)?;
                root.quarantine(&object).map_err(recovery_fs)?
            } else {
                root.reopen_quarantined(record.intent.object_id.clone(), &quarantine_name, expected)
                    .map_err(recovery_fs)?
            };
            transition_to_quarantined(broker, &record, &quarantined.identity, &quarantine_name)?;
            root.delete_quarantined(&quarantined).map_err(recovery_fs)?;
            transition_deleted(broker, &record.intent.object_id)?;
        }
        TransactionState::Published => {
            if prepared.is_some() {
                return Err(recovery_required(
                    "PUBLISHED object remained under prepared name",
                ));
            }
            let object = if published.is_some() {
                let published = root
                    .reopen_published(
                        record.intent.object_id.clone(),
                        record.intent.leaf.clone(),
                        expected,
                    )
                    .map_err(recovery_fs)?;
                root.quarantine(&published).map_err(recovery_fs)?
            } else {
                root.reopen_quarantined(record.intent.object_id.clone(), &quarantine_name, expected)
                    .map_err(recovery_fs)?
            };
            transition_to_quarantined(broker, &record, &object.identity, &quarantine_name)?;
            root.delete_quarantined(&object).map_err(recovery_fs)?;
            transition_deleted(broker, &record.intent.object_id)?;
        }
        TransactionState::DeleteIntent => {
            if prepared.is_some() {
                return Err(recovery_required(
                    "DELETE_INTENT object appeared under prepared name",
                ));
            }
            let object = if published.is_some() {
                let published = root
                    .reopen_published(
                        record.intent.object_id.clone(),
                        record.intent.leaf.clone(),
                        expected,
                    )
                    .map_err(recovery_fs)?;
                root.quarantine(&published).map_err(recovery_fs)?
            } else {
                root.reopen_quarantined(record.intent.object_id.clone(), &quarantine_name, expected)
                    .map_err(recovery_fs)?
            };
            transition_to_quarantined(broker, &record, &object.identity, &quarantine_name)?;
            root.delete_quarantined(&object).map_err(recovery_fs)?;
            transition_deleted(broker, &record.intent.object_id)?;
        }
        TransactionState::Quarantined => {
            if quarantined.is_none() {
                return Err(recovery_required(
                    "QUARANTINED object is not in private quarantine",
                ));
            }
            let object = root
                .reopen_quarantined(record.intent.object_id.clone(), &quarantine_name, expected)
                .map_err(recovery_fs)?;
            root.delete_quarantined(&object).map_err(recovery_fs)?;
            transition_deleted(broker, &record.intent.object_id)?;
        }
        state => {
            return Err(recovery_required(format!(
                "recover_object received non-recovery state {state:?}"
            )));
        }
    }
    Ok(())
}

fn transition_to_quarantined(
    broker: &Broker,
    record: &TransactionRecord,
    identity: &ObjectIdentity,
    quarantine_name: &str,
) -> Result<(), BrokerError> {
    transition(
        broker,
        record,
        TransactionState::Quarantined,
        TransitionUpdate {
            identity: Some(identity.clone()),
            quarantine_name: Some(quarantine_name.into()),
            detail: Some("restart replay quarantined exact opaque identity".into()),
            ..Default::default()
        },
    )
}

fn transition_deleted(
    broker: &Broker,
    object_id: &crate::protocol::ObjectId,
) -> Result<(), BrokerError> {
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .transition(
            object_id,
            TransactionState::Quarantined,
            TransactionState::Deleted,
            TransitionUpdate {
                detail: Some("restart replay deletion fsynced".into()),
                ..Default::default()
            },
        )
        .map_err(|error| BrokerError::journal("commit replayed deletion", error))
}

fn transition(
    broker: &Broker,
    record: &TransactionRecord,
    next: TransactionState,
    update: TransitionUpdate,
) -> Result<(), BrokerError> {
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .transition(&record.intent.object_id, record.state, next, update)
        .map_err(|error| BrokerError::journal("commit replay transition", error))
}

fn verify_observed(
    expected: &ObjectIdentity,
    observed: Option<&ObjectIdentity>,
    location: &str,
) -> Result<(), BrokerError> {
    if let Some(observed) = observed {
        if !observed.same_authority(expected) {
            return Err(recovery_required(format!(
                "{location} namespace entry has an opaque identity mismatch: expected dev/ino={}/{} observed={}/{}",
                expected.device, expected.inode, observed.device, observed.inode
            )));
        }
    }
    Ok(())
}

fn finish_pending_operations(broker: &Broker) -> Result<(), BrokerError> {
    let pending = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .list_pending_operations()
        .map_err(|error| BrokerError::journal("list pending operations", error))?;
    for operation in pending {
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

fn recovery_fs(error: crate::fs_tx::FsTxError) -> BrokerError {
    recovery_required(error.to_string())
}

fn recovery_required(message: impl Into<String>) -> BrokerError {
    BrokerError::new(
        StableCode::RecoveryRequired,
        message,
        "Do not delete or rename recovery entries manually; inspect SQLite, opaque identities, public/private namespaces, units, and cgroups.",
    )
    .fatal()
}
