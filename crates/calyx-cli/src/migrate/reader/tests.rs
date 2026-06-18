use super::*;

#[test]
fn streams_rows_in_rowid_order_and_preserves_empty_identity_fields() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();
    for (chunk_id, database_name, content, first) in [
        ("c3", "db", "gamma", 3.0),
        ("", "db", "empty chunk", 2.0),
        ("c1", "", "empty db", 1.0),
    ] {
        conn.execute(
            "INSERT INTO chunks VALUES(?1,?2,?3,?4)",
            (chunk_id, database_name, content, embedding_blob(first)),
        )
        .unwrap();
    }

    let rows = stream_rows(&conn).unwrap();

    assert_eq!(row_count(&conn).unwrap(), 3);
    assert_eq!(
        rows.iter().map(|row| row.row_num).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(rows[0].content, b"gamma");
    assert_eq!(rows[0].embedding.len(), GTE_EMBEDDING_DIM);
    assert_eq!(rows[0].embedding[0], 3.0);
    assert_eq!(rows[0].event_time_secs, None);
    assert_eq!(rows[1].chunk_id, "");
    assert_eq!(rows[2].database_name, "");
}

#[test]
fn streams_leapable_sqlite_vec_rows_from_backing_tables() {
    let conn = Connection::open_in_memory().unwrap();
    create_leapable_schema(&conn);
    insert_leapable_chunk(&conn, "chunk-a", "embed-a", 1, 0, "alpha text");
    insert_leapable_chunk(&conn, "chunk-b", "embed-b", 1, 1, "beta text");
    conn.execute(
        "INSERT INTO vec_embeddings_vector_chunks00(rowid, vectors) VALUES(1, ?1)",
        [vectors_blob(&[11.0, 22.0])],
    )
    .unwrap();

    let rows = stream_rows(&conn).unwrap();
    let read = read_chunk(&conn, "chunk-b").unwrap();

    assert_eq!(row_count(&conn).unwrap(), 2);
    assert_eq!(rows[0].chunk_id, "chunk-a");
    assert_eq!(rows[0].database_name, "contracts-general");
    assert_eq!(rows[0].content, b"alpha text");
    assert_eq!(rows[0].embedding[0], 11.0);
    assert_eq!(rows[0].event_time_secs, Some(1_704_204_000));
    assert_eq!(rows[1].embedding[0], 22.0);
    assert_eq!(read.content, b"beta text");
    assert_eq!(read.embedding[0], 22.0);
    assert_eq!(read.event_time_raw.as_deref(), Some("2024-01-02T14:00:00Z"));
}

#[test]
fn fixture_created_at_parses_as_source_event_time() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE chunks(
            chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB,created_at TEXT)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks VALUES('c1','db','alpha',?1,'2024-01-02 14:00:00')",
        [embedding_blob(1.0)],
    )
    .unwrap();

    let rows = stream_rows(&conn).unwrap();

    assert_eq!(rows[0].event_time_secs, Some(1_704_204_000));
    assert_eq!(
        rows[0].event_time_raw.as_deref(),
        Some("2024-01-02 14:00:00")
    );
}

#[test]
fn malformed_fixture_created_at_fails_closed() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE chunks(
            chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB,created_at TEXT)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks VALUES('c1','db','alpha',?1,'now')",
        [embedding_blob(1.0)],
    )
    .unwrap();

    let error = stream_rows(&conn).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
    assert!(error.message().contains("created_at"));
    assert!(error.message().contains("invalid source event timestamp"));
}

#[test]
fn leapable_schema_without_one_vector_per_chunk_fails_closed() {
    let conn = Connection::open_in_memory().unwrap();
    create_leapable_schema(&conn);
    insert_leapable_chunk(&conn, "chunk-a", "embed-a", 1, 0, "alpha text");
    conn.execute(
        "INSERT INTO chunks(id, document_id, ocr_result_id, text, text_hash, chunk_index,
         character_start, character_end, overlap_previous, overlap_next, provenance_id,
         created_at, embedding_status)
         VALUES('chunk-b','doc','ocr','beta text','hash-b',1,0,9,0,0,'prov-b',
                '2024-01-02T14:00:00Z','complete')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO vec_embeddings_vector_chunks00(rowid, vectors) VALUES(1, ?1)",
        [vectors_blob(&[11.0])],
    )
    .unwrap();

    let error = stream_rows(&conn).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
    assert!(error.message().contains("one embedding vector per chunk"));
    assert!(error.message().contains("chunks=2"));
    assert!(error.message().contains("vector_rows=1"));
}

#[test]
fn leapable_without_created_at_fails_on_timestamp_column() {
    let conn = Connection::open_in_memory().unwrap();
    create_leapable_schema_without_created_at(&conn);

    let error = source_schema(&conn).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
    assert!(error.message().contains("created_at"));
}

#[test]
fn malformed_leapable_created_at_fails_closed_after_vector_join() {
    let conn = Connection::open_in_memory().unwrap();
    create_leapable_schema(&conn);
    insert_leapable_chunk(&conn, "chunk-a", "embed-a", 1, 0, "alpha text");
    conn.execute(
        "UPDATE chunks SET created_at = 'now' WHERE id = 'chunk-a'",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO vec_embeddings_vector_chunks00(rowid, vectors) VALUES(1, ?1)",
        [vectors_blob(&[11.0])],
    )
    .unwrap();

    let error = stream_rows(&conn).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
    assert!(error.message().contains("row 1"));
    assert!(error.message().contains("created_at"));
}

#[test]
fn missing_required_schema_column_fails_closed() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT)",
        [],
    )
    .unwrap();

    let error = source_schema(&conn).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
    assert!(error.message().contains("embedding"));
    assert!(error.remediation().contains("Leapable Vault SQLite DB"));
}

#[test]
fn exact_gte_embedding_blob_decodes_first_little_endian_float() {
    let conn = one_row_db("c1", "db", "alpha", embedding_blob(1.0));
    let rows = stream_rows(&conn).unwrap();

    assert_eq!(rows[0].embedding.len(), GTE_EMBEDDING_DIM);
    assert_eq!(rows[0].embedding[0], 1.0);
}

#[test]
fn wrong_embedding_size_reports_row_number() {
    let conn = one_row_db("c1", "db", "alpha", vec![0_u8; GTE_EMBEDDING_BYTES - 4]);

    let error = stream_rows(&conn).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_EMBEDDING_FORMAT);
    assert!(error.message().contains("row 1"));
    assert!(error.message().contains("3068"));
}

#[test]
fn empty_chunks_table_streams_zero_rows() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();

    assert_eq!(row_count(&conn).unwrap(), 0);
    assert_eq!(stream_rows(&conn).unwrap(), Vec::new());
}

#[test]
fn non_utf8_chunk_id_fails_with_row_number() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id BLOB,database_name TEXT,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks VALUES(?1,'db','alpha',?2)",
        (vec![0xff, 0xfe], embedding_blob(1.0)),
    )
    .unwrap();

    let error = stream_rows(&conn).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
    assert!(error.message().contains("row 1"));
    assert!(error.message().contains("raw_hex=fffe"));
}

#[test]
fn null_embedding_fails_with_row_number() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO chunks VALUES('c1','db','alpha',NULL)", [])
        .unwrap();

    let error = stream_rows(&conn).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
    assert!(error.message().contains("row 1"));
    assert!(error.message().contains("embedding"));
}

#[test]
fn non_utf8_database_name_reports_raw_bytes() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name BLOB,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks VALUES('c1',?1,'alpha',?2)",
        (vec![0xff, 0x00], embedding_blob(1.0)),
    )
    .unwrap();

    let error = stream_rows(&conn).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
    assert!(error.message().contains("row 1"));
    assert!(error.message().contains("database_name"));
    assert!(error.message().contains("raw_hex=ff00"));
}

fn create_leapable_schema(conn: &Connection) {
    conn.execute(
        "CREATE TABLE database_metadata(id INTEGER PRIMARY KEY, database_name TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO database_metadata(id, database_name) VALUES(1, 'contracts-general')",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE chunks(
            id TEXT PRIMARY KEY, document_id TEXT NOT NULL, ocr_result_id TEXT NOT NULL,
            text TEXT NOT NULL, text_hash TEXT NOT NULL, chunk_index INTEGER NOT NULL,
            character_start INTEGER NOT NULL, character_end INTEGER NOT NULL,
            overlap_previous INTEGER NOT NULL, overlap_next INTEGER NOT NULL,
            provenance_id TEXT NOT NULL, created_at TEXT NOT NULL,
            embedding_status TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE embeddings(id TEXT PRIMARY KEY, chunk_id TEXT, document_id TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE vec_embeddings_rowids(
            rowid INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT UNIQUE NOT NULL,
            chunk_id INTEGER,
            chunk_offset INTEGER)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE vec_embeddings_vector_chunks00(rowid PRIMARY KEY, vectors BLOB NOT NULL)",
        [],
    )
    .unwrap();
}

fn create_leapable_schema_without_created_at(conn: &Connection) {
    conn.execute(
        "CREATE TABLE database_metadata(id INTEGER PRIMARY KEY, database_name TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE chunks(id TEXT PRIMARY KEY, text TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE embeddings(id TEXT PRIMARY KEY, chunk_id TEXT, document_id TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE vec_embeddings_rowids(id TEXT UNIQUE NOT NULL, chunk_id INTEGER, chunk_offset INTEGER)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE vec_embeddings_vector_chunks00(rowid PRIMARY KEY, vectors BLOB NOT NULL)",
        [],
    )
    .unwrap();
}

fn insert_leapable_chunk(
    conn: &Connection,
    chunk_id: &str,
    embedding_id: &str,
    storage_chunk_id: i64,
    chunk_offset: i64,
    text: &str,
) {
    conn.execute(
        "INSERT INTO chunks(id, document_id, ocr_result_id, text, text_hash, chunk_index,
         character_start, character_end, overlap_previous, overlap_next, provenance_id,
         created_at, embedding_status)
         VALUES(?1,'doc','ocr',?2,'hash',?3,0,length(?2),0,0,?4,
                '2024-01-02T14:00:00Z','complete')",
        (chunk_id, text, chunk_offset, format!("prov-{chunk_id}")),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO embeddings(id, chunk_id, document_id) VALUES(?1, ?2, 'doc')",
        (embedding_id, chunk_id),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO vec_embeddings_rowids(id, chunk_id, chunk_offset) VALUES(?1, ?2, ?3)",
        (embedding_id, storage_chunk_id, chunk_offset),
    )
    .unwrap();
}

fn one_row_db(
    chunk_id: &str,
    database_name: &str,
    content: &str,
    embedding: Vec<u8>,
) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks VALUES(?1,?2,?3,?4)",
        (chunk_id, database_name, content, embedding),
    )
    .unwrap();
    conn
}

fn vectors_blob(first_values: &[f32]) -> Vec<u8> {
    first_values
        .iter()
        .flat_map(|first| embedding_blob(*first))
        .collect()
}

fn embedding_blob(first: f32) -> Vec<u8> {
    std::iter::once(first)
        .chain((1..GTE_EMBEDDING_DIM).map(|idx| idx as f32 / 10.0))
        .flat_map(|value| value.to_le_bytes())
        .collect()
}
