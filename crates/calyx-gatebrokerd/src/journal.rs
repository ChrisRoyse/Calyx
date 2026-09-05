use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fs_tx::ObjectIdentity;
use crate::protocol::{
    LeafName, ObjectId, ProfileName, RequestId, RoleName, RootAlias, RunId, RunStatus, RunToken,
};

mod support;
use support::*;
mod db;
use db::JournalDb;

mod integrity;
mod object;
mod run;

mod stage;
pub use stage::*;

mod operation;
pub use operation::*;

const JOURNAL_SCHEMA_VERSION: i64 = 4;
const JOURNAL_APPLICATION_ID: i64 = 0x4359_4742;

#[derive(Debug, Clone)]
pub struct RunIntent {
    pub run_id: RunId,
    pub request_id: RequestId,
    pub run_token: RunToken,
    pub profile: ProfileName,
    pub owner_uid: u32,
    pub owner_pid: u32,
    pub owner_starttime: u64,
}

#[derive(Debug, Clone)]
pub struct RunRecord {
    pub intent: RunIntent,
    pub state: RunState,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Active,
    Succeeded,
    Failed,
    Aborted,
}

impl RunState {
    fn as_db(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }

    fn parse(value: &str) -> Result<Self, JournalError> {
        match value {
            "active" => Ok(Self::Active),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "aborted" => Ok(Self::Aborted),
            _ => Err(JournalError::Corrupt(format!(
                "unknown run state {value:?}"
            ))),
        }
    }
}

impl From<RunStatus> for RunState {
    fn from(value: RunStatus) -> Self {
        match value {
            RunStatus::Succeeded => Self::Succeeded,
            RunStatus::Failed => Self::Failed,
            RunStatus::Aborted => Self::Aborted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedRunState {
    Absent,
    Present(RunState),
}

impl std::fmt::Display for ObservedRunState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent => formatter.write_str("absent"),
            Self::Present(state) => write!(formatter, "{}", state.as_db()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct IntentRecord {
    pub object_id: ObjectId,
    pub request_id: RequestId,
    pub run_id: RunId,
    pub role: RoleName,
    pub root_alias: RootAlias,
    pub leaf: LeafName,
}

#[derive(Debug, Clone)]
pub struct TransactionRecord {
    pub intent: IntentRecord,
    pub state: TransactionState,
    pub identity: Option<ObjectIdentity>,
    pub quarantine_name: Option<String>,
    pub error_code: Option<String>,
    pub detail: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Intent,
    Prepared,
    Published,
    Committed,
    DeleteIntent,
    Quarantined,
    Deleted,
    MismatchPreserved,
    Failed,
}

impl TransactionState {
    pub fn is_recovery_required(self) -> bool {
        matches!(
            self,
            Self::Intent
                | Self::Prepared
                | Self::Published
                | Self::DeleteIntent
                | Self::Quarantined
        )
    }

    fn as_db(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Prepared => "prepared",
            Self::Published => "published",
            Self::Committed => "committed",
            Self::DeleteIntent => "delete_intent",
            Self::Quarantined => "quarantined",
            Self::Deleted => "deleted",
            Self::MismatchPreserved => "mismatch_preserved",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, JournalError> {
        match value {
            "intent" => Ok(Self::Intent),
            "prepared" => Ok(Self::Prepared),
            "published" => Ok(Self::Published),
            "committed" => Ok(Self::Committed),
            "delete_intent" => Ok(Self::DeleteIntent),
            "quarantined" => Ok(Self::Quarantined),
            "deleted" => Ok(Self::Deleted),
            "mismatch_preserved" => Ok(Self::MismatchPreserved),
            "failed" => Ok(Self::Failed),
            _ => Err(JournalError::Corrupt(format!(
                "unknown transaction state {value:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TransitionUpdate {
    pub identity: Option<ObjectIdentity>,
    pub quarantine_name: Option<String>,
    pub error_code: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JournalEvent {
    pub event_id: i64,
    pub object_id: ObjectId,
    pub from_state: Option<TransactionState>,
    pub to_state: TransactionState,
    pub error_code: Option<String>,
    pub detail: Option<String>,
    pub at_ms: i64,
}

pub struct Journal {
    path: PathBuf,
    connection: JournalDb,
}

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("journal path is invalid: {0}")]
    InvalidPath(String),
    #[error("Turso operation {operation} failed: {source}")]
    Sql {
        operation: &'static str,
        #[source]
        source: turso::Error,
    },
    #[error("journal durability invariant failed: {0}")]
    Durability(String),
    #[error("journal data is corrupt: {0}")]
    Corrupt(String),
    #[error("object transaction {0} was not found")]
    NotFound(String),
    #[error(
        "invalid transition for {object_id}: expected {expected:?}, actual {actual:?}, requested {next:?}"
    )]
    InvalidTransition {
        object_id: String,
        expected: TransactionState,
        actual: TransactionState,
        next: TransactionState,
    },
    #[error("invalid transition metadata: {0}")]
    InvalidMetadata(String),
    #[error("request id {request_id} was reused with different bytes or authority")]
    RequestConflict { request_id: RequestId },
    #[error("{operation} requires active run {run_id}, found {actual}")]
    RunNotActive {
        operation: &'static str,
        run_id: RunId,
        actual: ObservedRunState,
    },
    #[error(
        "run {run_id} cannot finish with unfinished_stages={unfinished_stages}, live_objects={live_objects}, failed_work={failed_work}"
    )]
    RunUndrained {
        run_id: RunId,
        unfinished_stages: i64,
        live_objects: i64,
        failed_work: i64,
    },
    #[error("filesystem durability operation failed for {path}: {source}")]
    FileSync {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<turso::Error> for JournalError {
    fn from(source: turso::Error) -> Self {
        Self::Sql {
            operation: "bind Turso parameters",
            source,
        }
    }
}
