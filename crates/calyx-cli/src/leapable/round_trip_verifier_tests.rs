use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rusqlite::{Connection, params};

use super::round_trip_verifier::{
    CALYX_CONTRACT_NAME_MISMATCH, CALYX_MANIFEST_CORRUPT, CALYX_ROUND_TRIP_MISMATCH, QueryVec,
    RoundTripVerifier, run_verify_round_trip,
};

#[test]
fn verifies_ten_chunk_fixture_byte_exact() {
    let root = temp_root("pass");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_fixture(&sqlite, "fixture_db", &(1..=10).collect::<Vec<_>>());
    migrate(&sqlite, &vault);

    let report = RoundTripVerifier::verify(&sqlite, &vault).unwrap();

    assert_eq!(report.total, 10);
    assert_eq!(report.matched, 10);
    assert!(report.mismatches.is_empty());
    assert!(report.gate_passes());
    cleanup(root);
}

#[test]
fn detects_text_hash_mismatch_with_chunk_id_and_hashes() {
    let root = temp_root("hash-mismatch");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_fixture(&sqlite, "fixture_db", &[1, 2, 3]);
    migrate(&sqlite, &vault);
    set_text_hash(&sqlite, "c002", [0xaa; 32]);

    let report = RoundTripVerifier::verify(&sqlite, &vault).unwrap();

    assert_eq!(report.mismatches.len(), 1);
    let mismatch = &report.mismatches[0];
    assert_eq!(mismatch.code, CALYX_ROUND_TRIP_MISMATCH);
    assert_eq!(mismatch.chunk_id, "c002");
    assert_eq!(mismatch.field, "text_hash");
    assert_eq!(mismatch.expected_hash, "aa".repeat(32));
    assert_ne!(mismatch.expected_hash, mismatch.actual_hash);
    cleanup(root);
}

#[test]
fn benchmark_recall_exact_fixture_passes() {
    let root = temp_root("benchmark");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_fixture(&sqlite, "fixture_db", &(1..=10).collect::<Vec<_>>());
    migrate(&sqlite, &vault);
    let queries = (1..=5)
        .map(|idx| QueryVec {
            query_vec: vector(idx as f32),
            expected_chunk_ids: vec![format!("c{idx:03}")],
        })
        .collect::<Vec<_>>();

    let bench = RoundTripVerifier::benchmark_recall(&sqlite, &vault, &queries, 1).unwrap();

    assert_eq!(bench.sqlite_mean_recall, 1.0);
    assert_eq!(bench.calyx_mean_recall, 1.0);
    assert_eq!(bench.queries.len(), queries.len());
    for (idx, row) in bench.queries.iter().enumerate() {
        let expected = vec![format!("c{:03}", idx + 1)];
        assert_eq!(row.sqlite_recall, 1.0, "{row:#?}");
        assert_eq!(row.calyx_recall, 1.0, "{row:#?}");
        assert_eq!(row.sqlite_top, expected, "{row:#?}");
        assert_eq!(row.calyx_top, expected, "{row:#?}");
        assert_eq!(row.gate, "PASS", "{row:#?}");
    }
    // The aggregate benchmark gate also includes a single-sample latency check.
    // This fixture is the deterministic exact-recall oracle.
    cleanup(root);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]
    #[test]
    fn verify_is_order_independent(seed in any::<u64>()) {
        let root = temp_root("prop-order");
        let sqlite = root.join("vault.db");
        let vault = root.join("vault.calyx");
        std::fs::create_dir_all(&root).unwrap();
        seed_fixture(&sqlite, "fixture_db", &permutation(seed, 10));
        migrate(&sqlite, &vault);

        let report = RoundTripVerifier::verify(&sqlite, &vault).unwrap();

        prop_assert_eq!(report.total, 10);
        prop_assert_eq!(report.matched, 10);
        prop_assert!(report.mismatches.is_empty());
        cleanup(root);
    }
}

#[test]
fn empty_database_is_vacuously_correct() {
    let root = temp_root("empty");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_fixture(&sqlite, "fixture_db", &[]);
    migrate(&sqlite, &vault);

    let report = RoundTripVerifier::verify(&sqlite, &vault).unwrap();

    assert_eq!(report.total, 0);
    assert_eq!(report.matched, 0);
    assert!(report.gate_passes());
    cleanup(root);
}

#[test]
fn database_name_mismatch_uses_distinct_error_code() {
    let root = temp_root("name-mismatch");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_fixture(&sqlite, "fixture_db", &[1, 2, 3]);
    migrate(&sqlite, &vault);
    Connection::open(&sqlite)
        .unwrap()
        .execute(
            "UPDATE chunks SET database_name='fixture_db ' WHERE chunk_id='c001'",
            [],
        )
        .unwrap();

    let report = RoundTripVerifier::verify(&sqlite, &vault).unwrap();

    assert_eq!(report.mismatches[0].code, CALYX_CONTRACT_NAME_MISMATCH);
    assert_eq!(report.mismatches[0].field, "database_name");
    cleanup(root);
}

#[test]
fn missing_calyx_constellation_reports_missing_field() {
    let root = temp_root("missing");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_fixture(&sqlite, "fixture_db", &[1, 2, 3]);
    migrate(&sqlite, &vault);
    insert_chunk(&sqlite, "fixture_db", 999);

    let report = RoundTripVerifier::verify(&sqlite, &vault).unwrap();

    assert!(
        report
            .mismatches
            .iter()
            .any(|m| m.chunk_id == "c999" && m.field == "missing")
    );
    cleanup(root);
}

#[test]
fn corrupt_manifest_fails_closed_without_report() {
    let root = temp_root("corrupt-manifest");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_fixture(&sqlite, "fixture_db", &[1]);
    migrate(&sqlite, &vault);
    std::fs::write(vault.join("migration-manifest.json"), b"not-json").unwrap();

    let error = RoundTripVerifier::verify(&sqlite, &vault).unwrap_err();

    assert_eq!(error.code, CALYX_MANIFEST_CORRUPT);
    cleanup(root);
}

#[test]
fn cli_writes_output_json_on_pass() {
    let root = temp_root("cli-output");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    let out = root.join("verify_report.json");
    std::fs::create_dir_all(&root).unwrap();
    seed_fixture(&sqlite, "fixture_db", &[1, 2]);
    migrate(&sqlite, &vault);

    run_verify_round_trip(&[
        "--sqlite".to_string(),
        sqlite.display().to_string(),
        "--calyx".to_string(),
        vault.display().to_string(),
        "--output".to_string(),
        out.display().to_string(),
    ])
    .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(out).unwrap()).unwrap();
    assert_eq!(json["verify"]["gate"], "PASS");
    assert_eq!(json["verify"]["matched"], 2);
    cleanup(root);
}

fn migrate(sqlite: &Path, vault: &Path) {
    crate::migrate::run(
        "vault",
        &[
            sqlite.display().to_string(),
            vault.display().to_string(),
            "--batch-size".to_string(),
            "4".to_string(),
        ],
    )
    .unwrap();
}

fn seed_fixture(path: &Path, database_name: &str, order: &[usize]) {
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE database_metadata(id INTEGER PRIMARY KEY, database_name TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content BLOB,embedding BLOB,text_hash BLOB)",
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
    conn.execute(
        "INSERT INTO database_metadata VALUES(1,?1)",
        [database_name],
    )
    .unwrap();
    for idx in order {
        insert_chunk_with_conn(&conn, database_name, *idx);
    }
}

fn insert_chunk(path: &Path, database_name: &str, idx: usize) {
    let conn = Connection::open(path).unwrap();
    insert_chunk_with_conn(&conn, database_name, idx);
}

fn insert_chunk_with_conn(conn: &Connection, database_name: &str, idx: usize) {
    let content = content(idx);
    let text_hash = blake3::hash(&content);
    conn.execute(
        "INSERT INTO chunks(chunk_id,database_name,content,embedding,text_hash) VALUES(?1,?2,?3,?4,?5)",
        params![
            format!("c{idx:03}"),
            database_name,
            content,
            vector_blob(idx as f32),
            text_hash.as_bytes().as_slice()
        ],
    )
    .unwrap();
}

fn set_text_hash(path: &Path, chunk_id: &str, hash: [u8; 32]) {
    Connection::open(path)
        .unwrap()
        .execute(
            "UPDATE chunks SET text_hash=?1 WHERE chunk_id=?2",
            params![hash.as_slice(), chunk_id],
        )
        .unwrap();
}

fn content(idx: usize) -> Vec<u8> {
    format!("seed=0xBEEF_CAFE;chunk={idx:03}").into_bytes()
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

fn permutation(seed: u64, count: usize) -> Vec<usize> {
    let mut out = (1..=count).collect::<Vec<_>>();
    let mut state = seed | 1;
    for idx in (1..out.len()).rev() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        out.swap(idx, (state as usize) % (idx + 1));
    }
    out
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-round-trip-{name}-{}-{}",
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
