use crate::protocol::{RequestId, RunId};

use super::db::{QueryExecutor, db_params};
use super::support::{corrupt_protocol, now_ms, run_from_row, sql, validate_optional_text};
use super::{Journal, JournalError, RunIntent, RunRecord, RunState};

impl Journal {
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
            .transaction()
            .map_err(|source| sql("begin run transaction", source))?;
        transaction.execute(
            "INSERT INTO runs(run_id,request_id,run_token,profile,owner_uid,owner_pid,owner_starttime,state,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,'active',?8,?8)",
            db_params![intent.run_id.as_str(), intent.request_id.as_str(), intent.run_token.as_str(), intent.profile.as_str(), intent.owner_uid, intent.owner_pid, intent.owner_starttime.to_string(), now]?,
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
            .transaction()
            .map_err(|source| sql("begin finish run transaction", source))?;
        let unfinished_stages = transaction
            .query_one(
                "SELECT count(*) FROM stages WHERE run_id=?1 AND state IN ('intent','running')",
                db_params![run_id.as_str()]?,
            )
            .map_err(|source| sql("count unfinished run stages", source))?;
        let unfinished_stages: i64 = unfinished_stages
            .get(0)
            .map_err(|source| sql("decode unfinished run stages", source))?;
        let live_objects = transaction
            .query_one(
                "SELECT count(*) FROM object_transactions WHERE run_id=?1 AND state NOT IN ('deleted','failed')",
                db_params![run_id.as_str()]?,
            )
            .map_err(|source| sql("count live run objects", source))?;
        let live_objects: i64 = live_objects
            .get(0)
            .map_err(|source| sql("decode live run objects", source))?;
        let failed_work: i64 = if next == RunState::Succeeded {
            let row = transaction
                .query_one(
                    "SELECT (SELECT count(*) FROM stages WHERE run_id=?1 AND state='failed') + (SELECT count(*) FROM object_transactions WHERE run_id=?1 AND state='failed')",
                    db_params![run_id.as_str()]?,
                )
                .map_err(|source| sql("count failed run work", source))?;
            row.get(0)
                .map_err(|source| sql("decode failed run work", source))?
        } else {
            0
        };
        if unfinished_stages != 0 || live_objects != 0 || failed_work != 0 {
            return Err(JournalError::RunUndrained {
                run_id: run_id.clone(),
                unfinished_stages,
                live_objects,
                failed_work,
            });
        }
        let finished_at = now_ms()?;
        let changed = transaction.execute(
            "UPDATE runs SET state=?1, detail=?2, updated_ms=?3 WHERE run_id=?4 AND state='active'",
            db_params![next.as_db(), detail, finished_at, run_id.as_str()]?,
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
        self.connection.query_optional(
            "SELECT request_id,run_token,profile,owner_uid,owner_pid,owner_starttime,state,created_ms,updated_ms,detail FROM runs WHERE run_id=?1",
            db_params![run_id.as_str()]?,
        ).map_err(|source| sql("read run", source))?
            .map(|row| run_from_row(run_id.clone(), &row))
            .transpose()
    }

    pub fn get_run_by_request(
        &self,
        request_id: &RequestId,
    ) -> Result<Option<RunRecord>, JournalError> {
        let value_row = self
            .connection
            .query_optional(
                "SELECT run_id FROM runs WHERE request_id=?1",
                db_params![request_id.as_str()]?,
            )
            .map_err(|source| sql("find run by request", source))?;
        let value = value_row
            .map(|row| {
                row.get::<String>(0)
                    .map_err(|source| sql("decode run by request", source))
            })
            .transpose()?;
        value
            .map(|value| RunId::new(value).map_err(corrupt_protocol))
            .transpose()?
            .map(|run_id| self.get_run(&run_id))
            .transpose()
            .map(Option::flatten)
    }

    pub fn list_active_runs(&self) -> Result<Vec<RunRecord>, JournalError> {
        let ids = self
            .connection
            .query(
                "SELECT run_id FROM runs WHERE state='active' ORDER BY created_ms,run_id",
                db_params!()?,
            )
            .map_err(|source| sql("query active runs", source))?
            .into_iter()
            .map(|row| {
                row.get::<String>(0)
                    .map_err(|source| sql("read active run id", source))
            })
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|value| {
                let id =
                    RunId::new(value).map_err(|error| JournalError::Corrupt(error.to_string()))?;
                self.get_run(&id)?
                    .ok_or_else(|| JournalError::Corrupt(format!("active run {id} disappeared")))
            })
            .collect()
    }
}
