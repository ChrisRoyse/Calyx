//! Document `(collection, doc_id, path...) -> leaf` key-encoding layer.

use std::collections::BTreeSet;

use calyx_core::{Clock, Modality, Result, Seq};
use serde_json::Value;

use crate::cf::{ColumnFamily, KeyRange, prefix_range};
use crate::collection::{
    Collection, CollectionMode, collection_has_lens, ingest_collection_constellation,
};
use crate::index::{IndexMaintenance, collection_has_maintained_index};
use crate::layers::relational::RecordKey;
use crate::vault::AsterVault;
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};

use super::Layer;

mod codec;
mod errors;
mod schema;
mod tree;

pub use codec::{DocId, collection_id, document_key, document_prefix};
use codec::{
    DocumentCell, decode_cell, document_key_from_segments, document_path_prefix, encode_cell,
    hex_bytes, parse_document_key, path_segments,
};
pub use errors::CALYX_SCHEMA_VIOLATION;
use errors::{corrupt_doc, invalid_argument};
use schema::validate_document;
use tree::{build_tree, docs_from_rows, flatten_document};

pub(crate) type DocumentWriteRows = Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>;

pub struct DocumentLayer<'a, C: Clock> {
    vault: &'a AsterVault<C>,
}

impl<'a, C: Clock> DocumentLayer<'a, C> {
    pub fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }

    pub fn put_doc(&self, col: &Collection, doc_id: DocId, doc: &Value) -> Result<Seq> {
        if collection_has_lens(col) {
            validate_document(col, doc)?;
            let value = encode_json_value(doc)?;
            let parts = [
                ("doc_id", doc_id.as_bytes().as_slice()),
                ("document", value.as_slice()),
            ];
            return ingest_collection_constellation(
                self.vault,
                col,
                "document",
                &parts,
                Modality::Structured,
            );
        }
        require_documents_mode(col)?;
        let rows = stage_doc_rows_at(self.vault, self.vault.latest_seq(), col, doc_id, doc)?;
        let prefix = document_prefix(col, doc_id);
        let subject = ledger_subject(&prefix);
        let payload = ledger_payload(col, doc_id, &prefix, &rows);
        self.vault.write_cf_batch_with_ledger_entry(
            rows,
            EntryKind::Ingest,
            subject,
            payload,
            ActorId::Service("calyx-aster-document".to_string()),
        )
    }

    pub fn get_doc(&self, col: &Collection, doc_id: DocId) -> Result<Option<Value>> {
        self.get_doc_at(self.vault.latest_seq(), col, doc_id)
    }

    pub fn get_doc_at(
        &self,
        snapshot: Seq,
        col: &Collection,
        doc_id: DocId,
    ) -> Result<Option<Value>> {
        self.get_subtree_at(snapshot, col, doc_id, &[])
    }

    pub fn get_subtree(
        &self,
        col: &Collection,
        doc_id: DocId,
        path: &[&str],
    ) -> Result<Option<Value>> {
        self.get_subtree_at(self.vault.latest_seq(), col, doc_id, path)
    }

    pub fn get_subtree_at(
        &self,
        snapshot: Seq,
        col: &Collection,
        doc_id: DocId,
        path: &[&str],
    ) -> Result<Option<Value>> {
        require_documents_mode(col)?;
        let base_path = path_segments(path)?;
        let prefix = document_path_prefix(col, doc_id, &base_path)?;
        let rows = self.vault.scan_cf_range_at(
            snapshot,
            ColumnFamily::Document,
            &prefix_range(&prefix),
        )?;
        let mut cells = Vec::new();
        for (key, value) in rows {
            let (stored_doc_id, stored_path) = parse_document_key(&key)?;
            if stored_doc_id != *doc_id.as_bytes() || !stored_path.starts_with(&base_path) {
                return Err(corrupt_doc("document scan returned an out-of-prefix key"));
            }
            if let Some(value) = decode_cell(&value)?.into_leaf_value()? {
                cells.push((stored_path[base_path.len()..].to_vec(), value));
            }
        }
        if cells.is_empty() {
            return Ok(None);
        }
        build_tree(cells).map(Some)
    }

    pub fn delete_doc(&self, col: &Collection, doc_id: DocId) -> Result<Seq> {
        require_documents_mode(col)?;
        // Read the prior version for index removal only when an index exists.
        let old_index_row = if collection_has_maintained_index(col) {
            self.get_doc(col, doc_id)?
                .map(|old| IndexMaintenance::document_row(col, &old))
                .transpose()?
        } else {
            None
        };
        let mut rows = Vec::new();
        for (key, value) in self.visible_doc_rows(col, doc_id)? {
            if matches!(decode_cell(&value)?, DocumentCell::Leaf(_)) {
                rows.push((
                    ColumnFamily::Document,
                    key,
                    encode_cell(&DocumentCell::Tombstone)?,
                ));
            }
        }
        if let Some(old_index_row) = &old_index_row {
            let doc_pk = RecordKey::from_bytes(doc_id.as_bytes().to_vec())?;
            IndexMaintenance::stage_delete(self.vault, &mut rows, col, &doc_pk, old_index_row)?;
        }
        let prefix = document_prefix(col, doc_id);
        let subject = ledger_subject(&prefix);
        let payload = ledger_payload(col, doc_id, &prefix, &rows);
        self.vault.write_cf_batch_with_ledger_entry(
            rows,
            EntryKind::Ingest,
            subject,
            payload,
            ActorId::Service("calyx-aster-document".to_string()),
        )
    }

    fn visible_doc_rows(&self, col: &Collection, doc_id: DocId) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.vault.scan_cf_range_at(
            self.vault.latest_seq(),
            ColumnFamily::Document,
            &prefix_range(&document_prefix(col, doc_id)),
        )
    }
}

impl<C> Layer for DocumentLayer<'_, C>
where
    C: Clock + Send + Sync,
{
    fn collection_mode() -> CollectionMode {
        CollectionMode::Documents
    }

    fn put(&self, col: &Collection, key: &[u8], value: &[u8]) -> Result<()> {
        let value = serde_json::from_slice(value)
            .map_err(|error| invalid_argument(format!("decode document JSON: {error}")))?;
        self.put_doc(col, DocId::from_slice(key)?, &value)?;
        Ok(())
    }

    fn get(&self, col: &Collection, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.get_doc(col, DocId::from_slice(key)?)?
            .map(|value| encode_json_value(&value))
            .transpose()
    }

    fn range(
        &self,
        col: &Collection,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<Vec<u8>>> {
        let start = DocId::from_slice(start)?;
        let end = DocId::from_slice(end)?;
        if limit == 0 || start >= end {
            return Ok(Vec::new());
        }
        let rows = self.vault.scan_cf_range_at(
            self.vault.latest_seq(),
            ColumnFamily::Document,
            &KeyRange {
                start: document_prefix(col, start),
                end: Some(document_prefix(col, end)),
            },
        )?;
        docs_from_rows(rows)?
            .into_iter()
            .take(limit)
            .map(|value| encode_json_value(&value))
            .collect()
    }
}

pub(crate) fn stage_doc_rows_at<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    doc_id: DocId,
    doc: &Value,
) -> Result<DocumentWriteRows> {
    require_documents_mode(col)?;
    validate_document(col, doc)?;
    let doc_pk = RecordKey::from_bytes(doc_id.as_bytes().to_vec())?;
    let mut flattened = Vec::new();
    flatten_document(&mut Vec::new(), doc, &mut flattened)?;
    let mut rows = Vec::with_capacity(flattened.len());
    let mut new_keys = BTreeSet::new();
    for (path, value) in flattened {
        let key = document_key_from_segments(col, doc_id, &path)?;
        let value = encode_cell(&DocumentCell::leaf(value)?)?;
        new_keys.insert(key.clone());
        rows.push((ColumnFamily::Document, key, value));
    }
    let old_rows = vault.scan_cf_range_at(
        snapshot,
        ColumnFamily::Document,
        &prefix_range(&document_prefix(col, doc_id)),
    )?;
    for (key, value) in old_rows {
        if !new_keys.contains(&key) && matches!(decode_cell(&value)?, DocumentCell::Leaf(_)) {
            rows.push((
                ColumnFamily::Document,
                key,
                encode_cell(&DocumentCell::Tombstone)?,
            ));
        }
    }
    if collection_has_maintained_index(col) {
        let old_index_row = DocumentLayer::new(vault)
            .get_doc_at(snapshot, col, doc_id)?
            .map(|old| IndexMaintenance::document_row(col, &old))
            .transpose()?;
        let new_index_row = IndexMaintenance::document_row(col, doc)?;
        IndexMaintenance::stage_put(
            vault,
            &mut rows,
            col,
            &doc_pk,
            old_index_row.as_ref(),
            &new_index_row,
        )?;
    }
    Ok(rows)
}

fn require_documents_mode(col: &Collection) -> Result<()> {
    if col.mode == CollectionMode::Documents {
        Ok(())
    } else {
        Err(invalid_argument(format!(
            "document layer requires Documents collection, got {:?}",
            col.mode
        )))
    }
}

fn ledger_subject(prefix: &[u8]) -> SubjectId {
    SubjectId::Query(blake3::hash(prefix).as_bytes().to_vec())
}

fn ledger_payload(
    col: &Collection,
    doc_id: DocId,
    prefix: &[u8],
    rows: &[(ColumnFamily, Vec<u8>, Vec<u8>)],
) -> Vec<u8> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("collection_id", format!("{:016x}", collection_id(col)))
        .insert_str("doc_id", hex_bytes(doc_id.as_bytes()))
        .insert_str("doc_hash", blake3::hash(prefix).to_hex().to_string())
        .insert_str("rows_hash", rows_hash(rows));
    RedactionPolicy::default().apply_to_payload(&payload)
}

fn rows_hash(rows: &[(ColumnFamily, Vec<u8>, Vec<u8>)]) -> String {
    let mut hasher = blake3::Hasher::new();
    for (_, key, value) in rows {
        hasher.update(&(key.len() as u64).to_be_bytes());
        hasher.update(key);
        hasher.update(&(value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    hasher.finalize().to_hex().to_string()
}

fn encode_json_value(value: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|error| corrupt_doc(format!("encode document JSON: {error}")))
}

#[cfg(test)]
mod tests;
