use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseEnvelope {
    pub version: u16,
    pub request_id: RequestId,
    pub outcome: ResponseOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "payload", rename_all = "snake_case")]
pub enum ResponseOutcome {
    Ok(Response),
    Error(ErrorResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Response {
    Health {
        healthy: bool,
        broker: BrokerHealth,
        worker: WorkerHealth,
        storage: StorageHealth,
        limits: LimitHealth,
    },
    RunBegun {
        run_id: RunId,
        run_token: RunToken,
    },
    ObjectCreated {
        object_id: ObjectId,
        absolute_path: AbsolutePath,
        root_path: AbsolutePath,
        root_identity: DiagnosticIdentity,
        object_identity: DiagnosticIdentity,
        state: ObjectState,
    },
    StageFinished {
        stage_id: StageId,
        unit: UnitName,
        invocation_id: InvocationId,
        control_group: AbsolutePath,
        exit_status: i32,
    },
    ObjectDeleted {
        object_id: ObjectId,
    },
    RunFinished {
        run_id: RunId,
        status: RunStatus,
    },
    RunAborted {
        run_id: RunId,
    },
    Inspection {
        run: Option<RunInspection>,
        objects: Vec<ObjectInspection>,
        stages: Vec<StageInspection>,
        truncated: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerHealth {
    pub pid: u32,
    pub uid: u32,
    pub unit: UnitName,
    pub cgroup: AbsolutePath,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerHealth {
    pub uid: u32,
    pub gid: u32,
    pub account: ContextValue,
    pub user_manager_absent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageHealth {
    pub database: AbsolutePath,
    pub managed_roots: BTreeMap<RootAlias, ManagedRootHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedRootHealth {
    pub shared: AbsolutePath,
    pub private: AbsolutePath,
    pub root_identity: DiagnosticIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitHealth {
    pub frame_bytes: usize,
    pub argv_entries: usize,
    pub environment_entries: usize,
    pub object_name_bytes: usize,
    pub active_runs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunInspection {
    pub id: RunId,
    pub state: ContextValue,
    pub profile: ProfileName,
    pub owner_uid: u32,
    pub owner_pid: u32,
    pub owner_starttime: u64,
    pub terminal_reason: Option<ContextValue>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectInspection {
    pub id: ObjectId,
    pub run_id: RunId,
    pub role: RoleName,
    pub root_alias: RootAlias,
    pub leaf: LeafName,
    pub path: AbsolutePath,
    pub state: ContextValue,
    pub identity: Option<DiagnosticIdentity>,
    pub quarantine_name: Option<ContextValue>,
    pub error_code: Option<ContextValue>,
    pub detail: Option<ContextValue>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageInspection {
    pub id: StageId,
    pub run_id: RunId,
    pub label: StageLabel,
    pub state: ContextValue,
    pub unit: Option<UnitName>,
    pub invocation_id: Option<InvocationId>,
    pub control_group: Option<AbsolutePath>,
    pub main_pid: Option<u32>,
    pub exit_status: Option<i32>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StableCode {
    InvalidFrame,
    ProtocolVersionMismatch,
    InvalidRequest,
    ConfigInvalid,
    JournalFailure,
    InvalidTransition,
    NotFound,
    Busy,
    OwnerDied,
    RecoveryRequired,
    ObjectCollision,
    ObjectMismatch,
    CapabilityUnavailable,
    PermissionDenied,
    StageFailed,
    SystemFailure,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub code: StableCode,
    pub message: ErrorText,
    pub remediation: ErrorText,
    #[serde(default)]
    pub context: BTreeMap<ContextKey, ContextValue>,
}

pub fn encode_response(response: &ResponseEnvelope) -> Result<Vec<u8>, ProtocolError> {
    if response.version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion {
            expected: PROTOCOL_VERSION,
            actual: response.version,
        });
    }
    let encoded = serde_json::to_vec(response)?;
    if encoded.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::OversizedFrame {
            actual: encoded.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }
    Ok(encoded)
}

pub fn decode_response(frame: &[u8]) -> Result<ResponseEnvelope, ProtocolError> {
    if frame.is_empty() {
        return Err(ProtocolError::EmptyFrame);
    }
    if frame.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::OversizedFrame {
            actual: frame.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }
    let response: ResponseEnvelope = serde_json::from_slice(frame)?;
    if response.version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion {
            expected: PROTOCOL_VERSION,
            actual: response.version,
        });
    }
    Ok(response)
}
