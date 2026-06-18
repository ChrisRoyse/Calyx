use std::path::Path;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, Row};

use super::errors;
use super::temporal::parse_event_time_secs;
use crate::error::{CliError, CliResult};

const GTE_EMBEDDING_DIM: usize = 768;
const GTE_EMBEDDING_BYTES: usize = GTE_EMBEDDING_DIM * std::mem::size_of::<f32>();
const FIXTURE_COLUMNS: [&str; 4] = ["chunk_id", "database_name", "content", "embedding"];
const LEAPABLE_CHUNK_COLUMNS: [&str; 3] = ["id", "text", "created_at"];

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum SourceSchema {
    CalyxFixture,
    LeapableVec,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChunkRow {
    pub row_num: u64,
    pub chunk_id: String,
    pub database_name: String,
    pub content: Vec<u8>,
    pub embedding: Vec<f32>,
    pub event_time_secs: Option<u64>,
    pub event_time_raw: Option<String>,
}

impl ChunkRow {
    pub fn content_hash(&self) -> [u8; 32] {
        *blake3::hash(&self.content).as_bytes()
    }

    pub fn pointer(&self) -> String {
        format!("sqlite://chunks/{}/{}", self.database_name, self.chunk_id)
    }
}

pub fn open_sqlite(path: &Path) -> CliResult<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(path, flags)
        .map_err(|err| CliError::io(format!("open sqlite {}: {err}", path.display())))
}

pub fn row_count(conn: &Connection) -> CliResult<u64> {
    match source_schema(conn)? {
        SourceSchema::CalyxFixture => {
            scalar_count(conn, "SELECT COUNT(*) FROM chunks", "count chunks")
        }
        SourceSchema::LeapableVec => {
            scalar_count(conn, "SELECT COUNT(*) FROM chunks", "count chunks")
        }
    }
}

pub fn stream_rows(conn: &Connection) -> CliResult<Vec<ChunkRow>> {
    match source_schema(conn)? {
        SourceSchema::CalyxFixture => stream_fixture_rows(conn),
        SourceSchema::LeapableVec => stream_leapable_rows(conn),
    }
}

pub fn read_chunk(conn: &Connection, chunk_id: &str) -> CliResult<ChunkRow> {
    match source_schema(conn)? {
        SourceSchema::CalyxFixture => read_fixture_chunk(conn, chunk_id),
        SourceSchema::LeapableVec => read_leapable_chunk(conn, chunk_id),
    }
}

fn source_schema(conn: &Connection) -> CliResult<SourceSchema> {
    let chunks = table_columns(conn, "chunks")?;
    if has_columns(&chunks, &FIXTURE_COLUMNS) {
        return Ok(SourceSchema::CalyxFixture);
    }
    let has_leapable_tables = has_leapable_vector_tables(conn)?;
    if has_columns(&chunks, &["id", "text"]) && has_leapable_tables {
        for required in LEAPABLE_CHUNK_COLUMNS {
            if !chunks.iter().any(|column| column == required) {
                return Err(errors::schema(format!(
                    "Leapable chunks table missing required column {required}"
                ))
                .into());
            }
        }
        validate_leapable_source(conn)?;
        return Ok(SourceSchema::LeapableVec);
    }
    for required in FIXTURE_COLUMNS {
        if !chunks.iter().any(|column| column == required) {
            return Err(
                errors::schema(format!("chunks table missing required column {required}")).into(),
            );
        }
    }
    Ok(SourceSchema::CalyxFixture)
}

fn has_leapable_vector_tables(conn: &Connection) -> CliResult<bool> {
    Ok(has_columns(
        &table_columns(conn, "database_metadata")?,
        &["database_name"],
    ) && has_columns(&table_columns(conn, "embeddings")?, &["id", "chunk_id"])
        && has_columns(
            &table_columns(conn, "vec_embeddings_rowids")?,
            &["id", "chunk_id", "chunk_offset"],
        )
        && has_columns(
            &table_columns(conn, "vec_embeddings_vector_chunks00")?,
            &["vectors"],
        ))
}

fn has_columns(columns: &[String], required: &[&str]) -> bool {
    required
        .iter()
        .all(|required| columns.iter().any(|column| column == required))
}

fn validate_leapable_source(conn: &Connection) -> CliResult {
    let chunks = scalar_count(conn, "SELECT COUNT(*) FROM chunks", "count Leapable chunks")?;
    let vector_rows = scalar_count(conn, LEAPABLE_JOIN_COUNT_SQL, "count Leapable vectors")?;
    if chunks != vector_rows {
        return Err(errors::schema(format!(
            "Leapable sqlite-vec source requires one embedding vector per chunk: chunks={chunks} vector_rows={vector_rows}"
        ))
        .into());
    }
    let invalid_offsets_sql = format!(
        "SELECT COUNT(*) FROM chunks c \
         JOIN embeddings e ON e.chunk_id = c.id \
         JOIN vec_embeddings_rowids r ON r.id = e.id \
         JOIN vec_embeddings_vector_chunks00 vc ON vc.rowid = r.chunk_id \
         WHERE r.chunk_offset IS NULL OR r.chunk_offset < 0 \
            OR length(vc.vectors) < ((r.chunk_offset + 1) * {GTE_EMBEDDING_BYTES})"
    );
    let invalid_offsets = scalar_count(
        conn,
        &invalid_offsets_sql,
        "validate Leapable vector offsets",
    )?;
    if invalid_offsets != 0 {
        return Err(errors::embedding(format!(
            "Leapable sqlite-vec source has {invalid_offsets} vector offsets outside backing blobs"
        ))
        .into());
    }
    Ok(())
}

const LEAPABLE_JOIN_COUNT_SQL: &str = "SELECT COUNT(*) FROM chunks c \
     JOIN embeddings e ON e.chunk_id = c.id \
     JOIN vec_embeddings_rowids r ON r.id = e.id \
     JOIN vec_embeddings_vector_chunks00 vc ON vc.rowid = r.chunk_id";

fn scalar_count(conn: &Connection, sql: &str, op: &'static str) -> CliResult<u64> {
    let count = conn
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .map_err(|err| errors::sqlite(op, err))?;
    u64::try_from(count)
        .map_err(|_| errors::schema(format!("SQLite row count {count} is negative")).into())
}

fn stream_fixture_rows(conn: &Connection) -> CliResult<Vec<ChunkRow>> {
    let has_created_at = table_columns(conn, "chunks")?
        .iter()
        .any(|column| column == "created_at");
    let sql = if has_created_at {
        "SELECT rowid, chunk_id, database_name, content, embedding, created_at \
         FROM chunks ORDER BY rowid"
    } else {
        "SELECT rowid, chunk_id, database_name, content, embedding, NULL AS created_at \
         FROM chunks ORDER BY rowid"
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|err| errors::sqlite("prepare chunk scan", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| errors::sqlite("query chunks", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| errors::sqlite("read chunk row", err))?
    {
        out.push(row_from_sqlite(row)?);
    }
    Ok(out)
}

fn stream_leapable_rows(conn: &Connection) -> CliResult<Vec<ChunkRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT c.rowid, c.id, \
                    (SELECT database_name FROM database_metadata ORDER BY id LIMIT 1), \
                    c.text, vc.vectors, r.chunk_offset, c.created_at \
             FROM chunks c \
             JOIN embeddings e ON e.chunk_id = c.id \
             JOIN vec_embeddings_rowids r ON r.id = e.id \
             JOIN vec_embeddings_vector_chunks00 vc ON vc.rowid = r.chunk_id \
             ORDER BY c.rowid",
        )
        .map_err(|err| errors::sqlite("prepare Leapable chunk scan", err))?;
    let mut rows = stmt
        .query([])
        .map_err(|err| errors::sqlite("query Leapable chunks", err))?;
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| errors::sqlite("read Leapable chunk row", err))?
    {
        out.push(row_from_leapable(row)?);
    }
    Ok(out)
}

fn read_fixture_chunk(conn: &Connection, chunk_id: &str) -> CliResult<ChunkRow> {
    let has_created_at = table_columns(conn, "chunks")?
        .iter()
        .any(|column| column == "created_at");
    let sql = if has_created_at {
        "SELECT rowid, chunk_id, database_name, content, embedding, created_at \
         FROM chunks WHERE chunk_id = ?1 ORDER BY rowid LIMIT 1"
    } else {
        "SELECT rowid, chunk_id, database_name, content, embedding, NULL AS created_at \
         FROM chunks WHERE chunk_id = ?1 ORDER BY rowid LIMIT 1"
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|err| errors::sqlite("prepare chunk read", err))?;
    let mut rows = stmt
        .query([chunk_id])
        .map_err(|err| errors::sqlite("query chunk", err))?;
    let Some(row) = rows
        .next()
        .map_err(|err| errors::sqlite("read chunk", err))?
    else {
        return Err(errors::schema(format!("chunk_id {chunk_id} not found")).into());
    };
    row_from_sqlite(row)
}

fn read_leapable_chunk(conn: &Connection, chunk_id: &str) -> CliResult<ChunkRow> {
    let mut stmt = conn
        .prepare(
            "SELECT c.rowid, c.id, \
                    (SELECT database_name FROM database_metadata ORDER BY id LIMIT 1), \
                    c.text, vc.vectors, r.chunk_offset, c.created_at \
             FROM chunks c \
             JOIN embeddings e ON e.chunk_id = c.id \
             JOIN vec_embeddings_rowids r ON r.id = e.id \
             JOIN vec_embeddings_vector_chunks00 vc ON vc.rowid = r.chunk_id \
             WHERE c.id = ?1 \
             ORDER BY c.rowid LIMIT 1",
        )
        .map_err(|err| errors::sqlite("prepare Leapable chunk read", err))?;
    let mut rows = stmt
        .query([chunk_id])
        .map_err(|err| errors::sqlite("query Leapable chunk", err))?;
    let Some(row) = rows
        .next()
        .map_err(|err| errors::sqlite("read Leapable chunk", err))?
    else {
        return Err(errors::schema(format!("chunk_id {chunk_id} not found")).into());
    };
    row_from_leapable(row)
}

fn table_columns(conn: &Connection, table: &str) -> CliResult<Vec<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|err| errors::sqlite("inspect sqlite schema", err))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| errors::sqlite("read sqlite schema", err))?;
    rows.map(|row| row.map_err(|err| errors::sqlite("decode schema row", err)))
        .collect::<calyx_core::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn row_from_sqlite(row: &Row<'_>) -> CliResult<ChunkRow> {
    let row_num = row_num(row)?;
    let (event_time_secs, event_time_raw) = optional_event_time(
        row.get_ref(5)
            .map_err(|err| errors::sqlite(&format!("read created_at at row {row_num}"), err))?,
        row_num,
    )?;
    Ok(ChunkRow {
        row_num,
        chunk_id: text_field(
            row.get_ref(1)
                .map_err(|err| errors::sqlite(&format!("read chunk_id at row {row_num}"), err))?,
            "chunk_id",
            row_num,
        )?,
        database_name: text_field(
            row.get_ref(2).map_err(|err| {
                errors::sqlite(&format!("read database_name at row {row_num}"), err)
            })?,
            "database_name",
            row_num,
        )?,
        content: value_bytes(
            row.get_ref(3)
                .map_err(|err| errors::sqlite(&format!("read content at row {row_num}"), err))?,
            "content",
            row_num,
        )?,
        embedding: decode_embedding(
            value_bytes(
                row.get_ref(4).map_err(|err| {
                    errors::sqlite(&format!("read embedding at row {row_num}"), err)
                })?,
                "embedding",
                row_num,
            )?,
            row_num,
        )?,
        event_time_secs,
        event_time_raw,
    })
}

fn row_from_leapable(row: &Row<'_>) -> CliResult<ChunkRow> {
    let row_num = row_num(row)?;
    let vectors = value_bytes(
        row.get_ref(4)
            .map_err(|err| errors::sqlite(&format!("read vectors at row {row_num}"), err))?,
        "vectors",
        row_num,
    )?;
    let chunk_offset: i64 = row
        .get(5)
        .map_err(|err| errors::sqlite(&format!("read chunk_offset at row {row_num}"), err))?;
    let (event_time_secs, event_time_raw) = required_event_time(
        row.get_ref(6)
            .map_err(|err| errors::sqlite(&format!("read created_at at row {row_num}"), err))?,
        row_num,
    )?;
    let offset = usize::try_from(chunk_offset).map_err(|_| {
        errors::embedding(format!(
            "row {row_num} sqlite-vec chunk_offset {chunk_offset} is negative"
        ))
    })?;
    let start = offset.checked_mul(GTE_EMBEDDING_BYTES).ok_or_else(|| {
        errors::embedding(format!("row {row_num} sqlite-vec vector offset overflow"))
    })?;
    let end = start.checked_add(GTE_EMBEDDING_BYTES).ok_or_else(|| {
        errors::embedding(format!("row {row_num} sqlite-vec vector end overflow"))
    })?;
    let vector = vectors.get(start..end).ok_or_else(|| {
        errors::embedding(format!(
            "row {row_num} sqlite-vec vector offset {chunk_offset} outside blob length {}",
            vectors.len()
        ))
    })?;
    Ok(ChunkRow {
        row_num,
        chunk_id: text_field(
            row.get_ref(1)
                .map_err(|err| errors::sqlite(&format!("read chunk_id at row {row_num}"), err))?,
            "chunk_id",
            row_num,
        )?,
        database_name: text_field(
            row.get_ref(2).map_err(|err| {
                errors::sqlite(&format!("read database_name at row {row_num}"), err)
            })?,
            "database_name",
            row_num,
        )?,
        content: value_bytes(
            row.get_ref(3)
                .map_err(|err| errors::sqlite(&format!("read content at row {row_num}"), err))?,
            "content",
            row_num,
        )?,
        embedding: decode_embedding(vector.to_vec(), row_num)?,
        event_time_secs: Some(event_time_secs),
        event_time_raw: Some(event_time_raw),
    })
}

fn row_num(row: &Row<'_>) -> CliResult<u64> {
    let rowid: i64 = row
        .get(0)
        .map_err(|err| errors::sqlite("read rowid", err))?;
    u64::try_from(rowid)
        .map_err(|_| errors::schema(format!("chunks rowid {rowid} is negative")).into())
}

fn text_field(value: ValueRef<'_>, field: &str, row_num: u64) -> CliResult<String> {
    let bytes = value_bytes(value, field, row_num)?;
    std::str::from_utf8(&bytes)
        .map(str::to_string)
        .map_err(|err| {
            errors::schema(format!(
                "row {row_num} {field} is not valid UTF-8: {err}; raw_hex={}",
                super::manifest::hex_encode(&bytes)
            ))
            .into()
        })
}

fn optional_event_time(
    value: ValueRef<'_>,
    row_num: u64,
) -> CliResult<(Option<u64>, Option<String>)> {
    match value {
        ValueRef::Null => Ok((None, None)),
        _ => required_event_time(value, row_num).map(|(secs, raw)| (Some(secs), Some(raw))),
    }
}

fn required_event_time(value: ValueRef<'_>, row_num: u64) -> CliResult<(u64, String)> {
    let raw = event_time_text(value, row_num)?;
    parse_event_time_secs(&raw, row_num, "created_at").map(|secs| (secs, raw))
}

fn event_time_text(value: ValueRef<'_>, row_num: u64) -> CliResult<String> {
    match value {
        ValueRef::Integer(value) => Ok(value.to_string()),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|err| {
                errors::schema(format!(
                    "row {row_num} created_at is not valid UTF-8: {err}; raw_hex={}",
                    super::manifest::hex_encode(bytes)
                ))
                .into()
            }),
        ValueRef::Null => Err(errors::schema(format!("row {row_num} created_at is NULL")).into()),
        ValueRef::Real(_) => {
            Err(errors::schema(format!("row {row_num} created_at must be TEXT or INTEGER")).into())
        }
    }
}

fn value_bytes(value: ValueRef<'_>, field: &str, row_num: u64) -> CliResult<Vec<u8>> {
    match value {
        ValueRef::Blob(bytes) | ValueRef::Text(bytes) => Ok(bytes.to_vec()),
        _ => Err(errors::schema(format!("row {row_num} {field} must be TEXT or BLOB")).into()),
    }
}

fn decode_embedding(bytes: Vec<u8>, row_num: u64) -> CliResult<Vec<f32>> {
    if bytes.len() != GTE_EMBEDDING_BYTES {
        return Err(errors::embedding(format!(
            "row {row_num} embedding byte length {} expected {GTE_EMBEDDING_BYTES}",
            bytes.len(),
        ))
        .into());
    }
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>();
    if values.iter().any(|value| !value.is_finite()) {
        return Err(
            errors::embedding(format!("row {row_num} embedding contains NaN or Inf")).into(),
        );
    }
    Ok(values)
}

#[cfg(test)]
mod tests;
