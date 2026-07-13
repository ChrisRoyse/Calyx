use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::*;
use crate::protocol::{RequestId, ResponseOutcome, RunId, decode_response};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationState {
    Pending,
    Succeeded,
    Failed,
}

impl OperationState {
    fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, JournalError> {
        match value {
            "pending" => Ok(Self::Pending),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            _ => Err(JournalError::Corrupt(format!(
                "unknown operation state {value:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OperationIntent {
    pub request_id: RequestId,
    pub request_hash: [u8; 32],
    pub verb: String,
    pub run_id: Option<RunId>,
}

#[derive(Debug, Clone)]
pub struct OperationRecord {
    pub intent: OperationIntent,
    pub state: OperationState,
    pub response_json: Option<Vec<u8>>,
    pub error_code: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone)]
pub enum BeginOperation {
    Inserted,
    Existing(OperationRecord),
}

impl Journal {
    pub fn begin_operation(
        &mut self,
        intent: &OperationIntent,
    ) -> Result<BeginOperation, JournalError> {
        validate_operation_verb(&intent.verb)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| sql("begin operation transaction", source))?;
        if let Some(existing) = read_operation(&transaction, &intent.request_id)? {
            if existing.intent.request_hash != intent.request_hash
                || existing.intent.verb != intent.verb
                || existing.intent.run_id != intent.run_id
            {
                return Err(JournalError::InvalidMetadata(format!(
                    "request id {} was reused with different bytes or authority",
                    intent.request_id
                )));
            }
            transaction
                .commit()
                .map_err(|source| sql("commit existing operation read", source))?;
            return Ok(BeginOperation::Existing(existing));
        }
        if let Some(run_id) = intent.run_id.as_ref() {
            require_active_run(&transaction, run_id, "operation intent")?;
        }
        let now = now_ms()?;
        transaction
            .execute(
                "INSERT INTO operations(request_id,request_hash,verb,run_id,state,created_ms,updated_ms) VALUES(?1,?2,?3,?4,'pending',?5,?5)",
                params![
                    intent.request_id.as_str(),
                    intent.request_hash.as_slice(),
                    intent.verb,
                    intent.run_id.as_ref().map(RunId::as_str),
                    now
                ],
            )
            .map_err(|source| sql("insert operation intent", source))?;
        transaction
            .commit()
            .map_err(|source| sql("commit operation intent", source))?;
        Ok(BeginOperation::Inserted)
    }

    pub fn finish_operation(
        &mut self,
        request_id: &RequestId,
        state: OperationState,
        response_json: &[u8],
        error_code: Option<&str>,
    ) -> Result<(), JournalError> {
        if state == OperationState::Pending {
            return Err(JournalError::InvalidMetadata(
                "terminal operation cannot remain pending".into(),
            ));
        }
        if response_json.is_empty() || response_json.len() > crate::protocol::MAX_FRAME_BYTES {
            return Err(JournalError::InvalidMetadata(
                "operation response is empty or oversized".into(),
            ));
        }
        validate_terminal_operation(request_id, state, response_json, error_code)
            .map_err(JournalError::InvalidMetadata)?;
        let changed = self
            .connection
            .execute(
                "UPDATE operations SET state=?1,response_json=?2,error_code=?3,updated_ms=?4 WHERE request_id=?5 AND state='pending'",
                params![state.as_db(), response_json, error_code, now_ms()?, request_id.as_str()],
            )
            .map_err(|source| sql("finish operation", source))?;
        if changed != 1 {
            return Err(JournalError::InvalidMetadata(format!(
                "operation {request_id} is absent or not pending"
            )));
        }
        Ok(())
    }

    pub fn get_operation(
        &self,
        request_id: &RequestId,
    ) -> Result<Option<OperationRecord>, JournalError> {
        read_operation(&self.connection, request_id)
    }

    pub fn list_pending_operations(&self) -> Result<Vec<OperationRecord>, JournalError> {
        let mut statement = self
            .connection
            .prepare("SELECT request_id FROM operations WHERE state='pending' ORDER BY created_ms,request_id")
            .map_err(|source| sql("prepare pending operation query", source))?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| sql("query pending operations", source))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| sql("read pending operation ids", source))?;
        ids.into_iter()
            .map(|value| {
                let id = RequestId::new(value).map_err(corrupt_protocol)?;
                self.get_operation(&id)?.ok_or_else(|| {
                    JournalError::Corrupt(format!("pending operation {id} disappeared"))
                })
            })
            .collect()
    }
}

fn operation_from_row(
    request_id: RequestId,
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<OperationRecord, JournalError>> {
    let hash: Vec<u8> = row.get(0)?;
    let run_id = row.get::<_, Option<String>>(2)?.map(RunId::new).transpose();
    let state: String = row.get(3)?;
    Ok((|| {
        let request_hash: [u8; 32] = hash.try_into().map_err(|value: Vec<u8>| {
            JournalError::Corrupt(format!(
                "operation request hash has length {}, expected 32",
                value.len()
            ))
        })?;
        let record = OperationRecord {
            intent: OperationIntent {
                request_id: request_id.clone(),
                request_hash,
                verb: row.get(1).map_err(corrupt_sql)?,
                run_id: run_id.map_err(corrupt_protocol)?,
            },
            state: OperationState::parse(&state)?,
            response_json: row.get(4).map_err(corrupt_sql)?,
            error_code: row.get(5).map_err(corrupt_sql)?,
            created_ms: row.get(6).map_err(corrupt_sql)?,
            updated_ms: row.get(7).map_err(corrupt_sql)?,
        };
        validate_operation_verb(&record.intent.verb)
            .map_err(|error| JournalError::Corrupt(error.to_string()))?;
        match record.state {
            OperationState::Pending => {
                if record.response_json.is_some() || record.error_code.is_some() {
                    return Err(JournalError::Corrupt(format!(
                        "pending operation {request_id} contains terminal metadata"
                    )));
                }
            }
            state => validate_terminal_operation(
                &request_id,
                state,
                record.response_json.as_deref().unwrap_or_default(),
                record.error_code.as_deref(),
            )
            .map_err(JournalError::Corrupt)?,
        }
        Ok(record)
    })())
}

fn read_operation(
    connection: &rusqlite::Connection,
    request_id: &RequestId,
) -> Result<Option<OperationRecord>, JournalError> {
    connection
        .query_row(
            "SELECT request_hash,verb,run_id,state,response_json,error_code,created_ms,updated_ms FROM operations WHERE request_id=?1",
            [request_id.as_str()],
            |row| operation_from_row(request_id.clone(), row),
        )
        .optional()
        .map_err(|source| sql("read operation", source))?
        .transpose()
}

fn validate_operation_verb(verb: &str) -> Result<(), JournalError> {
    validate_optional_text("operation verb", Some(verb), 64)?;
    if !verb
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(JournalError::InvalidMetadata(
            "operation verb must contain only lowercase ASCII letters and underscores".into(),
        ));
    }
    Ok(())
}

fn validate_terminal_operation(
    request_id: &RequestId,
    state: OperationState,
    response_json: &[u8],
    error_code: Option<&str>,
) -> Result<(), String> {
    if state == OperationState::Pending {
        return Err("terminal operation cannot remain pending".into());
    }
    if response_json.is_empty() || response_json.len() > crate::protocol::MAX_FRAME_BYTES {
        return Err("operation response is empty or oversized".into());
    }
    validate_optional_text("operation error code", error_code, 96)
        .map_err(|error| error.to_string())?;
    let response = decode_response(response_json)
        .map_err(|error| format!("operation response is not a valid protocol response: {error}"))?;
    if response.request_id != *request_id {
        return Err(format!(
            "operation response request id {} does not match {request_id}",
            response.request_id
        ));
    }
    match (state, &response.outcome, error_code) {
        (OperationState::Succeeded, ResponseOutcome::Ok(_), None)
        | (OperationState::Failed, ResponseOutcome::Error(_), Some(_)) => Ok(()),
        (OperationState::Succeeded, _, _) => {
            Err("succeeded operation requires an ok response and no error code".into())
        }
        (OperationState::Failed, _, _) => {
            Err("failed operation requires an error response and an error code".into())
        }
        (OperationState::Pending, _, _) => unreachable!("pending rejected above"),
    }
}
