use std::collections::BTreeMap;

use calyx_core::{
    CxFlags, CxId, InputRef, LedgerRef, METADATA_CHUNK_ID, METADATA_DATABASE_NAME, Modality,
    SlotId, SlotVector, VaultId,
};
use calyx_registry::{instantiate_panel, text_default};

use super::manifest::{hex_encode, now_ms};
use super::reader::ChunkRow;

pub const BASE_SLOT: SlotId = SlotId::new(0);
pub const METADATA_ROWID: &str = "sqlite_rowid";
pub const METADATA_CONTENT_HASH: &str = "content_hash_blake3";

#[derive(Clone, Debug)]
pub struct VaultSqliteAdapter {
    vault_id: VaultId,
    vault_salt: Vec<u8>,
    panel_version: u32,
}

impl VaultSqliteAdapter {
    pub fn new(vault_id: VaultId, vault_salt: Vec<u8>, panel_version: u32) -> Self {
        Self {
            vault_id,
            vault_salt,
            panel_version,
        }
    }

    pub fn cx_id(&self, row: &ChunkRow) -> CxId {
        CxId::from_input(&row.identity_bytes(), self.panel_version, &self.vault_salt)
    }

    pub fn constellation(&self, row: &ChunkRow) -> calyx_core::Constellation {
        let cx_id = self.cx_id(row);
        let mut slots = BTreeMap::new();
        slots.insert(
            BASE_SLOT,
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
            METADATA_CONTENT_HASH.to_string(),
            hex_encode(&row.content_hash()),
        );
        calyx_core::Constellation {
            cx_id,
            vault_id: self.vault_id,
            panel_version: self.panel_version,
            created_at: now_ms(),
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
        }
    }
}

pub fn default_panel_version() -> u32 {
    instantiate_panel(&text_default(), 0).panel.version
}

pub fn default_base_lens_id() -> String {
    instantiate_panel(&text_default(), 0).panel.slots[0]
        .lens_id
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_leapable_contract_metadata() {
        let adapter = VaultSqliteAdapter::new(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
            b"salt".to_vec(),
            default_panel_version(),
        );
        let row = ChunkRow {
            row_num: 7,
            chunk_id: "chunk/with spaces".to_string(),
            database_name: "leapable_db".to_string(),
            content: b"known content".to_vec(),
            embedding: vec![0.25, 0.75],
        };

        let cx = adapter.constellation(&row);

        assert_eq!(cx.chunk_id(), Some("chunk/with spaces"));
        assert_eq!(cx.database_name(), Some("leapable_db"));
        assert_eq!(cx.input_ref.hash, row.content_hash());
        assert!(matches!(
            cx.slots.get(&BASE_SLOT),
            Some(SlotVector::Dense { dim: 2, .. })
        ));
    }
}
