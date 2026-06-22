use std::collections::BTreeMap;

use calyx_core::{
    CalyxError, CxFlags, CxId, InputRef, LedgerRef, LensId, METADATA_CHUNK_ID,
    METADATA_DATABASE_NAME, METADATA_SOURCE_EVENT_TIME_RAW, METADATA_SOURCE_EVENT_TIME_SECS,
    METADATA_SOURCE_SEQUENCE, METADATA_TEMPORAL_INACTIVE_REASON, METADATA_TEMPORAL_LANE_STATE,
    Modality, SlotId, SlotVector, TEMPORAL_LANE_ACTIVE, TEMPORAL_LANE_INACTIVE,
    TEMPORAL_MISSING_CREATED_AT, VaultId, content_address,
};
use calyx_registry::{instantiate_panel, text_default};

use super::manifest::{hex_encode, now_ms};
use super::reader::ChunkRow;
use crate::error::CliResult;

pub const BASE_SLOT: SlotId = SlotId::new(0);
pub const METADATA_ROWID: &str = "sqlite_rowid";
pub const METADATA_CONTENT_HASH: &str = "content_hash_blake3";
pub const METADATA_GTE_LENS_ID: &str = "gte_lens_id";
pub const GTE_EMBEDDING_DIM: usize = 768;

const CX_ID_PANEL_VERSION: u32 = 1;
const CX_ID_SALT: [u8; 16] = [0; 16];

/// Rust port of the `vault-sqlite.ts` direct-import invariants:
/// 1. `CxId` is deterministic from content bytes and the migration ID domain.
/// 2. Candidate text is never stored; the constellation carries a redacted hash reference.
/// 3. `chunk_id` and `database_name` survive verbatim as string metadata.
/// 4. GTE vectors are exact 768-d finite f32 payloads isolated by explicit lens/slot identity.
#[derive(Clone, Debug)]
pub struct VaultSqliteAdapter {
    vault_id: VaultId,
    panel_version: u32,
    gte_lens_id: LensId,
    slot_id: SlotId,
}

impl VaultSqliteAdapter {
    pub fn new_with_lens_slot(
        vault_id: VaultId,
        panel_version: u32,
        gte_lens_id: LensId,
        slot_id: SlotId,
    ) -> Self {
        Self {
            vault_id,
            panel_version,
            gte_lens_id,
            slot_id,
        }
    }

    pub fn cx_id(&self, row: &ChunkRow) -> CxId {
        CxId::from_input(&row.content, CX_ID_PANEL_VERSION, &CX_ID_SALT)
    }

    pub fn constellation(&self, row: &ChunkRow) -> CliResult<calyx_core::Constellation> {
        validate_gte_embedding(row)?;
        let cx_id = self.cx_id(row);
        let mut slots = BTreeMap::new();
        slots.insert(
            self.slot_id,
            SlotVector::Dense {
                dim: row.embedding.len() as u32,
                data: row.embedding.clone(),
            },
        );
        let mut metadata = BTreeMap::new();
        metadata.insert(METADATA_CHUNK_ID.to_string(), row.chunk_id.clone());
        metadata.insert(
            METADATA_DATABASE_NAME.to_string(),
            row.database_name.clone(),
        );
        metadata.insert(METADATA_ROWID.to_string(), row.row_num.to_string());
        metadata.insert(
            METADATA_SOURCE_SEQUENCE.to_string(),
            "sqlite_rowid".to_string(),
        );
        metadata.insert(
            METADATA_CONTENT_HASH.to_string(),
            hex_encode(&row.content_hash()),
        );
        metadata.insert(
            METADATA_GTE_LENS_ID.to_string(),
            self.gte_lens_id.to_string(),
        );
        let created_at = match row.event_time_secs {
            Some(secs) => {
                metadata.insert(
                    METADATA_TEMPORAL_LANE_STATE.to_string(),
                    TEMPORAL_LANE_ACTIVE.to_string(),
                );
                metadata.insert(
                    METADATA_SOURCE_EVENT_TIME_SECS.to_string(),
                    secs.to_string(),
                );
                if let Some(raw) = &row.event_time_raw {
                    metadata.insert(METADATA_SOURCE_EVENT_TIME_RAW.to_string(), raw.clone());
                }
                secs
            }
            None => {
                metadata.insert(
                    METADATA_TEMPORAL_LANE_STATE.to_string(),
                    TEMPORAL_LANE_INACTIVE.to_string(),
                );
                metadata.insert(
                    METADATA_TEMPORAL_INACTIVE_REASON.to_string(),
                    TEMPORAL_MISSING_CREATED_AT.to_string(),
                );
                now_ms()
            }
        };
        Ok(calyx_core::Constellation {
            cx_id,
            vault_id: self.vault_id,
            panel_version: self.panel_version,
            created_at,
            input_ref: InputRef {
                hash: row.content_hash(),
                pointer: Some(row.pointer()),
                redacted: true,
            },
            modality: Modality::Text,
            slots,
            scalars: BTreeMap::new(),
            metadata,
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                redacted_input: true,
                ..CxFlags::default()
            },
        })
    }
}

#[allow(dead_code)]
pub fn gte_lens_id_for_hash(model_weights_hash: &[u8; 32]) -> LensId {
    LensId::from_bytes(content_address([model_weights_hash.as_slice()]))
}

fn validate_gte_embedding(row: &ChunkRow) -> CliResult {
    if row.embedding.len() != GTE_EMBEDDING_DIM {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "row {} GTE embedding dim {} expected {GTE_EMBEDDING_DIM}",
            row.row_num,
            row.embedding.len()
        ))
        .into());
    }
    if row.embedding.iter().any(|value| !value.is_finite()) {
        return Err(CalyxError::lens_numerical_invariant(format!(
            "row {} GTE embedding contains NaN or Inf",
            row.row_num
        ))
        .into());
    }
    Ok(())
}

pub fn default_panel_version() -> u32 {
    instantiate_panel(&text_default(), 0).panel.version
}

pub fn default_gte_lens_id() -> LensId {
    instantiate_panel(&text_default(), 0).panel.slots[0].lens_id
}

pub fn default_base_lens_id() -> String {
    default_gte_lens_id().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn from_chunk_row_is_deterministic_and_preserves_metadata() {
        let adapter = adapter();
        let row = ChunkRow {
            row_num: 7,
            chunk_id: "abc123".to_string(),
            database_name: "mydb".to_string(),
            content: b"hello".to_vec(),
            embedding: vec![1.0; GTE_EMBEDDING_DIM],
            event_time_secs: Some(1_704_204_000),
            event_time_raw: Some("2024-01-02T14:00:00Z".to_string()),
        };

        let cx = adapter.constellation(&row).unwrap();

        assert_eq!(cx.cx_id, CxId::from_input(b"hello", 1, &[0; 16]));
        assert_eq!(adapter.cx_id(&row), cx.cx_id);
        assert_eq!(cx.chunk_id(), Some("abc123"));
        assert_eq!(cx.database_name(), Some("mydb"));
        assert_eq!(cx.input_ref.hash, row.content_hash());
        assert!(cx.input_ref.redacted);
        assert!(!serde_json::to_string(&cx).unwrap().contains("hello"));
        assert_eq!(
            cx.metadata.get(METADATA_GTE_LENS_ID),
            Some(&default_base_lens_id())
        );
        assert_eq!(cx.created_at, 1_704_204_000);
        assert_eq!(
            cx.metadata.get(METADATA_TEMPORAL_LANE_STATE),
            Some(&TEMPORAL_LANE_ACTIVE.to_string())
        );
        assert!(matches!(
            cx.slots.get(&BASE_SLOT),
            Some(SlotVector::Dense { dim: 768, data }) if data == &row.embedding
        ));
    }

    #[test]
    fn same_content_reuses_cx_id_across_source_identity_fields() {
        let adapter = adapter();
        let left = row(
            "left",
            "db-a",
            b"same content",
            vec![0.0; GTE_EMBEDDING_DIM],
        );
        let right = row(
            "right",
            "db-b",
            b"same content",
            vec![1.0; GTE_EMBEDDING_DIM],
        );

        assert_eq!(adapter.cx_id(&left), adapter.cx_id(&right));
    }

    #[test]
    fn lens_identity_and_slot_identity_are_explicitly_isolated() {
        let vault_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let left = VaultSqliteAdapter::new_with_lens_slot(
            vault_id,
            default_panel_version(),
            LensId::from_bytes([1; 16]),
            SlotId::new(1),
        );
        let right = VaultSqliteAdapter::new_with_lens_slot(
            vault_id,
            default_panel_version(),
            LensId::from_bytes([2; 16]),
            SlotId::new(2),
        );
        let source = row("abc123", "mydb", b"hello", vec![1.0; GTE_EMBEDDING_DIM]);

        let left_cx = left.constellation(&source).unwrap();
        let right_cx = right.constellation(&source).unwrap();

        assert_eq!(left_cx.cx_id, right_cx.cx_id);
        assert!(left_cx.slots.contains_key(&SlotId::new(1)));
        assert!(!left_cx.slots.contains_key(&SlotId::new(2)));
        assert!(right_cx.slots.contains_key(&SlotId::new(2)));
        assert_eq!(
            left_cx.metadata.get(METADATA_GTE_LENS_ID).unwrap(),
            &LensId::from_bytes([1; 16]).to_string()
        );
        assert_eq!(
            right_cx.metadata.get(METADATA_GTE_LENS_ID).unwrap(),
            &LensId::from_bytes([2; 16]).to_string()
        );
    }

    #[test]
    fn gte_lens_id_for_hash_is_content_addressed() {
        let hash = [0x42; 32];
        let same = gte_lens_id_for_hash(&hash);
        let mut changed_hash = hash;
        changed_hash[0] = 0x43;

        assert_eq!(same, gte_lens_id_for_hash(&hash));
        assert_ne!(same, gte_lens_id_for_hash(&changed_hash));
    }

    #[test]
    fn invalid_embedding_dimension_fails_with_exact_code() {
        let error = adapter()
            .constellation(&row("abc123", "mydb", b"hello", vec![1.0; 767]))
            .unwrap_err();

        assert_eq!(error.code(), "CALYX_LENS_DIM_MISMATCH");
        assert!(error.message().contains("row 7"));
        assert!(error.message().contains("767"));
    }

    #[test]
    fn nan_embedding_fails_with_exact_code() {
        let mut embedding = vec![1.0; GTE_EMBEDDING_DIM];
        embedding[42] = f32::NAN;

        let error = adapter()
            .constellation(&row("abc123", "mydb", b"hello", embedding))
            .unwrap_err();

        assert_eq!(error.code(), "CALYX_LENS_NUMERICAL_INVARIANT");
        assert!(error.message().contains("row 7"));
    }

    #[test]
    fn empty_database_name_and_long_chunk_id_are_preserved() {
        let chunk_id = "x".repeat(1000);
        let cx = adapter()
            .constellation(&row(&chunk_id, "", b"hello", vec![1.0; GTE_EMBEDDING_DIM]))
            .unwrap();

        assert_eq!(cx.chunk_id(), Some(chunk_id.as_str()));
        assert_eq!(cx.database_name(), Some(""));
    }

    proptest! {
        #[test]
        fn finite_768_dense_embedding_round_trips(
            embedding in prop::collection::vec(-1.0f32..1.0f32, GTE_EMBEDDING_DIM)
        ) {
            let cx = adapter()
                .constellation(&row("abc123", "mydb", b"hello", embedding.clone()))
                .unwrap();

            prop_assert_eq!(
                cx.slots.get(&BASE_SLOT),
                Some(&SlotVector::Dense { dim: 768, data: embedding })
            );
        }
    }

    fn adapter() -> VaultSqliteAdapter {
        VaultSqliteAdapter::new_with_lens_slot(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
            default_panel_version(),
            default_gte_lens_id(),
            BASE_SLOT,
        )
    }

    fn row(chunk_id: &str, database_name: &str, content: &[u8], embedding: Vec<f32>) -> ChunkRow {
        ChunkRow {
            row_num: 7,
            chunk_id: chunk_id.to_string(),
            database_name: database_name.to_string(),
            content: content.to_vec(),
            embedding,
            event_time_secs: Some(1_704_204_000),
            event_time_raw: Some("2024-01-02T14:00:00Z".to_string()),
        }
    }
}
