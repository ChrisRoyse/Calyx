use super::db::{DbRow, QueryExecutor, db_params};
use super::*;
use crate::protocol::{
    Request, RequestId, ResponseOutcome, RunId, decode_request, decode_response,
};

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
    pub request_json: Vec<u8>,
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
        validate_operation_request(intent).map_err(JournalError::InvalidMetadata)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| sql("begin operation transaction", source))?;
        if let Some(existing) = read_operation(&transaction, &intent.request_id)? {
            if existing.intent.request_hash != intent.request_hash
                || existing.intent.request_json != intent.request_json
                || existing.intent.verb != intent.verb
                || existing.intent.run_id != intent.run_id
            {
                return Err(JournalError::RequestConflict {
                    request_id: intent.request_id.clone(),
                });
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
                "INSERT INTO operations(request_id,request_hash,request_json,verb,run_id,state,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,'pending',?6,?6)",
                db_params![
                    intent.request_id.as_str(),
                    intent.request_hash.as_slice(),
                    intent.request_json.as_slice(),
                    intent.verb.as_str(),
                    intent.run_id.as_ref().map(RunId::as_str),
                    now
                ]?,
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
        let finished_at = now_ms()?;
        let changed = self
            .connection
            .execute(
                "UPDATE operations SET state=?1,response_json=?2,error_code=?3,updated_ms=?4 WHERE request_id=?5 AND state='pending'",
                db_params![state.as_db(), response_json, error_code, finished_at, request_id.as_str()]?,
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
        let ids = self
            .connection
            .query(
                "SELECT request_id FROM operations WHERE state='pending' ORDER BY created_ms,request_id",
                db_params!()?,
            )
            .map_err(|source| sql("query pending operations", source))?
            .into_iter()
            .map(|row| row.get::<String>(0).map_err(|source| sql("read pending operation id", source)))
            .collect::<Result<Vec<_>, _>>()?;
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

fn operation_from_row(request_id: RequestId, row: &DbRow) -> Result<OperationRecord, JournalError> {
    let hash: Vec<u8> = row.get(0).map_err(corrupt_sql)?;
    let request_json: Vec<u8> = row.get(1).map_err(corrupt_sql)?;
    let run_id = row
        .get::<Option<String>>(3)
        .map_err(corrupt_sql)?
        .map(RunId::new)
        .transpose();
    let state: String = row.get(4).map_err(corrupt_sql)?;
    (|| {
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
                request_json,
                verb: row.get(2).map_err(corrupt_sql)?,
                run_id: run_id.map_err(corrupt_protocol)?,
            },
            state: OperationState::parse(&state)?,
            response_json: row.get(5).map_err(corrupt_sql)?,
            error_code: row.get(6).map_err(corrupt_sql)?,
            created_ms: row.get(7).map_err(corrupt_sql)?,
            updated_ms: row.get(8).map_err(corrupt_sql)?,
        };
        validate_operation_verb(&record.intent.verb)
            .map_err(|error| JournalError::Corrupt(error.to_string()))?;
        validate_operation_request(&record.intent).map_err(JournalError::Corrupt)?;
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
    })()
}

fn read_operation(
    connection: &impl QueryExecutor,
    request_id: &RequestId,
) -> Result<Option<OperationRecord>, JournalError> {
    connection
        .query_optional(
            "SELECT request_hash,request_json,verb,run_id,state,response_json,error_code,created_ms,updated_ms FROM operations WHERE request_id=?1",
            db_params![request_id.as_str()]?,
        )
        .map_err(|source| sql("read operation", source))?
        .map(|row| operation_from_row(request_id.clone(), &row))
        .transpose()
}

fn validate_operation_request(intent: &OperationIntent) -> Result<(), String> {
    if intent.request_json.is_empty()
        || intent.request_json.len() > crate::protocol::MAX_FRAME_BYTES
    {
        return Err("operation request is empty or oversized".into());
    }
    if blake3::hash(&intent.request_json).as_bytes() != &intent.request_hash {
        return Err("operation request bytes do not match the recorded hash".into());
    }
    let envelope = decode_request(&intent.request_json)
        .map_err(|error| format!("operation request is not a valid protocol request: {error}"))?;
    if envelope.request.request_id() != &intent.request_id {
        return Err(format!(
            "operation request id {} does not match {}",
            envelope.request.request_id(),
            intent.request_id
        ));
    }
    let (verb, run_id) = operation_authority(&envelope.request)?;
    if verb != intent.verb || run_id != intent.run_id.as_ref() {
        return Err(format!(
            "operation request authority does not match: expected verb={} run={:?}, actual verb={verb} run={run_id:?}",
            intent.verb,
            intent.run_id.as_ref().map(RunId::as_str)
        ));
    }
    Ok(())
}

fn operation_authority(request: &Request) -> Result<(&'static str, Option<&RunId>), String> {
    match request {
        Request::BeginRun(_) => Ok(("begin_run", None)),
        Request::CreateObject(request) => Ok(("create_object", Some(&request.run_id))),
        Request::ExecStage(request) => Ok(("exec_stage", Some(&request.run_id))),
        Request::DeleteObject(request) => Ok(("delete_object", Some(&request.run_id))),
        Request::FinishRun(request) => Ok(("finish_run", Some(&request.run_id))),
        Request::AbortRun(request) => Ok(("abort_run", Some(&request.run_id))),
        Request::Health(_) | Request::Inspect(_) => {
            Err("health and inspect requests are not durable mutations".into())
        }
    }
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
