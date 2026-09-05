use super::*;

pub(super) fn create_object(
    broker: &Broker,
    run: Arc<RunRuntime>,
    request: crate::protocol::CreateObjectRequest,
) -> Result<Response, BrokerError> {
    ensure_run_mutable(&run)?;
    ensure_no_stage(&run)?;
    let root = broker
        .roots
        .get(&request.root_alias)
        .cloned()
        .ok_or_else(|| not_found("managed root", request.root_alias.as_str()))?;
    let object_id =
        ids::object_id().map_err(|error| BrokerError::system("generate object id", error))?;
    let leaf = match request.leaf {
        Some(value) => value,
        None => LeafName::new(format!("{}-{object_id}", request.role))
            .map_err(|error| BrokerError::invalid(error.to_string()))?,
    };
    require_namespace_absent(&root, &object_id, &leaf)?;
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .begin_intent(&IntentRecord {
            object_id: object_id.clone(),
            request_id: request.request_id,
            run_id: run.id.clone(),
            role: request.role,
            root_alias: request.root_alias.clone(),
            leaf: leaf.clone(),
        })
        .map_err(|error| BrokerError::journal("record create intent", error))?;

    let prepared = match root.prepare(object_id.clone()) {
        Ok(value) => value,
        Err(error) => {
            fail_clean_intent(broker, &root, &object_id, &leaf, &error)?;
            return Err(fs_error("prepare object", error, false));
        }
    };
    transition(
        broker,
        &object_id,
        TransactionState::Intent,
        TransactionState::Prepared,
        TransitionUpdate {
            identity: Some(prepared.identity.clone()),
            ..Default::default()
        },
    )?;

    crate::systemd::verify_worker_idle(&broker.worker.name, broker.worker.uid)
        .map_err(|error| systemd_error("verify worker idle before object publication", error))?;
    let published = match root.publish(&prepared, leaf.clone()) {
        Ok(value) => value,
        Err(FsTxError::Collision(path)) => {
            discard_prepared(broker, &root, &prepared, "PUBLISH_COLLISION")?;
            return Err(BrokerError::new(
                StableCode::ObjectCollision,
                format!("object destination already exists: {path}"),
                "Choose a unique leaf; the broker never adopts or replaces existing objects.",
            ));
        }
        Err(error) => reconcile_publish_error(broker, &root, &prepared, &leaf, error)?,
    };
    transition(
        broker,
        &object_id,
        TransactionState::Prepared,
        TransactionState::Published,
        TransitionUpdate {
            identity: Some(published.identity.clone()),
            ..Default::default()
        },
    )?;
    transition(
        broker,
        &object_id,
        TransactionState::Published,
        TransactionState::Committed,
        TransitionUpdate::default(),
    )?;
    prove_published(&root, &object_id, &published)?;
    run.objects
        .lock()
        .map_err(|_| poisoned("run objects"))?
        .insert(
            object_id.clone(),
            LiveObject {
                root_alias: request.root_alias.clone(),
                leaf: leaf.clone(),
                published: published.clone(),
            },
        );
    let configured = broker
        .config
        .root(&request.root_alias)
        .ok_or_else(|| not_found("managed root", request.root_alias.as_str()))?;
    let root_path = absolute_path(&configured.raw().shared)?;
    let absolute = absolute_path(&configured.raw().shared.join(leaf.as_str()))?;
    Ok(Response::ObjectCreated {
        object_id,
        absolute_path: absolute,
        root_path,
        root_identity: diagnostic(root.root_identity()),
        object_identity: diagnostic(&published.identity),
        state: ObjectState::Published,
    })
}

pub(super) fn reconcile_publish_error(
    broker: &Broker,
    root: &FsRoot,
    prepared: &PreparedObject,
    leaf: &LeafName,
    error: FsTxError,
) -> Result<PublishedObject, BrokerError> {
    let prepared_now = root
        .inspect_private(&prepared.private_name)
        .map_err(|inspect| fatal_fs("inspect failed publish preparation", inspect))?;
    let quarantine_now = root
        .inspect_private(&format!("q-{}", prepared.object_id))
        .map_err(|inspect| fatal_fs("inspect failed publish quarantine", inspect))?;
    let published_now = root
        .inspect_shared(leaf)
        .map_err(|inspect| fatal_fs("inspect failed publish destination", inspect))?;
    if quarantine_now.is_none()
        && prepared_now.is_none()
        && published_now
            .as_ref()
            .is_some_and(|value| value.same_authority(&prepared.identity))
    {
        return root
            .reopen_published(prepared.object_id.clone(), leaf.clone(), &prepared.identity)
            .map_err(|reopen| fatal_fs("reopen reconciled published object", reopen));
    }
    if quarantine_now.is_none()
        && published_now.is_none()
        && prepared_now
            .as_ref()
            .is_some_and(|value| value.same_authority(&prepared.identity))
    {
        discard_prepared(broker, root, prepared, "PUBLISH_FAILED")?;
        return Err(fs_error("publish object", error, false));
    }
    if quarantine_now.is_none()
        && prepared_now
            .as_ref()
            .is_some_and(|value| value.same_authority(&prepared.identity))
        && published_now.is_some()
    {
        discard_prepared(broker, root, prepared, "PUBLISH_COLLISION")?;
        return Err(BrokerError::new(
            StableCode::ObjectCollision,
            format!("publish failed because {leaf} is occupied: {error}"),
            "Choose a unique leaf; the existing destination was preserved.",
        ));
    }
    Err(fatal_fs("reconcile failed publish", error))
}

pub(super) fn discard_prepared(
    broker: &Broker,
    root: &FsRoot,
    prepared: &PreparedObject,
    detail: &str,
) -> Result<(), BrokerError> {
    let quarantined = root
        .quarantine_prepared(prepared)
        .map_err(|error| fatal_fs("quarantine failed preparation", error))?;
    transition(
        broker,
        &prepared.object_id,
        TransactionState::Prepared,
        TransactionState::Quarantined,
        TransitionUpdate {
            identity: Some(quarantined.identity.clone()),
            quarantine_name: Some(quarantined.private_name.clone()),
            detail: Some(detail.into()),
            ..Default::default()
        },
    )?;
    root.delete_quarantined(&quarantined)
        .map_err(|error| fatal_fs("delete failed preparation", error))?;
    transition(
        broker,
        &prepared.object_id,
        TransactionState::Quarantined,
        TransactionState::Deleted,
        TransitionUpdate {
            detail: Some(detail.into()),
            ..Default::default()
        },
    )?;
    prove_absent(root, &prepared.object_id, None)
}

pub(super) fn fail_clean_intent(
    broker: &Broker,
    root: &FsRoot,
    object_id: &ObjectId,
    leaf: &LeafName,
    error: &FsTxError,
) -> Result<(), BrokerError> {
    let prepared = root
        .inspect_private(&format!("p-{object_id}"))
        .map_err(|failure| fatal_fs("inspect failed prepare", failure))?;
    let quarantined = root
        .inspect_private(&format!("q-{object_id}"))
        .map_err(|failure| fatal_fs("inspect failed prepare quarantine", failure))?;
    let published = root
        .inspect_shared(leaf)
        .map_err(|failure| fatal_fs("inspect failed prepare destination", failure))?;
    if prepared.is_some() || quarantined.is_some() || published.is_some() {
        return Err(BrokerError::new(
            StableCode::RecoveryRequired,
            format!(
                "prepare failed for {object_id}, but namespace state is not empty: prepared={} quarantined={} published={}: {error}",
                prepared.is_some(),
                quarantined.is_some(),
                published.is_some()
            ),
            "Do not delete the preserved entry by pathname; inspect the journal and opaque identity evidence.",
        )
        .fatal());
    }
    transition(
        broker,
        object_id,
        TransactionState::Intent,
        TransactionState::Failed,
        TransitionUpdate {
            error_code: Some("PREPARE_FAILED".into()),
            detail: Some(error.to_string()),
            ..Default::default()
        },
    )
}

pub(super) fn delete_object(
    broker: &Broker,
    run: &RunRuntime,
    object_id: &ObjectId,
    detail: &str,
) -> Result<(), BrokerError> {
    let object = run
        .objects
        .lock()
        .map_err(|_| poisoned("run objects"))?
        .get(object_id)
        .cloned()
        .ok_or_else(|| not_found("live object", object_id.as_str()))?;
    let root = broker
        .roots
        .get(&object.root_alias)
        .ok_or_else(|| not_found("managed root", object.root_alias.as_str()))?;
    transition(
        broker,
        object_id,
        TransactionState::Committed,
        TransactionState::DeleteIntent,
        TransitionUpdate {
            detail: Some(detail.into()),
            ..Default::default()
        },
    )?;
    let quarantined = match root.quarantine(&object.published) {
        Ok(value) => value,
        Err(error @ FsTxError::IdentityMismatch { .. }) => {
            transition(
                broker,
                object_id,
                TransactionState::DeleteIntent,
                TransactionState::MismatchPreserved,
                TransitionUpdate {
                    error_code: Some("OBJECT_IDENTITY_MISMATCH".into()),
                    detail: Some(error.to_string()),
                    ..Default::default()
                },
            )?;
            return Err(BrokerError::new(
                StableCode::ObjectMismatch,
                error.to_string(),
                "The replacement was preserved. Inspect the journal event and both namespaces before operator recovery.",
            )
            .fatal());
        }
        Err(error) => return Err(fatal_fs("quarantine committed object", error)),
    };
    transition(
        broker,
        object_id,
        TransactionState::DeleteIntent,
        TransactionState::Quarantined,
        TransitionUpdate {
            identity: Some(quarantined.identity.clone()),
            quarantine_name: Some(quarantined.private_name.clone()),
            detail: Some(detail.into()),
            ..Default::default()
        },
    )?;
    root.delete_quarantined(&quarantined)
        .map_err(|error| fatal_fs("delete quarantined object", error))?;
    transition(
        broker,
        object_id,
        TransactionState::Quarantined,
        TransactionState::Deleted,
        TransitionUpdate {
            detail: Some(detail.into()),
            ..Default::default()
        },
    )?;
    prove_absent(root, object_id, Some(&object.leaf))?;
    run.objects
        .lock()
        .map_err(|_| poisoned("run objects"))?
        .remove(object_id);
    Ok(())
}
