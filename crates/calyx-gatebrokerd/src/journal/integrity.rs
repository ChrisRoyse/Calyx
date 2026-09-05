use std::path::Path;

use crate::protocol::{ObjectId, RequestId, RunId, StageId};

use super::db::{JournalDb, QueryExecutor, db_params};
use super::support::{
    configure, corrupt_protocol, initialize_schema, preflight_schema,
    prepare_journal_auxiliary_files, prepare_journal_file, query_text_column, sql, sync_file,
    sync_parent, transaction_transition_allowed, validate_journal_file, validate_journal_path,
    validate_schema, validate_stage_event_chain,
};
use super::{JOURNAL_APPLICATION_ID, Journal, JournalError};

impl Journal {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let path = path.as_ref();
        validate_journal_path(path)?;
        let created = prepare_journal_file(path)?;
        let auxiliary_created = prepare_journal_auxiliary_files(path)?;
        if created || auxiliary_created {
            sync_parent(path)?;
        }
        let connection = JournalDb::open(path)?;
        let engine_row = connection
            .query_one("SELECT turso_version()", db_params!()?)
            .map_err(|source| sql("Turso engine handshake", source))?;
        let engine_version: String = engine_row
            .get(0)
            .map_err(|source| sql("decode Turso engine version", source))?;
        if engine_version.is_empty() {
            return Err(JournalError::Durability(format!(
                "Turso engine handshake returned {engine_version:?}"
            )));
        }
        configure(&connection)?;
        // Turso must switch to MVCC before schema/catalog reads. Catalog reads
        // establish a read snapshot, after which changing journal mode fails
        // with `Busy("database is locked")` on a fresh database.
        preflight_schema(&connection)?;
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
        let mode_row = self
            .connection
            .query_one("PRAGMA journal_mode", db_params!()?)
            .map_err(|source| sql("read journal_mode", source))?;
        let mode: String = mode_row
            .get(0)
            .map_err(|source| sql("decode journal_mode", source))?;
        let synchronous_row = self
            .connection
            .query_one("PRAGMA synchronous", db_params!()?)
            .map_err(|source| sql("read synchronous", source))?;
        let synchronous: i64 = synchronous_row
            .get(0)
            .map_err(|source| sql("decode synchronous", source))?;
        let foreign_keys_row = self
            .connection
            .query_one("PRAGMA foreign_keys", db_params!()?)
            .map_err(|source| sql("read foreign_keys", source))?;
        let foreign_keys: i64 = foreign_keys_row
            .get(0)
            .map_err(|source| sql("decode foreign_keys", source))?;
        let application_id_row = self
            .connection
            .query_one("PRAGMA application_id", db_params!()?)
            .map_err(|source| sql("read application_id", source))?;
        let application_id: i64 = application_id_row
            .get(0)
            .map_err(|source| sql("decode application_id", source))?;
        if !mode.eq_ignore_ascii_case("mvcc")
            || synchronous != 2
            || foreign_keys != 1
            || application_id != JOURNAL_APPLICATION_ID
        {
            return Err(JournalError::Durability(format!(
                "expected MVCC/FULL/foreign_keys=ON/application_id={JOURNAL_APPLICATION_ID}; got {mode}/{synchronous}/{foreign_keys}/{application_id}"
            )));
        }
        Ok(())
    }

    pub fn integrity_check(&self) -> Result<(), JournalError> {
        let result_row = self
            .connection
            .query_one("PRAGMA integrity_check(1)", db_params!()?)
            .map_err(|source| sql("integrity_check", source))?;
        let result: String = result_row
            .get(0)
            .map_err(|source| sql("decode integrity_check", source))?;
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
            let id = StageId::new(value).map_err(corrupt_protocol)?;
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
        let checkpoint = self
            .connection
            .query_one("PRAGMA wal_checkpoint(TRUNCATE)", db_params!()?)
            .map_err(|source| sql("WAL checkpoint", source))?;
        let busy: i64 = checkpoint
            .get(0)
            .map_err(|source| sql("decode checkpoint busy", source))?;
        let log: i64 = checkpoint
            .get(1)
            .map_err(|source| sql("decode checkpoint log", source))?;
        let checkpointed: i64 = checkpoint
            .get(2)
            .map_err(|source| sql("decode checkpoint completed", source))?;
        if busy != 0 || log != checkpointed {
            return Err(JournalError::Durability(format!(
                "WAL checkpoint incomplete: busy={busy}, log={log}, checkpointed={checkpointed}"
            )));
        }
        sync_file(&self.path)
    }
}
