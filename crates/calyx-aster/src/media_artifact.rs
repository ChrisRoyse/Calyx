//! Queryable media-derived text artifact records.
//!
//! The derived text constellation remains content-addressed by text bytes. These
//! rows represent the source-specific derivation run that produced or reused
//! that text, so two media inputs with identical transcript text can keep
//! separate provenance without rewriting the shared text Base row.

use crate::cf::{ColumnFamily, KeyRange, prefix_range};
use crate::vault::{AsterVault, encode};
use calyx_core::{
    CALYX_MEDIA_ARTIFACT_COLLISION, CALYX_MEDIA_ARTIFACT_INVALID, CalyxError, Clock, CxId,
    LedgerRef, Result,
};
use serde::{Deserialize, Serialize};

const PRIMARY_PREFIX: &[u8] = b"media-derived-artifact/v1/primary\0";
const SOURCE_PREFIX: &[u8] = b"media-derived-artifact/v1/source\0";
const TARGET_PREFIX: &[u8] = b"media-derived-artifact/v1/target\0";
const POINTER_PREFIX: &str = "calyx-vault://";
const MAX_ARTIFACT_ID_BYTES: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DerivedMediaArtifactDraft {
    pub artifact_id: String,
    pub source_cx_id: CxId,
    pub target_cx_id: CxId,
    pub derived_kind: String,
    pub source_modality: String,
    pub source_input_hash: String,
    pub source_sha256: String,
    pub source_pointer: String,
    pub target_pointer: String,
    pub target_text_sha256: String,
    pub runtime: String,
    pub model: String,
    pub language: Option<String>,
    pub confidence: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DerivedMediaArtifactRecord {
    pub artifact_id: String,
    pub source_cx_id: CxId,
    pub target_cx_id: CxId,
    pub derived_kind: String,
    pub source_modality: String,
    pub source_input_hash: String,
    pub source_sha256: String,
    pub source_pointer: String,
    pub target_pointer: String,
    pub target_text_sha256: String,
    pub runtime: String,
    pub model: String,
    pub language: Option<String>,
    pub confidence: Option<f64>,
    pub ledger_ref: LedgerRef,
}

impl DerivedMediaArtifactDraft {
    pub fn into_record(self, ledger_ref: LedgerRef) -> Result<DerivedMediaArtifactRecord> {
        self.validate()?;
        Ok(DerivedMediaArtifactRecord {
            artifact_id: self.artifact_id,
            source_cx_id: self.source_cx_id,
            target_cx_id: self.target_cx_id,
            derived_kind: self.derived_kind,
            source_modality: self.source_modality,
            source_input_hash: self.source_input_hash,
            source_sha256: self.source_sha256,
            source_pointer: self.source_pointer,
            target_pointer: self.target_pointer,
            target_text_sha256: self.target_text_sha256,
            runtime: self.runtime,
            model: self.model,
            language: self.language,
            confidence: self.confidence,
            ledger_ref,
        })
    }

    fn validate(&self) -> Result<()> {
        validate_artifact_id(&self.artifact_id)?;
        validate_required("derived_kind", &self.derived_kind)?;
        validate_required("source_modality", &self.source_modality)?;
        validate_required("source_input_hash", &self.source_input_hash)?;
        validate_required("source_sha256", &self.source_sha256)?;
        validate_vault_pointer("source_pointer", &self.source_pointer)?;
        validate_vault_pointer("target_pointer", &self.target_pointer)?;
        validate_required("target_text_sha256", &self.target_text_sha256)?;
        validate_required("runtime", &self.runtime)?;
        validate_required("model", &self.model)?;
        if let Some(confidence) = self.confidence
            && (!confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
        {
            return Err(media_artifact_error(format!(
                "derived artifact confidence {confidence} is outside [0,1]"
            )));
        }
        Ok(())
    }
}

impl DerivedMediaArtifactRecord {
    pub fn validate(&self) -> Result<()> {
        DerivedMediaArtifactDraft {
            artifact_id: self.artifact_id.clone(),
            source_cx_id: self.source_cx_id,
            target_cx_id: self.target_cx_id,
            derived_kind: self.derived_kind.clone(),
            source_modality: self.source_modality.clone(),
            source_input_hash: self.source_input_hash.clone(),
            source_sha256: self.source_sha256.clone(),
            source_pointer: self.source_pointer.clone(),
            target_pointer: self.target_pointer.clone(),
            target_text_sha256: self.target_text_sha256.clone(),
            runtime: self.runtime.clone(),
            model: self.model.clone(),
            language: self.language.clone(),
            confidence: self.confidence,
        }
        .validate()
    }
}

pub fn derived_media_artifact_key(artifact_id: &str) -> Result<Vec<u8>> {
    validate_artifact_id(artifact_id)?;
    Ok(artifact_key_unchecked(
        PRIMARY_PREFIX,
        artifact_id.as_bytes(),
    ))
}

pub fn derived_media_artifact_source_prefix(source_cx_id: CxId) -> KeyRange {
    let mut key = Vec::with_capacity(SOURCE_PREFIX.len() + 16);
    key.extend_from_slice(SOURCE_PREFIX);
    key.extend_from_slice(source_cx_id.as_bytes());
    prefix_range(&key)
}

pub fn derived_media_artifact_target_prefix(target_cx_id: CxId) -> KeyRange {
    let mut key = Vec::with_capacity(TARGET_PREFIX.len() + 16);
    key.extend_from_slice(TARGET_PREFIX);
    key.extend_from_slice(target_cx_id.as_bytes());
    prefix_range(&key)
}

pub fn decode_derived_media_artifact(bytes: &[u8]) -> Result<DerivedMediaArtifactRecord> {
    let record: DerivedMediaArtifactRecord = serde_json::from_slice(bytes).map_err(|error| {
        media_artifact_error(format!("decode derived media artifact record: {error}"))
    })?;
    record.validate()?;
    Ok(record)
}

pub(crate) fn derived_media_artifact_write_rows(
    record: &DerivedMediaArtifactRecord,
) -> Result<Vec<encode::WriteRow>> {
    record.validate()?;
    let primary_key = derived_media_artifact_key(&record.artifact_id)?;
    let value = serde_json::to_vec(record).map_err(|error| {
        media_artifact_error(format!("encode derived media artifact record: {error}"))
    })?;
    Ok(vec![
        encode::WriteRow {
            cf: ColumnFamily::Graph,
            key: primary_key,
            value,
        },
        encode::WriteRow {
            cf: ColumnFamily::Graph,
            key: source_index_key(record.source_cx_id, &record.artifact_id)?,
            value: record.artifact_id.as_bytes().to_vec(),
        },
        encode::WriteRow {
            cf: ColumnFamily::Graph,
            key: target_index_key(record.target_cx_id, &record.artifact_id)?,
            value: record.artifact_id.as_bytes().to_vec(),
        },
    ])
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn get_derived_media_artifact(
        &self,
        snapshot: u64,
        artifact_id: &str,
    ) -> Result<Option<DerivedMediaArtifactRecord>> {
        let Some(bytes) = self.read_cf_at(
            snapshot,
            ColumnFamily::Graph,
            &derived_media_artifact_key(artifact_id)?,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(decode_derived_media_artifact(&bytes)?))
    }

    pub fn derived_media_artifacts_for_source(
        &self,
        snapshot: u64,
        source_cx_id: CxId,
    ) -> Result<Vec<DerivedMediaArtifactRecord>> {
        self.derived_media_artifacts_from_index(
            snapshot,
            &derived_media_artifact_source_prefix(source_cx_id),
        )
    }

    pub fn derived_media_artifacts_for_target(
        &self,
        snapshot: u64,
        target_cx_id: CxId,
    ) -> Result<Vec<DerivedMediaArtifactRecord>> {
        self.derived_media_artifacts_from_index(
            snapshot,
            &derived_media_artifact_target_prefix(target_cx_id),
        )
    }

    fn derived_media_artifacts_from_index(
        &self,
        snapshot: u64,
        range: &KeyRange,
    ) -> Result<Vec<DerivedMediaArtifactRecord>> {
        let mut records = Vec::new();
        for (_, artifact_id_bytes) in self.scan_cf_range_at(snapshot, ColumnFamily::Graph, range)? {
            let artifact_id = std::str::from_utf8(&artifact_id_bytes).map_err(|error| {
                media_artifact_error(format!("artifact index value is not utf-8: {error}"))
            })?;
            let record = self
                .get_derived_media_artifact(snapshot, artifact_id)?
                .ok_or_else(|| {
                    CalyxError::aster_corrupt_shard(format!(
                        "derived media artifact index references missing artifact {artifact_id}"
                    ))
                })?;
            records.push(record);
        }
        Ok(records)
    }
}

pub(crate) fn ensure_no_artifact_collision<C>(
    vault: &AsterVault<C>,
    snapshot: u64,
    record: &DerivedMediaArtifactRecord,
) -> Result<()>
where
    C: Clock,
{
    let key = derived_media_artifact_key(&record.artifact_id)?;
    if let Some(existing) = vault.read_cf_at(snapshot, ColumnFamily::Graph, &key)? {
        let encoded = serde_json::to_vec(record).map_err(|error| {
            media_artifact_error(format!("encode derived media artifact record: {error}"))
        })?;
        if existing != encoded {
            return Err(CalyxError {
                code: CALYX_MEDIA_ARTIFACT_COLLISION,
                message: format!(
                    "derived media artifact id {} already exists with different bytes",
                    record.artifact_id
                ),
                remediation: "generate a fresh artifact id and retry the derivation",
            });
        }
    }
    Ok(())
}

fn source_index_key(source_cx_id: CxId, artifact_id: &str) -> Result<Vec<u8>> {
    validate_artifact_id(artifact_id)?;
    let mut key = Vec::with_capacity(SOURCE_PREFIX.len() + 16 + 1 + artifact_id.len());
    key.extend_from_slice(SOURCE_PREFIX);
    key.extend_from_slice(source_cx_id.as_bytes());
    key.push(0);
    key.extend_from_slice(artifact_id.as_bytes());
    Ok(key)
}

fn target_index_key(target_cx_id: CxId, artifact_id: &str) -> Result<Vec<u8>> {
    validate_artifact_id(artifact_id)?;
    let mut key = Vec::with_capacity(TARGET_PREFIX.len() + 16 + 1 + artifact_id.len());
    key.extend_from_slice(TARGET_PREFIX);
    key.extend_from_slice(target_cx_id.as_bytes());
    key.push(0);
    key.extend_from_slice(artifact_id.as_bytes());
    Ok(key)
}

fn artifact_key_unchecked(prefix: &[u8], artifact_id: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + artifact_id.len());
    key.extend_from_slice(prefix);
    key.extend_from_slice(artifact_id);
    key
}

fn validate_artifact_id(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_ARTIFACT_ID_BYTES {
        return Err(media_artifact_error(format!(
            "derived artifact id must be 1..={MAX_ARTIFACT_ID_BYTES} bytes"
        )));
    }
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(media_artifact_error(
            "derived artifact id must be URL-safe ASCII",
        ));
    }
    Ok(())
}

fn validate_required(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(media_artifact_error(format!(
            "derived artifact {field} must not be empty"
        )));
    }
    Ok(())
}

fn validate_vault_pointer(field: &str, value: &str) -> Result<()> {
    validate_required(field, value)?;
    if !value.starts_with(POINTER_PREFIX) {
        return Err(media_artifact_error(format!(
            "derived artifact {field} must be a vault-relative pointer"
        )));
    }
    Ok(())
}

fn media_artifact_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_MEDIA_ARTIFACT_INVALID,
        message: message.into(),
        remediation: "persist source-specific derived media provenance as a valid artifact record",
    }
}

#[cfg(test)]
mod tests {
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
}
