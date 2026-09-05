use super::*;

pub(super) fn ensure_run_mutable(run: &RunRuntime) -> Result<(), BrokerError> {
    if run.lifecycle.abort_signal.load(Ordering::Acquire) {
        return Err(BrokerError::new(
            StableCode::OwnerDied,
            format!("run {} is aborting", run.id),
            "Wait for exact process drain and object cleanup.",
        ));
    }
    if poll_owner(run, "verify mutable run")? {
        return Err(BrokerError::new(
            StableCode::OwnerDied,
            format!("run {} owner has exited", run.id),
            "Wait for automatic abort cleanup.",
        ));
    }
    Ok(())
}

pub(super) fn ensure_no_stage(run: &RunRuntime) -> Result<(), BrokerError> {
    if run
        .stage
        .0
        .lock()
        .map_err(|_| poisoned("run stage"))?
        .is_some()
    {
        return Err(BrokerError::new(
            StableCode::Busy,
            format!("run {} has an active stage", run.id),
            "Wait for the exact stage cgroup to drain before mutating object lifetime.",
        ));
    }
    Ok(())
}

pub(super) fn require_namespace_absent(
    root: &FsRoot,
    object_id: &ObjectId,
    leaf: &LeafName,
) -> Result<(), BrokerError> {
    let shared = root
        .inspect_shared(leaf)
        .map_err(|error| fs_error("preflight shared namespace", error, false))?;
    let prepared = root
        .inspect_private(&format!("p-{object_id}"))
        .map_err(|error| fs_error("preflight prepared namespace", error, false))?;
    let quarantined = root
        .inspect_private(&format!("q-{object_id}"))
        .map_err(|error| fs_error("preflight quarantine namespace", error, false))?;
    if shared.is_some() || prepared.is_some() || quarantined.is_some() {
        return Err(BrokerError::new(
            StableCode::ObjectCollision,
            format!(
                "namespace is occupied: shared={} prepared={} quarantined={}",
                shared.is_some(),
                prepared.is_some(),
                quarantined.is_some()
            ),
            "Choose a unique leaf; existing entries are never adopted or replaced.",
        ));
    }
    Ok(())
}

pub(super) fn prove_published(
    root: &FsRoot,
    object_id: &ObjectId,
    published: &PublishedObject,
) -> Result<(), BrokerError> {
    let shared = root
        .inspect_shared(&published.leaf)
        .map_err(|error| fatal_fs("read back published object", error))?;
    let prepared = root
        .inspect_private(&format!("p-{object_id}"))
        .map_err(|error| fatal_fs("read back prepared namespace", error))?;
    let quarantined = root
        .inspect_private(&format!("q-{object_id}"))
        .map_err(|error| fatal_fs("read back quarantine namespace", error))?;
    if prepared.is_some()
        || quarantined.is_some()
        || !shared
            .as_ref()
            .is_some_and(|value| value.same_authority(&published.identity))
    {
        return Err(BrokerError::new(
            StableCode::RecoveryRequired,
            format!(
                "published source-of-truth verification failed for {object_id}: shared_match={} prepared={} quarantined={}",
                shared
                    .as_ref()
                    .is_some_and(|value| value.same_authority(&published.identity)),
                prepared.is_some(),
                quarantined.is_some()
            ),
            "Inspect the durable object row and both namespaces before continuing.",
        )
        .fatal());
    }
    Ok(())
}

pub(super) fn prove_absent(
    root: &FsRoot,
    object_id: &ObjectId,
    leaf: Option<&LeafName>,
) -> Result<(), BrokerError> {
    let shared = leaf
        .map(|leaf| root.inspect_shared(leaf))
        .transpose()
        .map_err(|error| fatal_fs("read back deleted shared object", error))?
        .flatten();
    let prepared = root
        .inspect_private(&format!("p-{object_id}"))
        .map_err(|error| fatal_fs("read back deleted prepared object", error))?;
    let quarantined = root
        .inspect_private(&format!("q-{object_id}"))
        .map_err(|error| fatal_fs("read back deleted quarantined object", error))?;
    if shared.is_some() || prepared.is_some() || quarantined.is_some() {
        return Err(BrokerError::new(
            StableCode::RecoveryRequired,
            format!(
                "delete source-of-truth verification failed for {object_id}: shared={} prepared={} quarantined={}",
                shared.is_some(),
                prepared.is_some(),
                quarantined.is_some()
            ),
            "Do not continue; inspect the durable row and preserved namespace entry.",
        )
        .fatal());
    }
    Ok(())
}

pub(super) fn transition(
    broker: &Broker,
    object_id: &ObjectId,
    from: TransactionState,
    to: TransactionState,
    update: TransitionUpdate,
) -> Result<(), BrokerError> {
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .transition(object_id, from, to, update)
        .map_err(|error| BrokerError::journal("object transition", error))
}

pub(super) fn protocol_error(message: String) -> BrokerError {
    BrokerError::new(
        StableCode::InvalidFrame,
        message,
        "Send a canonical protocol-v1 JSON frame within the published bounds.",
    )
}

pub(super) fn fs_error(operation: &str, error: FsTxError, fatal: bool) -> BrokerError {
    let code = match error {
        FsTxError::Collision(_) => StableCode::ObjectCollision,
        FsTxError::IdentityMismatch { .. } => StableCode::ObjectMismatch,
        FsTxError::CapabilityUnavailable { .. } => StableCode::CapabilityUnavailable,
        _ => StableCode::SystemFailure,
    };
    let value = BrokerError::new(
        code,
        format!("{operation}: {error}"),
        "Inspect the exact filesystem operation, journal row, and both managed namespaces.",
    );
    if fatal { value.fatal() } else { value }
}

pub(super) fn fatal_fs(operation: &str, error: impl std::fmt::Display) -> BrokerError {
    BrokerError::new(
        StableCode::RecoveryRequired,
        format!("{operation}: {error}"),
        "Stop mutation and reconcile the Turso row with opaque filesystem identity evidence.",
    )
    .fatal()
}

pub(super) fn fatal_system(operation: &str, error: impl std::fmt::Display) -> BrokerError {
    BrokerError::new(
        StableCode::RecoveryRequired,
        format!("{operation}: {error}"),
        "Stop mutation and inspect the recorded unit, pidfd, and held cgroup descriptors.",
    )
    .fatal()
}

pub(super) fn not_found(kind: &str, value: &str) -> BrokerError {
    BrokerError::new(
        StableCode::NotFound,
        format!("{kind} {value:?} was not found"),
        "Inspect the durable run and object state and use the exact recorded identifier.",
    )
}

pub(super) fn constant_time_eq(first: &str, second: &str) -> bool {
    if first.len() != second.len() {
        return false;
    }
    first
        .bytes()
        .zip(second.bytes())
        .fold(0_u8, |difference, (first, second)| {
            difference | (first ^ second)
        })
        == 0
}

pub(super) fn absolute_path(path: &Path) -> Result<AbsolutePath, BrokerError> {
    let value = path.to_str().ok_or_else(|| {
        BrokerError::new(
            StableCode::ConfigInvalid,
            format!("path is not UTF-8: {}", path.display()),
            "Use normalized UTF-8 absolute paths in broker configuration.",
        )
    })?;
    AbsolutePath::new(value)
        .map_err(|error| BrokerError::system("construct absolute path response", error))
}

pub(super) fn diagnostic(identity: &ObjectIdentity) -> DiagnosticIdentity {
    DiagnosticIdentity {
        device: identity.device,
        inode: identity.inode,
    }
}

pub(super) fn context(value: &str) -> Result<ContextValue, BrokerError> {
    ContextValue::new(value)
        .map_err(|error| BrokerError::system("construct inspection state", error))
}

pub(super) fn optional_context(value: Option<String>) -> Result<Option<ContextValue>, BrokerError> {
    value
        .filter(|value| !value.is_empty())
        .map(|value| context(&value))
        .transpose()
}

pub(super) fn run_state_name(state: RunState) -> &'static str {
    match state {
        RunState::Active => "active",
        RunState::Succeeded => "succeeded",
        RunState::Failed => "failed",
        RunState::Aborted => "aborted",
    }
}

pub(super) fn transaction_state_name(state: TransactionState) -> &'static str {
    match state {
        TransactionState::Intent => "intent",
        TransactionState::Prepared => "prepared",
        TransactionState::Published => "published",
        TransactionState::Committed => "committed",
        TransactionState::DeleteIntent => "delete_intent",
        TransactionState::Quarantined => "quarantined",
        TransactionState::Deleted => "deleted",
        TransactionState::MismatchPreserved => "mismatch_preserved",
        TransactionState::Failed => "failed",
    }
}

pub(super) fn stage_state_name(state: crate::journal::StageState) -> &'static str {
    match state {
        crate::journal::StageState::Intent => "intent",
        crate::journal::StageState::Running => "running",
        crate::journal::StageState::Succeeded => "succeeded",
        crate::journal::StageState::Failed => "failed",
    }
}

pub(super) fn verb_name(request: &Request) -> &'static str {
    match request {
        Request::Health(_) => "health",
        Request::BeginRun(_) => "begin_run",
        Request::CreateObject(_) => "create_object",
        Request::ExecStage(_) => "exec_stage",
        Request::DeleteObject(_) => "delete_object",
        Request::FinishRun(_) => "finish_run",
        Request::AbortRun(_) => "abort_run",
        Request::Inspect(_) => "inspect",
    }
}
