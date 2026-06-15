#![allow(dead_code)]

use std::fs;
use std::path::Path;

use calyx_core::{CalyxError, VaultStore};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::dual_write::{FailedIngest, IngestReceipt, aster_dir, shadow_write_failed};
use crate::error::{CliError, CliResult};
use crate::migrate;
use crate::migrate::adapter::{GTE_EMBEDDING_DIM, default_base_lens_id, default_panel_version};
use crate::migrate::manifest::{MigrationManifest, hex_decode, hex_encode};
use crate::migrate::reader::ChunkRow;
use crate::migrate::verifier::row_exists_and_matches;

pub(crate) const CALYX_INVALID_TEXT_HASH: &str = "CALYX_INVALID_TEXT_HASH";
pub(crate) const METADATA_TEXT_HASH: &str = "text_hash";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct TextHash([u8; 32]);

impl TextHash {
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub(crate) fn from_hex(value: &str) -> Result<Self, CalyxError> {
        let bytes = hex_decode(value)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
            error(
                CALYX_INVALID_TEXT_HASH,
                format!("text_hash decoded to {} bytes, expected 32", bytes.len()),
                "provide a 32-byte hex text hash; never provide raw candidate text",
            )
        })?;
        Ok(Self(bytes))
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn hex(&self) -> String {
        hex_encode(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChunkMeta {
    pub(crate) database_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChunkRecord {
    pub(crate) chunk_id: String,
    pub(crate) text_hash: TextHash,
    pub(crate) vector: Vec<f32>,
    pub(crate) metadata: ChunkMeta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct BatchIngestReport {
    pub(crate) receipts: Vec<IngestReceipt>,
    pub(crate) failures: Vec<FailedIngest>,
}

pub(crate) struct DualWriteIngest {
    conn: Connection,
    vault: calyx_aster::vault::AsterVault,
    adapter: crate::migrate::adapter::VaultSqliteAdapter,
    inject_shadow_failure: bool,
}

impl DualWriteIngest {
    pub(crate) fn open(sqlite_path: &Path, calyx_dir: &Path) -> CliResult<Self> {
        fs::create_dir_all(calyx_dir)
            .map_err(|err| CliError::io(format!("create {}: {err}", calyx_dir.display())))?;
        let conn = Connection::open(sqlite_path)
            .map_err(|err| CliError::io(format!("open sqlite {}: {err}", sqlite_path.display())))?;
        init_typed_sqlite(&conn)?;
        let aster = aster_dir(calyx_dir);
        let manifest = MigrationManifest::load_or_create(
            &aster,
            sqlite_path,
            &[],
            default_base_lens_id(),
            default_panel_version(),
        )?;
        manifest.write(&aster)?;
        let vault = migrate::open_vault(&aster, &manifest)?;
        let adapter = migrate::adapter(&manifest)?;
        Ok(Self {
            conn,
            vault,
            adapter,
            inject_shadow_failure: false,
        })
    }

    pub(crate) fn inject_shadow_failure_for_tests(&mut self, inject: bool) {
        self.inject_shadow_failure = inject;
    }

    pub(crate) fn ingest(&mut self, record: ChunkRecord) -> CliResult<IngestReceipt> {
        validate_record(&record)?;
        let rowid = upsert_typed_row(&self.conn, &record)?;
        let row = typed_chunk_row(rowid, &record);
        if self.inject_shadow_failure {
            return Err(shadow_write_failed(format!(
                "injected shadow write failure after sqlite row {rowid}"
            ))
            .into());
        }
        let mut cx = self.adapter.constellation(&row)?;
        cx.metadata
            .insert(METADATA_TEXT_HASH.to_string(), record.text_hash.hex());
        if !row_exists_and_matches(&self.vault, &row, &self.adapter)? {
            self.vault.put(cx)?;
            self.vault.flush()?;
        }
        Ok(receipt_for(&self.adapter, &row, record.text_hash))
    }

    pub(crate) fn batch_ingest(&mut self, chunks: &[ChunkRecord]) -> BatchIngestReport {
        let mut receipts = Vec::new();
        let mut failures = Vec::new();
        for chunk in chunks {
            match self.ingest(chunk.clone()) {
                Ok(receipt) => receipts.push(receipt),
                Err(error) => failures.push(FailedIngest {
                    chunk_id: chunk.chunk_id.clone(),
                    database_name: chunk.metadata.database_name.clone(),
                    code: error.code().to_string(),
                    message: error.message().to_string(),
                }),
            }
        }
        BatchIngestReport { receipts, failures }
    }
}

fn init_typed_sqlite(conn: &Connection) -> CliResult {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS chunks(\
         chunk_id TEXT NOT NULL,\
         database_name TEXT NOT NULL,\
         content BLOB NOT NULL,\
         embedding BLOB NOT NULL,\
         text_hash BLOB NOT NULL,\
         UNIQUE(chunk_id,database_name))",
        [],
    )
    .map_err(|err| CliError::io(format!("create typed chunks table: {err}")))?;
    Ok(())
}

fn upsert_typed_row(conn: &Connection, record: &ChunkRecord) -> CliResult<u64> {
    let embedding = embedding_blob(&record.vector);
    conn.execute(
        "INSERT OR IGNORE INTO chunks(chunk_id,database_name,content,embedding,text_hash) \
         VALUES(?1,?2,?3,?4,?5)",
        params![
            &record.chunk_id,
            &record.metadata.database_name,
            record.text_hash.as_bytes().as_slice(),
            &embedding,
            record.text_hash.as_bytes().as_slice(),
        ],
    )
    .map_err(|err| CliError::io(format!("insert typed chunk {}: {err}", record.chunk_id)))?;
    let rowid: i64 = conn
        .query_row(
            "SELECT rowid FROM chunks WHERE chunk_id=?1 AND database_name=?2",
            params![&record.chunk_id, &record.metadata.database_name],
            |row| row.get(0),
        )
        .map_err(|err| CliError::io(format!("read typed rowid {}: {err}", record.chunk_id)))?;
    u64::try_from(rowid).map_err(|_| CliError::io(format!("negative sqlite rowid {rowid}")))
}

fn typed_chunk_row(rowid: u64, record: &ChunkRecord) -> ChunkRow {
    ChunkRow {
        row_num: rowid,
        chunk_id: record.chunk_id.clone(),
        database_name: record.metadata.database_name.clone(),
        content: record.text_hash.as_bytes().to_vec(),
        embedding: record.vector.clone(),
    }
}

fn validate_record(record: &ChunkRecord) -> CliResult {
    if record.vector.len() != GTE_EMBEDDING_DIM || record.vector.iter().any(|v| !v.is_finite()) {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "chunk {} vector dim {} expected {GTE_EMBEDDING_DIM} finite f32 values",
            record.chunk_id,
            record.vector.len()
        ))
        .into());
    }
    Ok(())
}

fn embedding_blob(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn receipt_for(
    adapter: &crate::migrate::adapter::VaultSqliteAdapter,
    row: &ChunkRow,
    text_hash: TextHash,
) -> IngestReceipt {
    IngestReceipt {
        chunk_id: row.chunk_id.clone(),
        database_name: row.database_name.clone(),
        sqlite_rowid: row.row_num,
        cx_id: adapter.cx_id(row).to_string(),
        text_hash: text_hash.hex(),
    }
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
