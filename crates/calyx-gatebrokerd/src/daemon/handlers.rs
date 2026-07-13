use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::accounts::{ProcessIdentity, authorize_client, require_owner_ancestry};
use crate::broker_error::{BrokerError, code_name};
use crate::fs_tx::{FsRoot, FsTxError, ObjectIdentity, PreparedObject, PublishedObject};
use crate::ids;
use crate::journal::{
    BeginOperation, IntentRecord, OperationIntent, OperationState, RunIntent, RunState,
    StageIntent, TransactionState, TransitionUpdate,
};
use crate::pidfd::OwnerLease;
use crate::protocol::{
    AbsolutePath, BrokerHealth, ContextValue, DiagnosticIdentity, ExecStageRequest, HealthRequest,
    InspectRequest, LeafName, LimitHealth, MAX_ARGV_ITEMS, MAX_ENV_ITEMS, MAX_FRAME_BYTES,
    ManagedRootHealth, ObjectId, ObjectInspection, ObjectState, PROTOCOL_VERSION, Request,
    RequestEnvelope, RequestId, Response, ResponseEnvelope, ResponseOutcome, RunId, RunInspection,
    RunStatus, RunToken, StableCode, StageInspection, StorageHealth, UnitName, WorkerHealth,
    decode_request, decode_response, encode_response,
};
use crate::systemd::{CapturedStage, StageSpec, SystemdError};
use crate::transport::{PeerCredentials, SeqpacketConnection};

use super::{Broker, LiveObject, LiveStage, RunRuntime, poisoned};

const ABORT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
const INSPECTION_LIMIT: usize = 128;

struct Reply {
    envelope: ResponseEnvelope,
    error: Option<BrokerError>,
}

impl Reply {
    fn success(request_id: RequestId, response: Response) -> Self {
        Self {
            envelope: ResponseEnvelope {
                version: PROTOCOL_VERSION,
                request_id,
                outcome: ResponseOutcome::Ok(response),
            },
            error: None,
        }
    }

    fn failure(request_id: RequestId, error: BrokerError) -> Self {
        Self {
            envelope: error.response(request_id),
            error: Some(error),
        }
    }
}

pub(super) fn serve_connection(broker: &Arc<Broker>, connection: SeqpacketConnection) {
    let generated_id = match ids::request_id() {
        Ok(value) => value,
        Err(error) => {
            BrokerError::system("generate diagnostic request id", error)
                .fatal()
                .log("connection_failed");
            std::process::exit(125);
        }
    };

    let result = serve_one(broker, &connection, &generated_id);
    let reply = match result {
        Ok(reply) => reply,
        Err(error) => Reply::failure(generated_id, error),
    };
    if let Some(error) = &reply.error {
        error.log("request_failed");
    }
    let fatal = reply.error.as_ref().is_some_and(|error| error.fatal);
    match encode_response(&reply.envelope) {
        Ok(encoded) => {
            if let Err(error) = connection.send(&encoded, &[]) {
                BrokerError::system("send response", error).log("response_send_failed");
            }
        }
        Err(error) => {
            BrokerError::system("encode response", error)
                .fatal()
                .log("response_encode_failed");
            std::process::exit(125);
        }
    }
    if fatal {
        broker.fatal.store(true, Ordering::Release);
        std::process::exit(125);
    }
}

fn serve_one(
    broker: &Arc<Broker>,
    connection: &SeqpacketConnection,
    generated_id: &RequestId,
) -> Result<Reply, BrokerError> {
    let peer = connection
        .peer_credentials()
        .map_err(|error| BrokerError::permission(format!("cannot authenticate peer: {error}")))?;
    let identity = authorize_client(peer, broker.client_group_gid, broker.worker.uid)
        .map_err(|error| BrokerError::permission(error.to_string()))?;
    let frame = connection.recv().map_err(|error| {
        BrokerError::new(
            StableCode::InvalidFrame,
            error.to_string(),
            "Send one bounded SOCK_SEQPACKET frame with the exact descriptor contract.",
        )
    })?;
    let envelope =
        decode_request(&frame.bytes).map_err(|error| protocol_error(error.to_string()))?;
    let request_id = envelope.request.request_id().clone();
    if &request_id == generated_id {
        // This is allowed, but intentionally has no special behavior. The
        // generated id is only a response correlation id for malformed frames.
    }
    validate_rights(&envelope, &frame.rights)?;
    dispatch(broker, peer, identity, envelope, &frame.bytes, frame.rights)
}

fn validate_rights(envelope: &RequestEnvelope, rights: &[OwnedFd]) -> Result<(), BrokerError> {
    let expected = usize::from(matches!(envelope.request, Request::ExecStage(_))) * 2;
    if rights.len() != expected {
        return Err(BrokerError::invalid(format!(
            "{} requires exactly {expected} SCM_RIGHTS descriptors; received {}",
            verb_name(&envelope.request),
            rights.len()
        )));
    }
    if expected == 2 {
        super::policy::validate_output_fd(rights[0].as_raw_fd(), "stage stdout")?;
        super::policy::validate_output_fd(rights[1].as_raw_fd(), "stage stderr")?;
    }
    Ok(())
}

fn dispatch(
    broker: &Arc<Broker>,
    peer: PeerCredentials,
    identity: ProcessIdentity,
    envelope: RequestEnvelope,
    frame: &[u8],
    rights: Vec<OwnedFd>,
) -> Result<Reply, BrokerError> {
    match envelope.request {
        Request::Health(request) => health(broker, request),
        Request::BeginRun(request) => {
            require_owner_ancestry(
                peer.pid,
                identity.uid,
                request.owner_pid,
                request.owner_starttime,
            )
            .map_err(|error| BrokerError::permission(error.to_string()))?;
            let request_id = request.request_id.clone();
            durable_operation(broker, frame, "begin_run", None, request_id, true, || {
                begin_run(broker, identity, request)
            })
        }
        Request::CreateObject(request) => {
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token, true)?;
            let request_id = request.request_id.clone();
            let run_id = request.run_id.clone();
            let execution_allowed = run.is_some();
            durable_operation(
                broker,
                frame,
                "create_object",
                Some(run_id),
                request_id,
                execution_allowed,
                || {
                    create_object(
                        broker,
                        require_active_runtime(run, &request.run_id)?,
                        request,
                    )
                },
            )
        }
        Request::ExecStage(request) => {
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token, true)?;
            let request_id = request.request_id.clone();
            let run_id = request.run_id.clone();
            let execution_allowed = run.is_some();
            durable_operation(
                broker,
                frame,
                "exec_stage",
                Some(run_id),
                request_id,
                execution_allowed,
                || {
                    exec_stage(
                        broker,
                        require_active_runtime(run, &request.run_id)?,
                        request,
                        rights,
                    )
                },
            )
        }
        Request::DeleteObject(request) => {
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token, true)?;
            let request_id = request.request_id.clone();
            let run_id = request.run_id.clone();
            let execution_allowed = run.is_some();
            durable_operation(
                broker,
                frame,
                "delete_object",
                Some(run_id),
                request_id,
                execution_allowed,
                || {
                    let run = require_active_runtime(run, &request.run_id)?;
                    let _operation = run
                        .operation
                        .lock()
                        .map_err(|_| poisoned("run operation"))?;
                    ensure_no_stage(&run)?;
                    delete_object(broker, &run, &request.object_id, "explicit delete")?;
                    Ok(Response::ObjectDeleted {
                        object_id: request.object_id,
                    })
                },
            )
        }
        Request::FinishRun(request) => {
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token, true)?;
            let request_id = request.request_id.clone();
            let run_id = request.run_id.clone();
            let execution_allowed = run.is_some();
            durable_operation(
                broker,
                frame,
                "finish_run",
                Some(run_id),
                request_id,
                execution_allowed,
                || {
                    finish_run(
                        broker,
                        require_active_runtime(run, &request.run_id)?,
                        request.run_id,
                        request.intended_status,
                    )
                },
            )
        }
        Request::AbortRun(request) => {
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token, true)?;
            let request_id = request.request_id.clone();
            let run_id = request.run_id.clone();
            let execution_allowed = run.is_some();
            durable_operation(
                broker,
                frame,
                "abort_run",
                Some(run_id),
                request_id,
                execution_allowed,
                || {
                    abort_run_internal(
                        broker,
                        require_active_runtime(run, &request.run_id)?,
                        request.reason.as_str(),
                    )?;
                    Ok(Response::RunAborted {
                        run_id: request.run_id,
                    })
                },
            )
        }
        Request::Inspect(request) => inspect(broker, peer, request),
    }
}

fn durable_operation<F>(
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
    let begun = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .begin_operation(&intent)
        .map_err(|error| BrokerError::journal("begin request operation", error))?;
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

    let outcome = action();
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

fn replay_operation(
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

fn begin_run(
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
        operation: std::sync::Mutex::new(()),
        objects: std::sync::Mutex::new(BTreeMap::new()),
        stage: (std::sync::Mutex::new(None), std::sync::Condvar::new()),
        aborting: std::sync::atomic::AtomicBool::new(false),
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

fn spawn_owner_watcher(broker: Weak<Broker>, run: Arc<RunRuntime>) {
    std::thread::spawn(move || {
        let result = run.owner.wait_for_exit();
        let Some(broker) = broker.upgrade() else {
            return;
        };
        if let Err(error) = result {
            let failure = BrokerError::new(
                StableCode::OwnerDied,
                format!("owner pidfd monitoring failed for run {}: {error}", run.id),
                "Inspect pidfd poll events and drain the recorded stage and objects before restart.",
            )
            .fatal();
            failure.log("owner_monitor_failed");
            broker.fatal.store(true, Ordering::Release);
            std::process::exit(125);
        }
        if let Err(error) = abort_run_internal(&broker, run, "OWNER_DIED") {
            if error.code == StableCode::Busy {
                return;
            }
            error.log("owner_death_cleanup_failed");
            broker.fatal.store(true, Ordering::Release);
            std::process::exit(125);
        }
    });
}

fn authorize_run(
    broker: &Broker,
    peer: PeerCredentials,
    run_id: &RunId,
    token: &RunToken,
    require_active_owner: bool,
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
        if require_active_owner {
            require_owner_ancestry(
                peer.pid,
                run.owner_uid,
                run.owner.pid(),
                run.owner.starttime(),
            )
            .map_err(|error| BrokerError::permission(error.to_string()))?;
            if run
                .owner
                .has_exited()
                .map_err(|error| BrokerError::system("poll run owner", error))?
            {
                return Err(BrokerError::new(
                    StableCode::OwnerDied,
                    format!("run {run_id} owner has exited"),
                    "Wait for automatic abort cleanup and inspect the durable run state.",
                ));
            }
            if run.aborting.load(Ordering::Acquire) {
                return Err(BrokerError::new(
                    StableCode::OwnerDied,
                    format!("run {run_id} is aborting"),
                    "Wait for exact cgroup drain and object cleanup to complete.",
                ));
            }
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

fn require_active_runtime(
    run: Option<Arc<RunRuntime>>,
    run_id: &RunId,
) -> Result<Arc<RunRuntime>, BrokerError> {
    run.ok_or_else(|| {
        BrokerError::new(
            StableCode::InvalidTransition,
            format!("run {run_id} is terminal"),
            "Only an identical completed request may be replayed for a terminal run.",
        )
    })
}

fn create_object(
    broker: &Broker,
    run: Arc<RunRuntime>,
    request: crate::protocol::CreateObjectRequest,
) -> Result<Response, BrokerError> {
    let _operation = run
        .operation
        .lock()
        .map_err(|_| poisoned("run operation"))?;
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

fn reconcile_publish_error(
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

fn discard_prepared(
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

fn fail_clean_intent(
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

fn delete_object(
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

fn finish_run(
    broker: &Broker,
    run: Arc<RunRuntime>,
    run_id: RunId,
    status: RunStatus,
) -> Result<Response, BrokerError> {
    let _operation = run
        .operation
        .lock()
        .map_err(|_| poisoned("run operation"))?;
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
    if run
        .owner
        .has_exited()
        .map_err(|error| BrokerError::system("verify owner before finish", error))?
    {
        return Err(BrokerError::new(
            StableCode::OwnerDied,
            format!("run {run_id} owner exited during finish"),
            "The owner-death watcher will record the terminal aborted state.",
        ));
    }
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .finish_run(&run_id, RunState::from(status), Some("controller finish"))
        .map_err(|error| BrokerError::journal("finish run", error))?;
    remove_runtime(broker, &run);
    Ok(Response::RunFinished { run_id, status })
}

fn abort_run_internal(
    broker: &Broker,
    run: Arc<RunRuntime>,
    reason: &str,
) -> Result<(), BrokerError> {
    {
        let stage = run.stage.0.lock().map_err(|_| poisoned("run stage"))?;
        if run.aborting.swap(true, Ordering::AcqRel) {
            return Err(BrokerError::new(
                StableCode::Busy,
                format!("run {} abort cleanup is already in progress", run.id),
                "Wait for the first abort operation to reach a durable terminal state.",
            ));
        }
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
    let _operation = run
        .operation
        .lock()
        .map_err(|_| poisoned("run operation"))?;
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
    let record = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .get_run(&run.id)
        .map_err(|error| BrokerError::journal("read aborting run", error))?;
    if record
        .as_ref()
        .is_some_and(|value| value.state == RunState::Active)
    {
        broker
            .journal
            .lock()
            .map_err(|_| poisoned("journal"))?
            .finish_run(&run.id, RunState::Aborted, Some(reason))
            .map_err(|error| BrokerError::journal("abort run", error))?;
    }
    remove_runtime(broker, &run);
    Ok(())
}

fn remove_runtime(broker: &Broker, run: &Arc<RunRuntime>) {
    if let Ok(mut runs) = broker.runs.lock() {
        if runs
            .get(&run.id)
            .is_some_and(|current| Arc::ptr_eq(current, run))
        {
            runs.remove(&run.id);
        }
    }
}

fn exec_stage(
    broker: &Broker,
    run: Arc<RunRuntime>,
    request: ExecStageRequest,
    rights: Vec<OwnedFd>,
) -> Result<Response, BrokerError> {
    let _operation = run
        .operation
        .lock()
        .map_err(|_| poisoned("run operation"))?;
    ensure_run_mutable(&run)?;
    ensure_no_stage(&run)?;
    if rights.len() != 2 {
        return Err(BrokerError::invalid(
            "exec_stage lost its two validated output descriptors",
        ));
    }
    let executable = Path::new(request.argv[0].as_str());
    super::policy::require_absolute_regular_executable(executable)?;
    let execution_root = broker
        .execution_roots
        .get(&request.cwd_root)
        .cloned()
        .ok_or_else(|| not_found("execution root", request.cwd_root.as_str()))?;
    let resolved = execution_root.resolve(&request.cwd).map_err(|error| {
        BrokerError::new(
            StableCode::InvalidRequest,
            format!("resolve execution cwd: {error}"),
            "Use a real traversal-free directory beneath the configured execution root.",
        )
    })?;

    let objects: Vec<(ObjectId, LiveObject)> = run
        .objects
        .lock()
        .map_err(|_| poisoned("run objects"))?
        .iter()
        .map(|(id, object)| (id.clone(), object.clone()))
        .collect();
    let mut writable_paths = Vec::with_capacity(objects.len());
    for (object_id, object) in &objects {
        let root = broker
            .roots
            .get(&object.root_alias)
            .ok_or_else(|| not_found("managed root", object.root_alias.as_str()))?;
        prove_published(root, object_id, &object.published)?;
        let configured = broker
            .config
            .root(&object.root_alias)
            .ok_or_else(|| not_found("managed root", object.root_alias.as_str()))?;
        writable_paths.push(configured.raw().shared.join(object.leaf.as_str()));
    }

    let stage_id =
        ids::stage_id().map_err(|error| BrokerError::system("generate stage id", error))?;
    let prefix = &broker.config.raw().unit_prefix;
    let unit_name = format!("{prefix}-{stage_id}.service");
    let slice_name = format!("{prefix}-{stage_id}.slice");
    let spec = StageSpec {
        unit_name,
        slice_name,
        broker_unit: "calyx-gatebrokerd.service".into(),
        worker_user: broker.worker.name.clone(),
        execution_root: execution_root.path().to_path_buf(),
        relative_cwd: PathBuf::from(request.cwd.as_str()),
        execution_root_uid: execution_root.expected_uid(),
        execution_root_mode: execution_root.expected_mode(),
        cwd_fd: resolved.raw_fd(),
        argv: request
            .argv
            .iter()
            .map(|value| OsString::from(value.as_str()))
            .collect(),
        environment: request
            .env
            .iter()
            .map(|entry| {
                (
                    OsString::from(entry.name.as_str()),
                    OsString::from(entry.value.as_str()),
                )
            })
            .collect(),
        writable_paths,
    };
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .begin_stage(&StageIntent {
            stage_id: stage_id.clone(),
            request_id: request.request_id,
            run_id: run.id.clone(),
            label: request.label,
        })
        .map_err(|error| BrokerError::journal("record stage intent", error))?;

    let captured = match CapturedStage::capture(&spec, rights[0].as_raw_fd(), rights[1].as_raw_fd())
    {
        Ok(value) => value,
        Err(error) => {
            broker
                .journal
                .lock()
                .map_err(|_| poisoned("journal"))?
                .fail_stage_intent(&stage_id, 125, &error.to_string())
                .map_err(|journal| BrokerError::journal("fail stage capture", journal))?;
            return Err(systemd_error("capture stage", error));
        }
    };
    let evidence = captured.evidence().clone();
    let unit = UnitName::new(evidence.unit_name.clone())
        .map_err(|error| fatal_system("parse captured unit", error))?;
    let invocation = crate::protocol::InvocationId::new(evidence.invocation_id.clone())
        .map_err(|error| fatal_system("parse captured invocation", error))?;
    let control_group = AbsolutePath::new(evidence.control_group.clone())
        .map_err(|error| fatal_system("parse captured cgroup", error))?;
    let authority = Arc::new(
        captured
            .abort_authority()
            .map_err(|error| systemd_error("duplicate captured authority", error))?,
    );
    broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .mark_stage_running(
            &stage_id,
            &unit,
            &invocation,
            &control_group,
            evidence.main_pid,
        )
        .map_err(|error| BrokerError::journal("publish running stage", error))?;
    {
        let mut slot = run.stage.0.lock().map_err(|_| poisoned("run stage"))?;
        if slot.is_some() {
            return Err(BrokerError::new(
                StableCode::RecoveryRequired,
                format!("run {} acquired two live stage authorities", run.id),
                "Stop the broker and inspect both recorded stage/cgroup identities.",
            )
            .fatal());
        }
        *slot = Some(LiveStage {
            id: stage_id.clone(),
            authority,
        });
    }

    let mut captured = Some(captured);
    let release = {
        let slot = run.stage.0.lock().map_err(|_| poisoned("run stage"))?;
        let live = slot.as_ref().ok_or_else(|| {
            BrokerError::new(
                StableCode::RecoveryRequired,
                format!("live authority for captured stage {stage_id} disappeared"),
                "Inspect abort/launch serialization before restart.",
            )
            .fatal()
        })?;
        if live.id != stage_id {
            return Err(BrokerError::new(
                StableCode::RecoveryRequired,
                format!(
                    "captured stage identity changed: expected={stage_id} actual={}",
                    live.id
                ),
                "Inspect abort/launch serialization before restart.",
            )
            .fatal());
        }
        if run.aborting.load(Ordering::Acquire) {
            None
        } else {
            // The abort transition is serialized by this same mutex. After
            // release starts, an abort can only acquire this exact authority
            // and kill the now-running stage.
            Some(
                captured
                    .take()
                    .expect("captured stage is consumed exactly once")
                    .release(run.owner.raw_fd()),
            )
        }
    };
    let Some(release) = release else {
        drop(captured);
        record_stage_terminal(broker, &run, &stage_id, 125)?;
        return Err(BrokerError::new(
            StableCode::OwnerDied,
            format!(
                "run {} began aborting before stage {stage_id} release",
                run.id
            ),
            "The blocked shim was drained without executing the payload.",
        ));
    };
    let running = match release {
        Ok(value) => value,
        Err(error) => {
            record_stage_terminal(broker, &run, &stage_id, 125)?;
            match run.owner.has_exited() {
                Ok(true) => {
                    return Err(BrokerError::new(
                        StableCode::OwnerDied,
                        format!("owner exited before stage {} release: {error}", stage_id),
                        "Automatic abort will clean every registered object after exact stage drain.",
                    ));
                }
                Ok(false) => {}
                Err(owner_error) => {
                    return Err(fatal_system(
                        "verify owner after failed stage release",
                        owner_error,
                    ));
                }
            }
            return Err(systemd_error("release stage", error));
        }
    };
    let result = match running.wait() {
        Ok(value) => value,
        Err(error) => {
            record_stage_terminal(broker, &run, &stage_id, 125)?;
            return Err(systemd_error("wait for stage drain", error));
        }
    };
    record_stage_terminal(broker, &run, &stage_id, result.exit_status)?;
    Ok(Response::StageFinished {
        stage_id,
        unit,
        invocation_id: invocation,
        control_group,
        exit_status: result.exit_status,
    })
}

fn record_stage_terminal(
    broker: &Broker,
    run: &RunRuntime,
    stage_id: &crate::protocol::StageId,
    exit_status: i32,
) -> Result<(), BrokerError> {
    let journal_result = broker
        .journal
        .lock()
        .map_err(|_| poisoned("journal"))?
        .finish_stage(stage_id, exit_status)
        .map_err(|error| BrokerError::journal("finish stage", error));
    let clear_result = (|| {
        let mut slot = run.stage.0.lock().map_err(|_| poisoned("run stage"))?;
        let live = slot.take().ok_or_else(|| {
            BrokerError::new(
                StableCode::RecoveryRequired,
                format!("live authority for stage {stage_id} disappeared"),
                "Inspect concurrent abort and stage journal ordering before restart.",
            )
            .fatal()
        })?;
        if live.id != *stage_id {
            return Err(BrokerError::new(
                StableCode::RecoveryRequired,
                format!(
                    "live stage identity changed: expected={stage_id} actual={}",
                    live.id
                ),
                "Inspect concurrent stage state before restart.",
            )
            .fatal());
        }
        run.stage.1.notify_all();
        Ok(())
    })();
    journal_result?;
    clear_result
}

fn systemd_error(operation: &str, error: SystemdError) -> BrokerError {
    match error {
        SystemdError::InvalidSpec(detail) => BrokerError::new(
            StableCode::InvalidRequest,
            format!("{operation}: {detail}"),
            "Correct the stage command/environment/path policy and retry with a new request id.",
        ),
        SystemdError::OwnerExited => BrokerError::new(
            StableCode::OwnerDied,
            format!("{operation}: owner exited"),
            "Wait for automatic exact stage/object cleanup.",
        ),
        other => BrokerError::new(
            StableCode::StageFailed,
            format!("{operation}: {other}"),
            "Inspect the recorded unit, invocation, main PID, service/slice cgroup, and cleanup evidence.",
        )
        .fatal(),
    }
}

fn health(broker: &Broker, request: HealthRequest) -> Result<Reply, BrokerError> {
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
                user_manager_absent: worker_manager_absent(broker.worker.uid)?,
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

fn inspect(
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
        (None, None) => broker
            .journal
            .lock()
            .map_err(|_| poisoned("journal"))?
            .list_active_runs()
            .map_err(|error| BrokerError::journal("inspect active runs", error))?
            .into_iter()
            .next(),
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
                unit: record.unit,
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

fn ensure_run_mutable(run: &RunRuntime) -> Result<(), BrokerError> {
    if run.aborting.load(Ordering::Acquire) {
        return Err(BrokerError::new(
            StableCode::OwnerDied,
            format!("run {} is aborting", run.id),
            "Wait for exact process drain and object cleanup.",
        ));
    }
    if run
        .owner
        .has_exited()
        .map_err(|error| BrokerError::system("poll run owner", error))?
    {
        return Err(BrokerError::new(
            StableCode::OwnerDied,
            format!("run {} owner has exited", run.id),
            "Wait for automatic abort cleanup.",
        ));
    }
    Ok(())
}

fn ensure_no_stage(run: &RunRuntime) -> Result<(), BrokerError> {
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

fn require_namespace_absent(
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

fn prove_published(
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

fn prove_absent(
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

fn transition(
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

fn worker_manager_absent(uid: u32) -> Result<bool, BrokerError> {
    let output = std::process::Command::new("systemctl")
        .args([
            "show",
            &format!("user@{uid}.service"),
            "--property=ActiveState",
            "--value",
        ])
        .output()
        .map_err(|error| BrokerError::system("inspect worker user manager", error))?;
    if !output.status.success() {
        return Err(BrokerError::system(
            "inspect worker user manager",
            format!(
                "status={} stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let state = String::from_utf8(output.stdout)
        .map_err(|error| BrokerError::system("decode worker user manager state", error))?;
    Ok(state.trim() != "active")
}

fn protocol_error(message: String) -> BrokerError {
    BrokerError::new(
        StableCode::InvalidFrame,
        message,
        "Send a canonical protocol-v1 JSON frame within the published bounds.",
    )
}

fn fs_error(operation: &str, error: FsTxError, fatal: bool) -> BrokerError {
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

fn fatal_fs(operation: &str, error: impl std::fmt::Display) -> BrokerError {
    BrokerError::new(
        StableCode::RecoveryRequired,
        format!("{operation}: {error}"),
        "Stop mutation and reconcile the SQLite row with opaque filesystem identity evidence.",
    )
    .fatal()
}

fn fatal_system(operation: &str, error: impl std::fmt::Display) -> BrokerError {
    BrokerError::new(
        StableCode::RecoveryRequired,
        format!("{operation}: {error}"),
        "Stop mutation and inspect the recorded unit, pidfd, and held cgroup descriptors.",
    )
    .fatal()
}

fn not_found(kind: &str, value: &str) -> BrokerError {
    BrokerError::new(
        StableCode::NotFound,
        format!("{kind} {value:?} was not found"),
        "Inspect the durable run and object state and use the exact recorded identifier.",
    )
}

fn constant_time_eq(first: &str, second: &str) -> bool {
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

fn absolute_path(path: &Path) -> Result<AbsolutePath, BrokerError> {
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

fn diagnostic(identity: &ObjectIdentity) -> DiagnosticIdentity {
    DiagnosticIdentity {
        device: identity.device,
        inode: identity.inode,
    }
}

fn context(value: &str) -> Result<ContextValue, BrokerError> {
    ContextValue::new(value)
        .map_err(|error| BrokerError::system("construct inspection state", error))
}

fn optional_context(value: Option<String>) -> Result<Option<ContextValue>, BrokerError> {
    value
        .filter(|value| !value.is_empty())
        .map(|value| context(&value))
        .transpose()
}

fn run_state_name(state: RunState) -> &'static str {
    match state {
        RunState::Active => "active",
        RunState::Succeeded => "succeeded",
        RunState::Failed => "failed",
        RunState::Aborted => "aborted",
    }
}

fn transaction_state_name(state: TransactionState) -> &'static str {
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

fn stage_state_name(state: crate::journal::StageState) -> &'static str {
    match state {
        crate::journal::StageState::Intent => "intent",
        crate::journal::StageState::Running => "running",
        crate::journal::StageState::Succeeded => "succeeded",
        crate::journal::StageState::Failed => "failed",
    }
}

fn verb_name(request: &Request) -> &'static str {
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
