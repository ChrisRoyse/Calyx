use super::*;
use crate::vault::VaultOptions;
use calyx_core::VaultId;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

#[test]
fn graph_collection_lifecycle_roundtrips_from_physical_storage() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "calyx-graph-lifecycle-{}-{unique}",
        std::process::id()
    ));
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("durable vault");
    let lifecycle = GraphCollectionLifecycle::new(&vault).expect("lifecycle");
    let writing = GraphCollectionGenerationState::new(
        "biomed_test",
        "gen-1",
        GraphCollectionGenerationStatus::Writing,
        "test-materializer",
    )
    .with_reason("started")
    .with_detail("source", "unit-test");
    lifecycle.put_state(&writing).expect("write state");
    let accepted = GraphCollectionGenerationState {
        status: GraphCollectionGenerationStatus::Accepted,
        reason: Some("readback passed".to_string()),
        ..writing
    };
    lifecycle.put_state(&accepted).expect("accept state");
    vault.flush().expect("flush");

    let physical = PhysicalGraphCollectionLifecycle::open_latest(&dir).expect("physical");
    let rows = physical.list_states().expect("list states");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, accepted);
    assert_eq!(rows[0].state.status.as_str(), "accepted");
    assert!(rows[0].state.visible_by_default());
    assert!(rows[0].value_bytes > 0);
    assert_eq!(rows[0].key_sha256.len(), 64);
    assert_eq!(rows[0].value_sha256.len(), 64);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn graph_collection_lifecycle_rejects_invalid_state() {
    let vault = AsterVault::new(vault_id(), b"salt");
    let lifecycle = GraphCollectionLifecycle::new(&vault).expect("lifecycle");
    let invalid = GraphCollectionGenerationState::new(
        "valid_collection",
        "",
        GraphCollectionGenerationStatus::Writing,
        "test-materializer",
    );
    let error = lifecycle.put_state(&invalid).expect_err("invalid state");
    assert_eq!(error.code, "CALYX_GRAPH_COLLECTION_LIFECYCLE_INVALID");
}

#[test]
fn graph_collection_lifecycle_physical_writer_roundtrips_bytes() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "calyx-graph-lifecycle-physical-{}-{unique}",
        std::process::id()
    ));
    let mut writer = PhysicalGraphCollectionLifecycle::open_latest(&dir).expect("physical writer");
    let state = GraphCollectionGenerationState::new(
        "biomed_test",
        "gen-physical",
        GraphCollectionGenerationStatus::Tombstoned,
        "test-materializer",
    )
    .with_reason("maintenance tombstone")
    .with_detail("source", "unit-test");
    writer.put_state_physical(&state).expect("write physical");
    drop(writer);

    let physical = PhysicalGraphCollectionLifecycle::open_latest(&dir).expect("physical readback");
    let rows = physical.list_states().expect("list states");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, state);
    assert_eq!(rows[0].key_sha256.len(), 64);
    assert_eq!(rows[0].value_sha256.len(), 64);
    assert!(rows[0].value_bytes > 0);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn graph_collection_lifecycle_open_latest_accepted_fails_closed_on_tombstone() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "calyx-graph-lifecycle-open-{}-{unique}",
        std::process::id()
    ));
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("durable vault");
    let lifecycle = GraphCollectionLifecycle::new(&vault).expect("lifecycle");
    let tombstoned = GraphCollectionGenerationState::new(
        "biomed_test",
        "gen-2",
        GraphCollectionGenerationStatus::Tombstoned,
        "test-materializer",
    )
    .with_reason("aborted before readback");
    lifecycle.put_state(&tombstoned).expect("write tombstone");
    vault.flush().expect("flush tombstone");

    let error = match PhysicalPlainGraph::open_latest(&dir, "biomed_test") {
        Ok(_) => panic!("tombstoned-only collection must fail closed"),
        Err(error) => error,
    };
    assert_eq!(error.code, "CALYX_GRAPH_COLLECTION_NOT_ACCEPTED");
    PhysicalPlainGraph::open_latest_unchecked(&dir, "biomed_test")
        .expect("explicit unchecked open");

    let accepted = GraphCollectionGenerationState {
        status: GraphCollectionGenerationStatus::Accepted,
        reason: Some("physical readback passed".to_string()),
        ..tombstoned
    };
    lifecycle.put_state(&accepted).expect("write accepted");
    vault.flush().expect("flush accepted");
    PhysicalPlainGraph::open_latest(&dir, "biomed_test").expect("accepted collection");

    let _ = std::fs::remove_dir_all(dir);
}

/// A `writing` generation with no accepted generation must distinguish a genuinely
/// in-progress rebuild (fresh row → transient, retryable) from a WEDGED/stale one (old
/// row, no active rebuild → non-retryable). These are DISTINCT codes so a stale flag is
/// never mistaken for a transient one (#1977).
#[test]
fn writing_generation_reports_in_progress_when_fresh_and_stale_when_wedged() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "calyx-graph-lifecycle-rebuild-{}-{unique}",
        std::process::id()
    ));
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("durable vault");
    let lifecycle = GraphCollectionLifecycle::new(&vault).expect("lifecycle");

    // A FRESH writing generation (just touched) → transient rebuild-in-progress.
    let fresh = GraphCollectionGenerationState::new(
        "biomed_test",
        "gen-fresh",
        GraphCollectionGenerationStatus::Writing,
        "test-materializer",
    );
    lifecycle.put_state(&fresh).expect("write fresh writing");
    vault.flush().expect("flush fresh");
    let error = PhysicalPlainGraph::open_latest(&dir, "biomed_test")
        .err()
        .expect("fresh writing must fail closed");
    assert_eq!(error.code, "CALYX_GRAPH_COLLECTION_REBUILD_IN_PROGRESS");

    // A STALE writing generation (row updated far in the past) → wedged, non-retryable.
    let mut stale = GraphCollectionGenerationState::new(
        "biomed_test",
        "gen-fresh",
        GraphCollectionGenerationStatus::Writing,
        "test-materializer",
    );
    stale.updated_unix_ms = 1; // ancient — no rebuild process is running
    lifecycle.put_state(&stale).expect("write stale writing");
    vault.flush().expect("flush stale");
    let error = PhysicalPlainGraph::open_latest(&dir, "biomed_test")
        .err()
        .expect("stale writing must fail closed");
    assert_eq!(error.code, "CALYX_GRAPH_COLLECTION_REBUILD_STALE");

    let _ = std::fs::remove_dir_all(dir);
}
