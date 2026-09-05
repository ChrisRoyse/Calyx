use super::support::*;

#[test]
fn incompatible_version_one_schema_is_rejected_instead_of_patched() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("obsolete.turso");
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
    let path = temp.path().join("event-corruption.turso");
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
    let path = temp.path().join("edge.turso");
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
