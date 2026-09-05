use super::support::*;

#[test]
fn operation_idempotence_is_atomic_and_terminal_responses_are_replayable() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("operations.turso");
    let mut journal = Journal::open(&path).unwrap();
    let (run_id, _) = seed(&mut journal);
    let success_id = request(10);
    let success = operation_intent(
        Request::AbortRun(AbortRunRequest {
            request_id: success_id.clone(),
            run_id: run_id.clone(),
            run_token: RunToken::new("ab".repeat(32)).unwrap(),
            reason: ReasonText::new("synthetic abort").unwrap(),
        }),
        "abort_run",
        Some(run_id.clone()),
    );
    assert!(matches!(
        journal.begin_operation(&success).unwrap(),
        BeginOperation::Inserted
    ));
    let duplicate = journal.begin_operation(&success).unwrap();
    assert!(matches!(
        duplicate,
        BeginOperation::Existing(ref record) if record.state == OperationState::Pending
    ));

    let before: (String, Option<Vec<u8>>) = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT state,response_json FROM operations WHERE request_id=?1",
            [success_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    println!("EDGE operation-reuse before={before:?}");
    let changed = operation_intent(
        Request::AbortRun(AbortRunRequest {
            request_id: success_id.clone(),
            run_id: run_id.clone(),
            run_token: RunToken::new("ab".repeat(32)).unwrap(),
            reason: ReasonText::new("different valid request bytes").unwrap(),
        }),
        "abort_run",
        Some(run_id.clone()),
    );
    assert!(matches!(
        journal.begin_operation(&changed),
        Err(calyx_gatebrokerd::journal::JournalError::RequestConflict { .. })
    ));
    let after: (String, Option<Vec<u8>>) = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT state,response_json FROM operations WHERE request_id=?1",
            [success_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    println!("EDGE operation-reuse after={after:?}");
    assert_eq!(before, after);

    let success_bytes = encode_response(&ResponseEnvelope {
        version: PROTOCOL_VERSION,
        request_id: success_id.clone(),
        outcome: ResponseOutcome::Ok(Response::RunAborted {
            run_id: run_id.clone(),
        }),
    })
    .unwrap();
    journal
        .finish_operation(&success_id, OperationState::Succeeded, &success_bytes, None)
        .unwrap();

    let failure_id = request(11);
    let failure = operation_intent(
        Request::AbortRun(AbortRunRequest {
            request_id: failure_id.clone(),
            run_id: run_id.clone(),
            run_token: RunToken::new("ab".repeat(32)).unwrap(),
            reason: ReasonText::new("synthetic failed abort response").unwrap(),
        }),
        "abort_run",
        Some(run_id),
    );
    assert!(matches!(
        journal.begin_operation(&failure).unwrap(),
        BeginOperation::Inserted
    ));
    let failure_bytes = encode_response(&ResponseEnvelope {
        version: PROTOCOL_VERSION,
        request_id: failure_id.clone(),
        outcome: ResponseOutcome::Error(ErrorResponse {
            code: StableCode::SystemFailure,
            message: ErrorText::new("synthetic stage failure").unwrap(),
            remediation: ErrorText::new("inspect the synthetic failure").unwrap(),
            context: Default::default(),
        }),
    })
    .unwrap();
    journal
        .finish_operation(
            &failure_id,
            OperationState::Failed,
            &failure_bytes,
            Some("SYSTEM_FAILURE"),
        )
        .unwrap();
    journal.checkpoint().unwrap();
    drop(journal);

    let reopened = Journal::open(&path).unwrap();
    let success_record = reopened.get_operation(&success_id).unwrap().unwrap();
    assert_eq!(success_record.state, OperationState::Succeeded);
    assert_eq!(
        success_record.response_json.as_deref(),
        Some(success_bytes.as_slice())
    );
    let failure_record = reopened.get_operation(&failure_id).unwrap().unwrap();
    assert_eq!(failure_record.state, OperationState::Failed);
    assert_eq!(failure_record.error_code.as_deref(), Some("SYSTEM_FAILURE"));
    let independent = Connection::open(&path).unwrap();
    let rows = independent
        .prepare("SELECT request_id,state,length(request_hash),length(request_json),length(response_json),error_code FROM operations ORDER BY request_id")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    println!("SOURCE_OF_TRUTH operations={rows:?}");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.2 == 32 && row.3 > 0 && row.4 > 0));
}

#[test]
fn operation_request_bytes_are_hash_bound_on_independent_reopen() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("operation-request-corruption.turso");
    let mut journal = Journal::open(&path).unwrap();
    let run_id = RunId::new("91919191919191919191919191919191").unwrap();
    journal
        .begin_run(&RunIntent {
            run_id: run_id.clone(),
            request_id: request(90),
            run_token: RunToken::new("91".repeat(32)).unwrap(),
            profile: ProfileName::new("gate").unwrap(),
            owner_uid: 1000,
            owner_pid: 4321,
            owner_starttime: 919_191,
        })
        .unwrap();
    let operation = operation_intent(
        Request::AbortRun(AbortRunRequest {
            request_id: request(91),
            run_id: run_id.clone(),
            run_token: RunToken::new("91".repeat(32)).unwrap(),
            reason: ReasonText::new("hash-bound request proof").unwrap(),
        }),
        "abort_run",
        Some(run_id),
    );
    journal.begin_operation(&operation).unwrap();
    journal.checkpoint().unwrap();
    drop(journal);

    let independent = Connection::open(&path).unwrap();
    let before: (i64, i64) = independent
        .query_row(
            "SELECT length(request_hash),length(request_json) FROM operations WHERE request_id=?1",
            [operation.request_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    println!("EDGE operation-request-corruption before={before:?}");
    independent
        .execute(
            "UPDATE operations SET request_json=x'7b7d' WHERE request_id=?1",
            [operation.request_id.as_str()],
        )
        .unwrap();
    let after: (i64, i64) = independent
        .query_row(
            "SELECT length(request_hash),length(request_json) FROM operations WHERE request_id=?1",
            [operation.request_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    println!("EDGE operation-request-corruption after={after:?}");
    assert_eq!(before.0, 32);
    assert_eq!(after, (32, 2));
    drop(independent);

    let error = match Journal::open(&path) {
        Ok(_) => panic!("hash/request mismatch must fail closed"),
        Err(error) => error,
    };
    println!("SOURCE_OF_TRUTH operation-request-corruption error={error}");
    assert!(error.to_string().contains("do not match the recorded hash"));
}
