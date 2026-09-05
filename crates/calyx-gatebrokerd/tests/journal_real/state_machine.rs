use super::support::*;

#[test]
fn state_machine_is_durable_and_visible_through_an_independent_reader() {
    let temp = TempDir::new().expect("real temporary filesystem");
    let path = temp.path().join("journal.turso");
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
        .mark_stage_running(&StageRunningEvidence {
            stage_id: &stage_id,
            expected_unit: &UnitName::new("calyx-gate-proof.service").unwrap(),
            expected_slice_unit: &UnitName::new("calyx-gate.slice").unwrap(),
            invocation_id: &InvocationId::new("55555555555555555555555555555555").unwrap(),
            control_group: &AbsolutePath::new(
                "/calyx.slice/calyx-gate.slice/calyx-gate-proof.service",
            )
            .unwrap(),
            slice_control_group: &AbsolutePath::new("/calyx.slice/calyx-gate.slice").unwrap(),
            control_group_identity: RecordedCgroupIdentity {
                device: 29,
                inode: 8_001,
            },
            slice_control_group_identity: RecordedCgroupIdentity {
                device: 29,
                inode: 8_000,
            },
            main_pid: 4242,
        })
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
    let recorded_stage: (String, String, String, u32, String, String, String, String, String, String) =
        independent
            .query_row(
                "SELECT unit,slice_unit,worker_user,worker_uid,control_group,slice_control_group,control_group_device,control_group_inode,slice_control_group_device,slice_control_group_inode FROM stages WHERE stage_id='44444444444444444444444444444444'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .expect("read physical planned and captured stage authority");
    println!("SOURCE_OF_TRUTH stage_authority={recorded_stage:?}");
    assert_eq!(
        recorded_stage,
        (
            "calyx-gate-proof.service".into(),
            "calyx-gate.slice".into(),
            "calyxgate-worker".into(),
            990,
            "/calyx.slice/calyx-gate.slice/calyx-gate-proof.service".into(),
            "/calyx.slice/calyx-gate.slice".into(),
            "29".into(),
            "8001".into(),
            "29".into(),
            "8000".into(),
        )
    );
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
        ("wal", 1129924418, 4)
    );
    assert_eq!(strict_tables, 6);
}
