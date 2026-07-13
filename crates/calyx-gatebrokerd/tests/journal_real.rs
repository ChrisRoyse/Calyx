use calyx_gatebrokerd::fs_tx::{MAX_OPAQUE_HANDLE_BYTES, ObjectIdentity, OpaqueHandle};
use calyx_gatebrokerd::journal::{
    BeginOperation, IntentRecord, Journal, OperationIntent, OperationState, RecordedCgroupIdentity,
    RunIntent, RunState, StageIntent, StageState, TransactionState, TransitionUpdate,
};
use calyx_gatebrokerd::protocol::{
    AbsolutePath, ErrorResponse, ErrorText, InvocationId, LeafName, ObjectId, PROTOCOL_VERSION,
    ProfileName, RequestId, Response, ResponseEnvelope, ResponseOutcome, RoleName, RootAlias,
    RunId, RunToken, StableCode, StageId, StageLabel, UnitName, encode_response,
};
use rusqlite::Connection;
use tempfile::TempDir;

fn request(value: u128) -> RequestId {
    let raw = format!("{value:032x}");
    RequestId::new(format!(
        "{}-{}-{}-{}-{}",
        &raw[0..8],
        &raw[8..12],
        &raw[12..16],
        &raw[16..20],
        &raw[20..32]
    ))
    .expect("request UUID")
}

fn identity(byte: u8) -> ObjectIdentity {
    ObjectIdentity {
        device: 42,
        inode: 9001,
        owner_uid: 1001,
        owner_gid: 1002,
        mode: 0o700,
        opaque: OpaqueHandle::new(7, 1, vec![byte; MAX_OPAQUE_HANDLE_BYTES])
            .expect("bounded handle"),
    }
}

#[cfg(unix)]
fn assert_live_journal_files(path: &std::path::Path) {
    use std::os::unix::fs::MetadataExt;

    let database = path.metadata().expect("physical SQLite database");
    let wal = path.with_file_name(format!(
        "{}-wal",
        path.file_name().unwrap().to_string_lossy()
    ));
    let wal_metadata = wal.metadata().expect("physical SQLite WAL");
    let effective_uid = unsafe { libc::geteuid() };
    println!(
        "SOURCE_OF_TRUTH database={} uid={} mode={:04o} nlink={} bytes={} wal={} wal_uid={} wal_mode={:04o} wal_nlink={} wal_bytes={}",
        path.display(),
        database.uid(),
        database.mode() & 0o7777,
        database.nlink(),
        database.len(),
        wal.display(),
        wal_metadata.uid(),
        wal_metadata.mode() & 0o7777,
        wal_metadata.nlink(),
        wal_metadata.len(),
    );
    assert_eq!(
        (database.uid(), database.mode() & 0o777, database.nlink()),
        (effective_uid, 0o600, 1)
    );
    assert_eq!(
        (
            wal_metadata.uid(),
            wal_metadata.mode() & 0o777,
            wal_metadata.nlink()
        ),
        (effective_uid, 0o600, 1)
    );
    assert!(database.len() > 0 && wal_metadata.len() > 0);
}

fn seed(journal: &mut Journal) -> (RunId, ObjectId) {
    let run_id = RunId::new("11111111111111111111111111111111").unwrap();
    journal
        .begin_run(&RunIntent {
            run_id: run_id.clone(),
            request_id: request(1),
            run_token: RunToken::new("ab".repeat(32)).unwrap(),
            profile: ProfileName::new("gate").unwrap(),
            owner_uid: 1000,
            owner_pid: 4321,
            owner_starttime: 987_654,
        })
        .expect("persist run");
    let object_id = ObjectId::new("22222222222222222222222222222222").unwrap();
    journal
        .begin_intent(&IntentRecord {
            object_id: object_id.clone(),
            request_id: request(2),
            run_id: run_id.clone(),
            role: RoleName::new("target").unwrap(),
            root_alias: RootAlias::new("target").unwrap(),
            leaf: LeafName::new("proof-leaf").unwrap(),
        })
        .expect("persist intent");
    (run_id, object_id)
}

#[test]
fn state_machine_is_durable_and_visible_through_an_independent_reader() {
    let temp = TempDir::new().expect("real temporary filesystem");
    let path = temp.path().join("journal.sqlite");
    let mut journal = Journal::open(&path).expect("open real SQLite journal");
    let (run_id, object_id) = seed(&mut journal);
    assert_eq!(journal.list_active_runs().unwrap().len(), 1);
    let stage_id = StageId::new("44444444444444444444444444444444").unwrap();
    journal
        .begin_stage(&StageIntent {
            stage_id: stage_id.clone(),
            request_id: request(3),
            run_id: run_id.clone(),
            label: StageLabel::new("compile").unwrap(),
            unit: UnitName::new("calyx-gate-proof.service").unwrap(),
            slice_unit: UnitName::new("calyx-gate.slice").unwrap(),
            worker_user: "calyxgate-worker".into(),
            worker_uid: 990,
        })
        .unwrap();
    assert_eq!(journal.list_incomplete_stages().unwrap().len(), 1);
    journal
        .mark_stage_running(
            &stage_id,
            &UnitName::new("calyx-gate-proof.service").unwrap(),
            &UnitName::new("calyx-gate.slice").unwrap(),
            &InvocationId::new("55555555555555555555555555555555").unwrap(),
            &AbsolutePath::new("/system.slice/calyx-gate-proof.service").unwrap(),
            &AbsolutePath::new("/calyx-gate.slice").unwrap(),
            RecordedCgroupIdentity {
                device: 29,
                inode: 8_001,
            },
            RecordedCgroupIdentity {
                device: 29,
                inode: 8_000,
            },
            4242,
        )
        .unwrap();
    assert_eq!(
        journal.finish_stage(&stage_id, 0).unwrap(),
        StageState::Succeeded
    );
    journal
        .transition(
            &object_id,
            TransactionState::Intent,
            TransactionState::Prepared,
            TransitionUpdate {
                identity: Some(identity(0x5a)),
                ..Default::default()
            },
        )
        .unwrap();
    for (from, to, update) in [
        (
            TransactionState::Prepared,
            TransactionState::Published,
            TransitionUpdate::default(),
        ),
        (
            TransactionState::Published,
            TransactionState::Committed,
            TransitionUpdate::default(),
        ),
        (
            TransactionState::Committed,
            TransactionState::DeleteIntent,
            TransitionUpdate::default(),
        ),
        (
            TransactionState::DeleteIntent,
            TransactionState::Quarantined,
            TransitionUpdate {
                quarantine_name: Some("q-22222222222222222222222222222222".into()),
                ..Default::default()
            },
        ),
        (
            TransactionState::Quarantined,
            TransactionState::Deleted,
            TransitionUpdate::default(),
        ),
    ] {
        journal.transition(&object_id, from, to, update).unwrap();
    }
    journal
        .finish_run(&run_id, RunState::Succeeded, Some("verified"))
        .unwrap();
    #[cfg(unix)]
    assert_live_journal_files(&path);
    journal.checkpoint().expect("durable checkpoint");
    drop(journal);

    let reopened = Journal::open(&path).expect("reopen source of truth");
    let record = reopened.get(&object_id).unwrap().expect("physical row");
    assert_eq!(record.state, TransactionState::Deleted);
    assert_eq!(
        record.identity.expect("stored identity").opaque.bytes(),
        &[0x5a; MAX_OPAQUE_HANDLE_BYTES]
    );
    assert_eq!(reopened.events(&object_id).unwrap().len(), 7);
    assert_eq!(
        reopened.get_run(&run_id).unwrap().unwrap().state,
        RunState::Succeeded
    );
    assert_eq!(
        reopened.get_run(&run_id).unwrap().unwrap().intent.owner_uid,
        1000
    );
    assert_eq!(
        reopened.get_stage(&stage_id).unwrap().unwrap().state,
        StageState::Succeeded
    );
    assert_eq!(
        reopened.get_stage(&stage_id).unwrap().unwrap().main_pid,
        Some(4242)
    );
    assert!(reopened.list_active_runs().unwrap().is_empty());
    assert!(reopened.list_incomplete_stages().unwrap().is_empty());

    let independent = Connection::open(&path).expect("independent SQLite reader");
    let state: String = independent
        .query_row(
            "SELECT state FROM object_transactions WHERE object_id='22222222222222222222222222222222'",
            [],
            |row| row.get(0),
        )
        .expect("read physical state");
    println!("SOURCE_OF_TRUTH object_id=22222222222222222222222222222222 state={state}");
    assert_eq!(state, "deleted");
    let stage_state: String = independent
        .query_row(
            "SELECT state FROM stages WHERE stage_id='44444444444444444444444444444444'",
            [],
            |row| row.get(0),
        )
        .expect("read physical stage state");
    println!("SOURCE_OF_TRUTH stage_id=44444444444444444444444444444444 state={stage_state}");
    assert_eq!(stage_state, "succeeded");
    let (owner_uid, main_pid): (u32, u32) = independent
        .query_row(
            "SELECT runs.owner_uid,stages.main_pid FROM runs JOIN stages USING(run_id) WHERE stages.stage_id='44444444444444444444444444444444'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read physical owner and stage pid");
    println!("SOURCE_OF_TRUTH owner_uid={owner_uid} main_pid={main_pid}");
    assert_eq!((owner_uid, main_pid), (1000, 4242));
    let journal_mode: String = independent
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    let application_id: i64 = independent
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .unwrap();
    let schema_version: i64 = independent
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    let strict_tables: i64 = independent
        .query_row(
            "SELECT count(*) FROM pragma_table_list WHERE schema='main' AND name NOT LIKE 'sqlite_%' AND strict=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    println!(
        "SOURCE_OF_TRUTH journal_mode={journal_mode} application_id={application_id} schema_version={schema_version} strict_tables={strict_tables}"
    );
    assert_eq!(
        (journal_mode.as_str(), application_id, schema_version),
        ("wal", 1129924418, 3)
    );
    assert_eq!(strict_tables, 6);
}

#[test]
fn operation_idempotence_is_atomic_and_terminal_responses_are_replayable() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("operations.sqlite");
    let mut journal = Journal::open(&path).unwrap();
    let (run_id, _) = seed(&mut journal);
    let success_id = request(10);
    let success = OperationIntent {
        request_id: success_id.clone(),
        request_hash: [0x5a; 32],
        verb: "abort_run".into(),
        run_id: Some(run_id.clone()),
    };
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
    let mut changed = success.clone();
    changed.request_hash = [0xa5; 32];
    assert!(journal.begin_operation(&changed).is_err());
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
    let failure = OperationIntent {
        request_id: failure_id.clone(),
        request_hash: [0x33; 32],
        verb: "exec_stage".into(),
        run_id: Some(run_id),
    };
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
        .prepare("SELECT request_id,state,length(request_hash),length(response_json),error_code FROM operations ORDER BY request_id")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    println!("SOURCE_OF_TRUTH operations={rows:?}");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.2 == 32 && row.3 > 0));
}

#[test]
fn incompatible_version_one_schema_is_rejected_instead_of_patched() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("obsolete.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE runs(run_id TEXT PRIMARY KEY,owner_pid INTEGER NOT NULL); PRAGMA user_version=1;",
        )
        .unwrap();
    drop(connection);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let result = Journal::open(&path);
    let error = result.err().expect("schema v1 must fail closed");
    println!("EDGE obsolete-schema error={error}");
    assert!(
        error
            .to_string()
            .contains("schema version 1 is unsupported")
    );
    let independent = Connection::open(&path).unwrap();
    let columns: Vec<String> = independent
        .prepare("PRAGMA table_info(runs)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    println!("SOURCE_OF_TRUTH obsolete_columns={columns:?}");
    assert_eq!(columns, ["run_id", "owner_pid"]);
}

#[test]
fn semantically_broken_event_chain_is_detected_on_independent_reopen() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("event-corruption.sqlite");
    let mut journal = Journal::open(&path).unwrap();
    let (_, object_id) = seed(&mut journal);
    journal.checkpoint().unwrap();
    drop(journal);

    let independent = Connection::open(&path).unwrap();
    let before: (Option<String>, String) = independent
        .query_row(
            "SELECT from_state,to_state FROM journal_events WHERE object_id=?1 ORDER BY event_id LIMIT 1",
            [object_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    println!("EDGE event-chain-corruption before={before:?}");
    independent
        .execute(
            "UPDATE journal_events SET from_state='prepared' WHERE object_id=?1 AND from_state IS NULL",
            [object_id.as_str()],
        )
        .unwrap();
    let after: (Option<String>, String) = independent
        .query_row(
            "SELECT from_state,to_state FROM journal_events WHERE object_id=?1 ORDER BY event_id LIMIT 1",
            [object_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    println!("EDGE event-chain-corruption after={after:?}");
    drop(independent);

    let error = Journal::open(&path)
        .err()
        .expect("semantic corruption must fail closed");
    println!("SOURCE_OF_TRUTH semantic-integrity-error={error}");
    assert!(error.to_string().contains("event") && error.to_string().contains("expected None"));
}

#[test]
fn invalid_transitions_and_duplicate_idempotency_keys_leave_state_unchanged() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("edge.sqlite");
    let mut journal = Journal::open(&path).unwrap();
    let (run_id, object_id) = seed(&mut journal);

    let before = journal.get(&object_id).unwrap().unwrap().state;
    println!("EDGE invalid-transition before={before:?}");
    assert!(
        journal
            .transition(
                &object_id,
                TransactionState::Intent,
                TransactionState::Deleted,
                TransitionUpdate::default(),
            )
            .is_err()
    );
    let after = journal.get(&object_id).unwrap().unwrap().state;
    println!("EDGE invalid-transition after={after:?}");
    assert_eq!(before, after);

    let duplicate = IntentRecord {
        object_id: ObjectId::new("33333333333333333333333333333333").unwrap(),
        request_id: request(2),
        run_id,
        role: RoleName::new("target").unwrap(),
        root_alias: RootAlias::new("target").unwrap(),
        leaf: LeafName::new("different-leaf").unwrap(),
    };
    assert!(journal.begin_intent(&duplicate).is_err());
    let independent = Connection::open(&path).unwrap();
    let count: i64 = independent
        .query_row("SELECT count(*) FROM object_transactions", [], |row| {
            row.get(0)
        })
        .unwrap();
    println!("EDGE duplicate-request rows_after={count}");
    assert_eq!(count, 1);
}

#[test]
fn missing_required_transition_metadata_is_rejected_before_sql_mutation() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("metadata.sqlite");
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
    let path = temp.path().join("terminal-run.sqlite");
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
    let replay_intent = OperationIntent {
        request_id: replay_id.clone(),
        request_hash: [0x77; 32],
        verb: "finish_run".into(),
        run_id: Some(run_id.clone()),
    };
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
    let new_result = journal.begin_operation(&OperationIntent {
        request_id: new_id.clone(),
        request_hash: [0x22; 32],
        verb: "delete_object".into(),
        run_id: Some(run_id.clone()),
    });
    let error = new_result.expect_err("terminal run must reject new work");
    println!("EDGE terminal-run-new-operation error={error}");
    assert_eq!(
        error.to_string(),
        format!(
            "invalid transition metadata: operation intent requires active run {run_id}, found succeeded"
        )
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
