use super::*;
use crate::vault::VaultOptions;
use calyx_core::{DERIVED_TEXT_MODE, VaultId};
use calyx_ledger::{ActorId, SubjectId};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn duplicate_transcript_artifacts_keep_distinct_source_lineage() {
    let dir = temp_dir("media-artifact-duplicate-transcript");
    let vault = durable_vault(&dir);
    let target_cx_id = cx(200);
    let first = draft("artifact_a", cx(1), target_cx_id, 1);
    let second = draft("artifact_b", cx(2), target_cx_id, 2);

    assert_eq!(graph_rows(&vault), 0);
    assert_eq!(ledger_rows(&vault), 0);

    let first_commit = vault
        .put_batch_with_ingest_ledger_and_media_artifact(
            Vec::new(),
            SubjectId::Cx(target_cx_id),
            payload(&first.artifact_id),
            ActorId::Service("calyx-aster-test".to_string()),
            first,
        )
        .unwrap();
    let second_commit = vault
        .put_batch_with_ingest_ledger_and_media_artifact(
            Vec::new(),
            SubjectId::Cx(target_cx_id),
            payload(&second.artifact_id),
            ActorId::Service("calyx-aster-test".to_string()),
            second,
        )
        .unwrap();

    let snapshot = vault.latest_seq();
    assert_eq!(ledger_rows(&vault), 2);
    assert_eq!(graph_rows(&vault), 6);
    assert_eq!(
        vault
            .get_derived_media_artifact(snapshot, &first_commit.artifact.artifact_id)
            .unwrap(),
        Some(first_commit.artifact.clone())
    );
    assert_eq!(
        vault
            .get_derived_media_artifact(snapshot, &second_commit.artifact.artifact_id)
            .unwrap(),
        Some(second_commit.artifact.clone())
    );
    let target_records = vault
        .derived_media_artifacts_for_target(snapshot, target_cx_id)
        .unwrap();
    assert_eq!(target_records.len(), 2);
    assert!(target_records.contains(&first_commit.artifact));
    assert!(target_records.contains(&second_commit.artifact));
    assert_eq!(
        vault
            .derived_media_artifacts_for_source(snapshot, cx(1))
            .unwrap(),
        vec![first_commit.artifact.clone()]
    );
    assert_eq!(
        vault
            .derived_media_artifacts_for_source(snapshot, cx(2))
            .unwrap(),
        vec![second_commit.artifact.clone()]
    );
    assert_eq!(
        first_commit.artifact.target_text_sha256,
        second_commit.artifact.target_text_sha256
    );
    assert_ne!(
        first_commit.artifact.source_cx_id,
        second_commit.artifact.source_cx_id
    );

    drop(vault);
    let reopened = durable_vault(&dir);
    let reopened_snapshot = reopened.latest_seq();
    assert_eq!(
        reopened
            .derived_media_artifacts_for_target(reopened_snapshot, target_cx_id)
            .unwrap()
            .len(),
        2
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn invalid_artifact_pointer_does_not_write_graph_or_ledger_rows() {
    let dir = temp_dir("media-artifact-invalid-pointer");
    let vault = durable_vault(&dir);
    let mut invalid = draft("artifact_bad_pointer", cx(1), cx(200), 1);
    invalid.source_pointer = "file:///tmp/raw.wav".to_string();

    let before_seq = vault.latest_seq();
    let before_graph = graph_rows(&vault);
    let before_ledger = ledger_rows(&vault);
    let err = vault
        .put_batch_with_ingest_ledger_and_media_artifact(
            Vec::new(),
            SubjectId::Cx(cx(200)),
            payload(&invalid.artifact_id),
            ActorId::Service("calyx-aster-test".to_string()),
            invalid,
        )
        .unwrap_err();

    assert_eq!(err.code, CALYX_MEDIA_ARTIFACT_INVALID);
    assert_eq!(vault.latest_seq(), before_seq);
    assert_eq!(graph_rows(&vault), before_graph);
    assert_eq!(ledger_rows(&vault), before_ledger);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn artifact_id_collision_does_not_partially_commit() {
    let dir = temp_dir("media-artifact-collision");
    let vault = durable_vault(&dir);
    let original = draft("artifact_collision", cx(1), cx(200), 1);
    let original_commit = vault
        .put_batch_with_ingest_ledger_and_media_artifact(
            Vec::new(),
            SubjectId::Cx(cx(200)),
            payload(&original.artifact_id),
            ActorId::Service("calyx-aster-test".to_string()),
            original,
        )
        .unwrap();
    let before_seq = vault.latest_seq();
    let before_graph = graph_rows(&vault);
    let before_ledger = ledger_rows(&vault);

    let collision = draft("artifact_collision", cx(2), cx(200), 2);
    let err = vault
        .put_batch_with_ingest_ledger_and_media_artifact(
            Vec::new(),
            SubjectId::Cx(cx(200)),
            payload(&collision.artifact_id),
            ActorId::Service("calyx-aster-test".to_string()),
            collision,
        )
        .unwrap_err();

    assert_eq!(err.code, CALYX_MEDIA_ARTIFACT_COLLISION);
    assert_eq!(vault.latest_seq(), before_seq);
    assert_eq!(graph_rows(&vault), before_graph);
    assert_eq!(ledger_rows(&vault), before_ledger);
    assert_eq!(
        vault
            .get_derived_media_artifact(vault.latest_seq(), "artifact_collision")
            .unwrap(),
        Some(original_commit.artifact)
    );
    fs::remove_dir_all(dir).ok();
}

fn durable_vault(dir: &PathBuf) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        b"media-artifact-tests".to_vec(),
        VaultOptions::default(),
    )
    .unwrap()
}

fn graph_rows(vault: &AsterVault) -> usize {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Graph)
        .unwrap()
        .len()
}

fn ledger_rows(vault: &AsterVault) -> usize {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
        .len()
}

fn draft(
    artifact_id: &str,
    source_cx_id: CxId,
    target_cx_id: CxId,
    source_seed: u8,
) -> DerivedMediaArtifactDraft {
    DerivedMediaArtifactDraft {
        artifact_id: artifact_id.to_string(),
        source_cx_id,
        target_cx_id,
        derived_kind: "transcript".to_string(),
        source_modality: "audio".to_string(),
        source_input_hash: hex32(source_seed),
        source_sha256: hex32(source_seed.saturating_add(10)),
        source_pointer: format!(
            "calyx-vault://inputs/media/audio/{}.wav",
            hex32(source_seed)
        ),
        target_pointer: "calyx-vault://inputs/derived_text/transcript/shared-transcript.txt"
            .to_string(),
        target_text_sha256: hex32(99),
        runtime: "calyx-real-transcriber".to_string(),
        model: "calyx-real-transcriber-v1".to_string(),
        language: Some("en".to_string()),
        confidence: Some(0.91),
    }
}

fn payload(artifact_id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "mode": DERIVED_TEXT_MODE,
        "derived_artifact_id": artifact_id,
    }))
    .unwrap()
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn hex32(seed: u8) -> String {
    [seed; 32]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    dir
}
