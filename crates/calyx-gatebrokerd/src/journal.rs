use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fs_tx::{ObjectIdentity, OpaqueHandle};
use crate::protocol::{
    LeafName, ObjectId, ProfileName, RequestId, RoleName, RootAlias, RunId, RunStatus, RunToken,
};

mod support;
use support::*;

mod stage;
pub use stage::*;

mod operation;
pub use operation::*;

const JOURNAL_SCHEMA_VERSION: i64 = 3;
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
    connection: Connection,
}

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("journal path is invalid: {0}")]
    InvalidPath(String),
    #[error("SQLite operation {operation} failed: {source}")]
    Sql {
        operation: &'static str,
        #[source]
        source: rusqlite::Error,
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
    #[error("filesystem durability operation failed for {path}: {source}")]
    FileSync {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl Journal {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let path = path.as_ref();
        validate_journal_path(path)?;
        let created = prepare_journal_file(path)?;
        if created {
            sync_parent(path)?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX;
        let connection =
            Connection::open_with_flags(path, flags).map_err(|source| sql("open", source))?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|source| sql("set busy timeout", source))?;
        preflight_schema(&connection)?;
        configure(&connection)?;
        initialize_schema(&connection)?;
        validate_journal_file(path)?;
        let journal = Self {
            path: path.to_path_buf(),
            connection,
        };
        journal.verify_durability_settings()?;
        journal.integrity_check()?;
        Ok(journal)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn verify_durability_settings(&self) -> Result<(), JournalError> {
        let mode: String = self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(|source| sql("read journal_mode", source))?;
        let synchronous: i64 = self
            .connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .map_err(|source| sql("read synchronous", source))?;
        let foreign_keys: i64 = self
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .map_err(|source| sql("read foreign_keys", source))?;
        let trusted_schema: i64 = self
            .connection
            .query_row("PRAGMA trusted_schema", [], |row| row.get(0))
            .map_err(|source| sql("read trusted_schema", source))?;
        let fullfsync: i64 = self
            .connection
            .query_row("PRAGMA fullfsync", [], |row| row.get(0))
            .map_err(|source| sql("read fullfsync", source))?;
        let checkpoint_fullfsync: i64 = self
            .connection
            .query_row("PRAGMA checkpoint_fullfsync", [], |row| row.get(0))
            .map_err(|source| sql("read checkpoint_fullfsync", source))?;
        let application_id: i64 = self
            .connection
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .map_err(|source| sql("read application_id", source))?;
        if !mode.eq_ignore_ascii_case("wal")
            || synchronous != 2
            || foreign_keys != 1
            || trusted_schema != 0
            || fullfsync != 1
            || checkpoint_fullfsync != 1
            || application_id != JOURNAL_APPLICATION_ID
        {
            return Err(JournalError::Durability(format!(
                "expected WAL/FULL/foreign_keys=ON/trusted_schema=OFF/fullfsync=ON/checkpoint_fullfsync=ON/application_id={JOURNAL_APPLICATION_ID}; got {mode}/{synchronous}/{foreign_keys}/{trusted_schema}/{fullfsync}/{checkpoint_fullfsync}/{application_id}"
            )));
        }
        Ok(())
    }

    pub fn integrity_check(&self) -> Result<(), JournalError> {
        let result: String = self
            .connection
            .query_row("PRAGMA integrity_check(1)", [], |row| row.get(0))
            .map_err(|source| sql("integrity_check", source))?;
        if result != "ok" {
            return Err(JournalError::Corrupt(format!(
                "integrity_check returned {result:?}"
            )));
        }
        validate_schema(&self.connection)?;
        self.semantic_integrity_check()
    }

    fn semantic_integrity_check(&self) -> Result<(), JournalError> {
        for value in query_text_column(
            &self.connection,
            "SELECT run_id FROM runs ORDER BY run_id",
            "scan run ids",
        )? {
            let id = RunId::new(value).map_err(corrupt_protocol)?;
            self.get_run(&id)?
                .ok_or_else(|| JournalError::Corrupt(format!("run {id} disappeared")))?;
        }
        for value in query_text_column(
            &self.connection,
            "SELECT object_id FROM object_transactions ORDER BY object_id",
            "scan object ids",
        )? {
            let id = ObjectId::new(value).map_err(corrupt_protocol)?;
            let record = self
                .get(&id)?
                .ok_or_else(|| JournalError::Corrupt(format!("object {id} disappeared")))?;
            let events = self.events(&id)?;
            let mut previous = None;
            for event in &events {
                if event.from_state != previous {
                    return Err(JournalError::Corrupt(format!(
                        "object {id} event {} starts at {:?}, expected {:?}",
                        event.event_id, event.from_state, previous
                    )));
                }
                if let Some(from) = event.from_state
                    && !transaction_transition_allowed(from, event.to_state)
                {
                    return Err(JournalError::Corrupt(format!(
                        "object {id} event {} records forbidden transition {from:?} -> {:?}",
                        event.event_id, event.to_state
                    )));
                }
                previous = Some(event.to_state);
            }
            if previous != Some(record.state) {
                return Err(JournalError::Corrupt(format!(
                    "object {id} event chain ends at {previous:?}, row is {:?}",
                    record.state
                )));
            }
        }
        for value in query_text_column(
            &self.connection,
            "SELECT stage_id FROM stages ORDER BY stage_id",
            "scan stage ids",
        )? {
            let id = crate::protocol::StageId::new(value).map_err(corrupt_protocol)?;
            let record = self
                .get_stage(&id)?
                .ok_or_else(|| JournalError::Corrupt(format!("stage {id} disappeared")))?;
            validate_stage_event_chain(&self.connection, &record)?;
        }
        for value in query_text_column(
            &self.connection,
            "SELECT request_id FROM operations ORDER BY request_id",
            "scan operation ids",
        )? {
            let id = RequestId::new(value).map_err(corrupt_protocol)?;
            self.get_operation(&id)?
                .ok_or_else(|| JournalError::Corrupt(format!("operation {id} disappeared")))?;
        }
        Ok(())
    }

    pub fn checkpoint(&self) -> Result<(), JournalError> {
        let (busy, log, checkpointed): (i64, i64, i64) = self
            .connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|source| sql("WAL checkpoint", source))?;
        if busy != 0 || log != checkpointed {
            return Err(JournalError::Durability(format!(
                "WAL checkpoint incomplete: busy={busy}, log={log}, checkpointed={checkpointed}"
            )));
        }
        sync_file(&self.path)
    }

    pub fn begin_run(&mut self, intent: &RunIntent) -> Result<(), JournalError> {
        if intent.owner_pid == 0
            || intent.owner_pid > i32::MAX as u32
            || intent.owner_starttime == 0
        {
            return Err(JournalError::InvalidMetadata(
                "run owner identity must be nonzero".into(),
            ));
        }
        let now = now_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| sql("begin run transaction", source))?;
        transaction.execute(
            "INSERT INTO runs(run_id,request_id,run_token,profile,owner_uid,owner_pid,owner_starttime,state,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,'active',?8,?8)",
            params![intent.run_id.as_str(), intent.request_id.as_str(), intent.run_token.as_str(), intent.profile.as_str(), intent.owner_uid, intent.owner_pid, intent.owner_starttime.to_string(), now],
        ).map_err(|source| sql("insert run", source))?;
        transaction
            .commit()
            .map_err(|source| sql("commit run", source))
    }

    pub fn finish_run(
        &mut self,
        run_id: &RunId,
        next: RunState,
        detail: Option<&str>,
    ) -> Result<(), JournalError> {
        if next == RunState::Active {
            return Err(JournalError::InvalidMetadata(
                "finished run cannot remain active".into(),
            ));
        }
        validate_optional_text("run detail", detail, 2_048)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| sql("begin finish run transaction", source))?;
        let unfinished_stages: i64 = transaction
            .query_row(
                "SELECT count(*) FROM stages WHERE run_id=?1 AND state IN ('intent','running')",
                [run_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|source| sql("count unfinished run stages", source))?;
        let live_objects: i64 = transaction
            .query_row(
                "SELECT count(*) FROM object_transactions WHERE run_id=?1 AND state NOT IN ('deleted','failed')",
                [run_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|source| sql("count live run objects", source))?;
        let failed_work: i64 = if next == RunState::Succeeded {
            transaction
                .query_row(
                    "SELECT (SELECT count(*) FROM stages WHERE run_id=?1 AND state='failed') + (SELECT count(*) FROM object_transactions WHERE run_id=?1 AND state='failed')",
                    [run_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|source| sql("count failed run work", source))?
        } else {
            0
        };
        if unfinished_stages != 0 || live_objects != 0 || failed_work != 0 {
            return Err(JournalError::InvalidMetadata(format!(
                "run {run_id} cannot finish with unfinished_stages={unfinished_stages}, live_objects={live_objects}, failed_work={failed_work}"
            )));
        }
        let changed = transaction.execute(
            "UPDATE runs SET state=?1, detail=?2, updated_ms=?3 WHERE run_id=?4 AND state='active'",
            params![next.as_db(), detail, now_ms()?, run_id.as_str()],
        ).map_err(|source| sql("finish run", source))?;
        if changed != 1 {
            return Err(JournalError::InvalidMetadata(format!(
                "run {run_id} is absent or not active"
            )));
        }
        transaction
            .commit()
            .map_err(|source| sql("commit finished run", source))
    }

    pub fn get_run(&self, run_id: &RunId) -> Result<Option<RunRecord>, JournalError> {
        self.connection.query_row(
            "SELECT request_id,run_token,profile,owner_uid,owner_pid,owner_starttime,state,created_ms,updated_ms,detail FROM runs WHERE run_id=?1",
            [run_id.as_str()],
            |row| run_from_row(run_id.clone(), row),
        ).optional().map_err(|source| sql("read run", source))?.transpose()
    }

    pub fn get_run_by_request(
        &self,
        request_id: &RequestId,
    ) -> Result<Option<RunRecord>, JournalError> {
        let value = self
            .connection
            .query_row(
                "SELECT run_id FROM runs WHERE request_id=?1",
                [request_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|source| sql("find run by request", source))?;
        value
            .map(|value| RunId::new(value).map_err(corrupt_protocol))
            .transpose()?
            .map(|run_id| self.get_run(&run_id))
            .transpose()
            .map(Option::flatten)
    }

    pub fn list_active_runs(&self) -> Result<Vec<RunRecord>, JournalError> {
        let mut statement = self
            .connection
            .prepare("SELECT run_id FROM runs WHERE state='active' ORDER BY created_ms,run_id")
            .map_err(|source| sql("prepare active run query", source))?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| sql("query active runs", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| sql("read active run ids", source))?;
        ids.into_iter()
            .map(|value| {
                let id =
                    RunId::new(value).map_err(|error| JournalError::Corrupt(error.to_string()))?;
                self.get_run(&id)?
                    .ok_or_else(|| JournalError::Corrupt(format!("active run {id} disappeared")))
            })
            .collect()
    }

    pub fn begin_intent(&mut self, intent: &IntentRecord) -> Result<(), JournalError> {
        let now = now_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| sql("begin object intent", source))?;
        require_active_run(&transaction, &intent.run_id, "object intent")?;
        transaction.execute(
            "INSERT INTO object_transactions(object_id,request_id,run_id,role,root_alias,leaf,state,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,?6,'intent',?7,?7)",
            params![intent.object_id.as_str(), intent.request_id.as_str(), intent.run_id.as_str(), intent.role.as_str(), intent.root_alias.as_str(), intent.leaf.as_str(), now],
        ).map_err(|source| sql("insert object intent", source))?;
        transaction
            .execute(
                "INSERT INTO journal_events(object_id,to_state,at_ms) VALUES(?1,'intent',?2)",
                params![intent.object_id.as_str(), now],
            )
            .map_err(|source| sql("insert intent event", source))?;
        transaction
            .commit()
            .map_err(|source| sql("commit object intent", source))
    }

    pub fn transition(
        &mut self,
        object_id: &ObjectId,
        expected: TransactionState,
        next: TransactionState,
        update: TransitionUpdate,
    ) -> Result<(), JournalError> {
        validate_transition(expected, next, &update)?;
        let now = now_ms()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| sql("begin state transition", source))?;
        let current = read_transaction(&transaction, object_id)?
            .ok_or_else(|| JournalError::NotFound(object_id.to_string()))?;
        if current.state != expected {
            return Err(JournalError::InvalidTransition {
                object_id: object_id.to_string(),
                expected,
                actual: current.state,
                next,
            });
        }
        if let (Some(current), Some(replacement)) =
            (current.identity.as_ref(), update.identity.as_ref())
            && (!current.same_authority(replacement)
                || current.device != replacement.device
                || current.inode != replacement.inode)
        {
            return Err(JournalError::InvalidMetadata(format!(
                "object {object_id} identity authority cannot change during a journal transition"
            )));
        }
        if let (Some(current), Some(replacement)) = (
            current.quarantine_name.as_deref(),
            update.quarantine_name.as_deref(),
        ) && current != replacement
        {
            return Err(JournalError::InvalidMetadata(format!(
                "object {object_id} quarantine name is immutable"
            )));
        }
        if let Some(name) = update.quarantine_name.as_deref()
            && name != format!("q-{object_id}")
        {
            return Err(JournalError::InvalidMetadata(format!(
                "object {object_id} quarantine name must be q-{object_id}"
            )));
        }
        let identity = update.identity.as_ref().or(current.identity.as_ref());
        let fields = identity.map(identity_fields);
        let changed = transaction.execute(
            "UPDATE object_transactions SET state=?1,device=?2,inode=?3,owner_uid=?4,owner_gid=?5,mode=?6,mount_id=?7,handle_type=?8,handle=?9,quarantine_name=COALESCE(?10,quarantine_name),error_code=?11,detail=?12,updated_ms=?13 WHERE object_id=?14 AND state=?15",
            params![next.as_db(), fields.as_ref().map(|v| v.0.as_str()), fields.as_ref().map(|v| v.1.as_str()), fields.as_ref().map(|v| v.2), fields.as_ref().map(|v| v.3), fields.as_ref().map(|v| v.4), fields.as_ref().map(|v| v.5), fields.as_ref().map(|v| v.6), fields.as_ref().map(|v| v.7.as_slice()), update.quarantine_name, update.error_code, update.detail, now, object_id.as_str(), expected.as_db()],
        ).map_err(|source| sql("update transaction state", source))?;
        if changed != 1 {
            return Err(JournalError::Durability(
                "compare-and-set transition changed no rows".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO journal_events(object_id,from_state,to_state,error_code,detail,at_ms) VALUES(?1,?2,?3,?4,?5,?6)",
            params![object_id.as_str(), expected.as_db(), next.as_db(), update.error_code, update.detail, now],
        ).map_err(|source| sql("insert transition event", source))?;
        transaction
            .commit()
            .map_err(|source| sql("commit state transition", source))
    }

    pub fn get(&self, object_id: &ObjectId) -> Result<Option<TransactionRecord>, JournalError> {
        read_transaction(&self.connection, object_id)
    }

    pub fn by_request(
        &self,
        request_id: &RequestId,
    ) -> Result<Option<TransactionRecord>, JournalError> {
        let object: Option<String> = self
            .connection
            .query_row(
                "SELECT object_id FROM object_transactions WHERE request_id=?1",
                [request_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| sql("find transaction by request", source))?;
        object
            .map(|value| {
                ObjectId::new(value).map_err(|error| JournalError::Corrupt(error.to_string()))
            })
            .transpose()?
            .map(|id| self.get(&id))
            .transpose()
            .map(Option::flatten)
    }

    pub fn list_incomplete(&self) -> Result<Vec<TransactionRecord>, JournalError> {
        let mut statement = self.connection.prepare(
            "SELECT object_id FROM object_transactions WHERE state IN ('intent','prepared','published','delete_intent','quarantined') ORDER BY created_ms,object_id",
        ).map_err(|source| sql("prepare incomplete query", source))?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| sql("query incomplete transactions", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| sql("read incomplete transaction ids", source))?;
        ids.into_iter()
            .map(|id| {
                let id =
                    ObjectId::new(id).map_err(|error| JournalError::Corrupt(error.to_string()))?;
                self.get(&id)?
                    .ok_or_else(|| JournalError::NotFound(id.to_string()))
            })
            .collect()
    }

    pub fn list_objects_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Vec<TransactionRecord>, JournalError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT object_id FROM object_transactions WHERE run_id=?1 ORDER BY created_ms,object_id",
            )
            .map_err(|source| sql("prepare run object query", source))?;
        let ids = statement
            .query_map([run_id.as_str()], |row| row.get::<_, String>(0))
            .map_err(|source| sql("query run objects", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| sql("read run object ids", source))?;
        ids.into_iter()
            .map(|value| {
                let id = ObjectId::new(value).map_err(corrupt_protocol)?;
                self.get(&id)?
                    .ok_or_else(|| JournalError::Corrupt(format!("run object {id} disappeared")))
            })
            .collect()
    }

    pub fn events(&self, object_id: &ObjectId) -> Result<Vec<JournalEvent>, JournalError> {
        let mut statement = self.connection.prepare(
            "SELECT event_id,from_state,to_state,error_code,detail,at_ms FROM journal_events WHERE object_id=?1 ORDER BY event_id",
        ).map_err(|source| sql("prepare event query", source))?;
        let rows = statement
            .query_map([object_id.as_str()], |row| {
                Ok((
                    row.get(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .map_err(|source| sql("query events", source))?;
        rows.map(|row| {
            let (event_id, from, to, error_code, detail, at_ms) =
                row.map_err(|source| sql("read event", source))?;
            Ok(JournalEvent {
                event_id,
                object_id: object_id.clone(),
                from_state: from.map(|v| TransactionState::parse(&v)).transpose()?,
                to_state: TransactionState::parse(&to)?,
                error_code,
                detail,
                at_ms,
            })
        })
        .collect()
    }
}
