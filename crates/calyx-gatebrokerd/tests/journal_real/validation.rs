use super::support::*;

#[test]
fn missing_required_transition_metadata_is_rejected_before_sql_mutation() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("metadata.turso");
    let mut journal = Journal::open(&path).unwrap();
    let (run_id, object_id) = seed(&mut journal);
    let before = journal.get(&object_id).unwrap().unwrap().state;
    let result = journal.transition(
        &object_id,
        TransactionState::Intent,
        TransactionState::Prepared,
        TransitionUpdate::default(),
    );
    let after = journal.get(&object_id).unwrap().unwrap().state;
    println!("EDGE missing-identity before={before:?} after={after:?}");
    assert!(result.is_err());
    assert_eq!(before, after);
    let run_before = journal.get_run(&run_id).unwrap().unwrap().state;
    let finish = journal.finish_run(&run_id, RunState::Succeeded, None);
    let run_after = journal.get_run(&run_id).unwrap().unwrap().state;
    println!("EDGE live-object-finish before={run_before:?} after={run_after:?}");
    assert!(finish.is_err());
    assert_eq!(run_before, run_after);
}

#[test]
fn terminal_runs_replay_existing_operations_but_reject_new_authority() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("terminal-run.turso");
    let mut journal = Journal::open(&path).unwrap();
    let run_id = RunId::new("77777777777777777777777777777777").unwrap();
    journal
        .begin_run(&RunIntent {
            run_id: run_id.clone(),
            request_id: request(20),
            run_token: RunToken::new("cd".repeat(32)).unwrap(),
            profile: ProfileName::new("gate").unwrap(),
            owner_uid: 1000,
            owner_pid: 4321,
            owner_starttime: 555_000,
        })
        .unwrap();
    let second_run = journal.begin_run(&RunIntent {
        run_id: RunId::new("99999999999999999999999999999999").unwrap(),
        request_id: request(24),
        run_token: RunToken::new("ef".repeat(32)).unwrap(),
        profile: ProfileName::new("gate").unwrap(),
        owner_uid: 1001,
        owner_pid: 4322,
        owner_starttime: 555_001,
    });
    assert!(second_run.is_err());
    let active_before_finish = journal.list_active_runs().unwrap();
    println!(
        "EDGE single-active-run active_ids={:?}",
        active_before_finish
            .iter()
            .map(|record| record.intent.run_id.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(active_before_finish.len(), 1);
    let replay_id = request(21);
    let replay_intent = operation_intent(
        Request::FinishRun(FinishRunRequest {
            request_id: replay_id.clone(),
            run_id: run_id.clone(),
            run_token: RunToken::new("cd".repeat(32)).unwrap(),
            intended_status: RunStatus::Succeeded,
        }),
        "finish_run",
        Some(run_id.clone()),
    );
    assert!(matches!(
        journal.begin_operation(&replay_intent).unwrap(),
        BeginOperation::Inserted
    ));
    let replay_bytes = encode_response(&ResponseEnvelope {
        version: PROTOCOL_VERSION,
        request_id: replay_id.clone(),
        outcome: ResponseOutcome::Ok(Response::RunFinished {
            run_id: run_id.clone(),
            status: calyx_gatebrokerd::protocol::RunStatus::Succeeded,
        }),
    })
    .unwrap();
    journal
        .finish_operation(&replay_id, OperationState::Succeeded, &replay_bytes, None)
        .unwrap();
    journal
        .finish_run(&run_id, RunState::Succeeded, Some("synthetic terminal run"))
        .unwrap();
    assert!(matches!(
        journal.begin_operation(&replay_intent).unwrap(),
        BeginOperation::Existing(ref record) if record.state == OperationState::Succeeded
    ));

    let new_id = request(22);
    let new_intent = operation_intent(
        Request::FinishRun(FinishRunRequest {
            request_id: new_id.clone(),
            run_id: run_id.clone(),
            run_token: RunToken::new("cd".repeat(32)).unwrap(),
            intended_status: RunStatus::Failed,
        }),
        "finish_run",
        Some(run_id.clone()),
    );
    let new_result = journal.begin_operation(&new_intent);
    let error = new_result.expect_err("terminal run must reject new work");
    println!("EDGE terminal-run-new-operation error={error}");
    assert!(matches!(
        &error,
        calyx_gatebrokerd::journal::JournalError::RunNotActive {
            actual: calyx_gatebrokerd::journal::ObservedRunState::Present(RunState::Succeeded),
            ..
        }
    ));
    assert_eq!(
        error.to_string(),
        format!("operation intent requires active run {run_id}, found succeeded")
    );
    let stage_result = journal.begin_stage(&StageIntent {
        stage_id: StageId::new("88888888888888888888888888888888").unwrap(),
        request_id: request(23),
        run_id: run_id.clone(),
        label: StageLabel::new("late").unwrap(),
        unit: UnitName::new("calyx-gate-late.service").unwrap(),
        slice_unit: UnitName::new("calyx-gate.slice").unwrap(),
        worker_user: "calyxgate-worker".into(),
        worker_uid: 990,
    });
    assert!(stage_result.is_err());
    let independent = Connection::open(&path).unwrap();
    let state: String = independent
        .query_row(
            "SELECT state FROM runs WHERE run_id=?1",
            [run_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let new_operations: i64 = independent
        .query_row(
            "SELECT count(*) FROM operations WHERE request_id=?1",
            [new_id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let stages: i64 = independent
        .query_row("SELECT count(*) FROM stages", [], |row| row.get(0))
        .unwrap();
    println!(
        "SOURCE_OF_TRUTH terminal_run_state={state} rejected_operation_rows={new_operations} stage_rows={stages}"
    );
    assert_eq!(
        (state.as_str(), new_operations, stages),
        ("succeeded", 0, 0)
    );
}
