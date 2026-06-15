use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rusqlite::{Connection, params};

use super::dual_write::{CALYX_SHADOW_WRITE_FAILED, replay_existing_sqlite, verify_against};
use super::dual_write_typed::{
    CALYX_INVALID_TEXT_HASH, ChunkMeta, ChunkRecord, DualWriteIngest, TextHash,
};
use super::recall_comparator::{
    CALYX_INVALID_TOP_K, CALYX_INVALID_VECTOR, QuerySpec, RecallComparator,
};

#[test]
fn typed_ingest_writes_five_chunks_and_reingest_is_idempotent() {
    let root = temp_root("typed-five");
    let sqlite = root.join("typed.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    let mut ingest = DualWriteIngest::open(&sqlite, &vault).unwrap();
    let chunks = records(5);

    let first = ingest.batch_ingest(&chunks);
    let second = ingest.batch_ingest(&chunks);

    assert_eq!(first.receipts.len(), 5);
    assert!(first.failures.is_empty());
    assert_eq!(second.receipts, first.receipts);
    assert_eq!(sqlite_count(&sqlite), 5);
    for receipt in &first.receipts {
        let json = serde_json::to_value(receipt).unwrap().to_string();
        assert!(!json.contains("raw_text"));
        assert!(!json.contains("candidate_text"));
        assert!(!json.contains("persisted_text"));
    }
    cleanup(root);
}

#[test]
fn recall_comparator_identical_query_passes_v0_gate() {
    let root = temp_root("recall-pass");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_dual_write_sqlite(&sqlite, 5);
    let report = replay_existing_sqlite(&sqlite, &vault).unwrap();
    assert_eq!(report.gate, "PASS");
    let query = QuerySpec {
        query_vec: vector(3.0),
        expected_chunk_ids: vec!["c003".to_string()],
    };

    let parity = RecallComparator::compare(&sqlite, &vault, &[query], 1).unwrap();
    let verify = verify_against(&sqlite, &vault).unwrap();

    assert_eq!(parity.gate, "PASS");
    assert_eq!(parity.queries[0].sqlite_recall, 1.0);
    assert_eq!(parity.queries[0].calyx_recall, 1.0);
    assert_eq!(verify.verify.matched, 5);
    assert!(parity.report_path.is_file());
    assert!(
        !std::fs::read_to_string(parity.report_path)
            .unwrap()
            .contains("raw text")
    );
    cleanup(root);
}

#[test]
fn injected_shadow_failure_preserves_sqlite_row() {
    let root = temp_root("injected-failure");
    let sqlite = root.join("typed.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    let mut ingest = DualWriteIngest::open(&sqlite, &vault).unwrap();
    ingest.inject_shadow_failure_for_tests(true);
    let mut chunks = records(1);

    let error = ingest.ingest(chunks.remove(0)).unwrap_err();

    assert_eq!(error.code(), CALYX_SHADOW_WRITE_FAILED);
    assert_eq!(sqlite_count(&sqlite), 1);
    cleanup(root);
}

#[test]
fn comparator_rejects_zero_vector_and_zero_top_k() {
    let root = temp_root("bad-query");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_dual_write_sqlite(&sqlite, 2);
    replay_existing_sqlite(&sqlite, &vault).unwrap();
    let query = QuerySpec {
        query_vec: vec![0.0; 768],
        expected_chunk_ids: vec!["c000".to_string()],
    };

    let vector_error =
        RecallComparator::compare(&sqlite, &vault, std::slice::from_ref(&query), 1).unwrap_err();
    let topk_error = RecallComparator::compare(&sqlite, &vault, &[query], 0).unwrap_err();

    assert_eq!(vector_error.code(), CALYX_INVALID_VECTOR);
    assert_eq!(topk_error.code(), CALYX_INVALID_TOP_K);
    cleanup(root);
}

#[test]
fn text_hash_hex_requires_exact_32_bytes() {
    let error = TextHash::from_hex("abcd").unwrap_err();

    assert_eq!(error.code, CALYX_INVALID_TEXT_HASH);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1))]
    #[test]
    fn receipt_preserves_chunk_id_byte_exact(count in 1usize..20) {
        let root = temp_root("prop-receipts");
        let sqlite = root.join("typed.db");
        let vault = root.join("vault.calyx");
        std::fs::create_dir_all(&root).unwrap();
        let mut ingest = DualWriteIngest::open(&sqlite, &vault).unwrap();
        let chunks = records(count);

        let report = ingest.batch_ingest(&chunks);

        prop_assert!(report.failures.is_empty());
        for (idx, receipt) in report.receipts.iter().enumerate() {
            prop_assert_eq!(&receipt.chunk_id, &chunks[idx].chunk_id);
        }
        cleanup(root);
    }
}

fn records(count: usize) -> Vec<ChunkRecord> {
    (0..count)
        .map(|idx| ChunkRecord {
            chunk_id: format!("c{idx:03}"),
            text_hash: TextHash::from_bytes([idx as u8; 32]),
            vector: vector(idx as f32 + 1.0),
            metadata: ChunkMeta {
                database_name: "typed_db".to_string(),
            },
        })
        .collect()
}

fn seed_dual_write_sqlite(path: &Path, rows: usize) {
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE database_metadata(id INTEGER PRIMARY KEY, database_name TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE creator_databases(id INTEGER,database_name TEXT,created_at TEXT)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE queries(id INTEGER,database_name TEXT,query_text TEXT)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO database_metadata VALUES(1,'test_vault')", [])
        .unwrap();
    conn.execute(
        "INSERT INTO creator_databases VALUES(1,'test_vault','2026-06-15T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO queries VALUES(1,'test_vault','known query')",
        [],
    )
    .unwrap();
    for idx in 0..rows {
        conn.execute(
            "INSERT INTO chunks VALUES(?1,'test_vault',?2,?3)",
            params![
                format!("c{idx:03}"),
                format!("content-{idx}"),
                vector_blob(idx as f32)
            ],
        )
        .unwrap();
    }
}

fn vector(first: f32) -> Vec<f32> {
    std::iter::once(first)
        .chain((1..768).map(|idx| idx as f32 / 768.0))
        .collect()
}

fn vector_blob(first: f32) -> Vec<u8> {
    vector(first)
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn sqlite_count(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
        .unwrap()
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-dual-write-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn cleanup(root: PathBuf) {
    let _ = std::fs::remove_dir_all(root);
}
