use super::*;

pub(super) fn recover_object(
    broker: &Broker,
    record: TransactionRecord,
) -> Result<(), BrokerError> {
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
        // parent fsync but before the Turso terminal transition. Re-fsync the
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

pub(super) fn transition_to_quarantined(
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

pub(super) fn transition_deleted(
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

pub(super) fn transition(
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

pub(super) fn verify_observed(
    expected: &ObjectIdentity,
    observed: Option<&ObjectIdentity>,
    location: &str,
) -> Result<(), BrokerError> {
    if let Some(observed) = observed
        && !observed.same_authority(expected)
    {
        return Err(recovery_required(format!(
            "{location} namespace entry has an opaque identity mismatch: expected dev/ino={}/{} observed={}/{}",
            expected.device, expected.inode, observed.device, observed.inode
        )));
    }
    Ok(())
}
