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
    BeginOperation, IntentRecord, JournalError, OperationIntent, OperationState,
    RecordedCgroupIdentity, RunIntent, RunState, StageIntent, StageRunningEvidence,
    TransactionState, TransitionUpdate,
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
use crate::systemd::{CapturedStage, STAGE_SLICE_NAME, StageSpec, SystemdError};
use crate::transport::{PeerCredentials, SeqpacketConnection};

use super::{Broker, LiveObject, LiveStage, RunLifecycle, RunLifecycleState, RunRuntime, poisoned};
mod inspection;
mod lifecycle;
mod objects;
mod operations;
mod run;
mod stage;
mod support;

use inspection::*;
use lifecycle::*;
use objects::*;
use operations::*;
use run::*;
use stage::*;
use support::*;

const ABORT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
const INSPECTION_LIMIT: usize = 128;

struct Reply {
    envelope: ResponseEnvelope,
    error: Option<BrokerError>,
}

struct RunOperation<'a> {
    peer: PeerCredentials,
    frame: &'a [u8],
    verb: &'static str,
    run_id: RunId,
    run_token: RunToken,
    request_id: RequestId,
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
    if let Err(error) = validate_rights(&envelope, &frame.rights) {
        return Ok(Reply::failure(request_id, error));
    }
    match dispatch(broker, peer, identity, envelope, &frame.bytes, frame.rights) {
        Ok(reply) => Ok(reply),
        Err(error) => Ok(Reply::failure(request_id, error)),
    }
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
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token)?;
            let request_id = request.request_id.clone();
            let run_id = request.run_id.clone();
            let run_token = request.run_token.clone();
            durable_run_operation(
                broker,
                run,
                RunOperation {
                    peer,
                    frame,
                    verb: "create_object",
                    run_id,
                    run_token,
                    request_id,
                },
                |run| create_object(broker, run, request),
            )
        }
        Request::ExecStage(request) => {
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token)?;
            let request_id = request.request_id.clone();
            let run_id = request.run_id.clone();
            let run_token = request.run_token.clone();
            durable_run_operation(
                broker,
                run,
                RunOperation {
                    peer,
                    frame,
                    verb: "exec_stage",
                    run_id,
                    run_token,
                    request_id,
                },
                |run| exec_stage(broker, run, request, rights),
            )
        }
        Request::DeleteObject(request) => {
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token)?;
            let request_id = request.request_id.clone();
            let run_id = request.run_id.clone();
            let run_token = request.run_token.clone();
            durable_run_operation(
                broker,
                run,
                RunOperation {
                    peer,
                    frame,
                    verb: "delete_object",
                    run_id,
                    run_token,
                    request_id,
                },
                |run| {
                    ensure_run_mutable(&run)?;
                    ensure_no_stage(&run)?;
                    delete_object(broker, &run, &request.object_id, "explicit delete")?;
                    Ok(Response::ObjectDeleted {
                        object_id: request.object_id,
                    })
                },
            )
        }
        Request::FinishRun(request) => {
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token)?;
            let request_id = request.request_id.clone();
            let run_id = request.run_id.clone();
            let run_token = request.run_token.clone();
            durable_run_operation(
                broker,
                run,
                RunOperation {
                    peer,
                    frame,
                    verb: "finish_run",
                    run_id,
                    run_token,
                    request_id,
                },
                |run| finish_run(broker, run, request.run_id, request.intended_status),
            )
        }
        Request::AbortRun(request) => {
            let run = authorize_run(broker, peer, &request.run_id, &request.run_token)?;
            durable_abort_operation(broker, peer, frame, run, request)
        }
        Request::Inspect(request) => inspect(broker, peer, request),
    }
}
