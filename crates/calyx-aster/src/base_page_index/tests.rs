use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_ledger::LedgerHeadAnchor;

use super::*;
use crate::sst::write_sst;

#[test]
fn index_pages_preserve_latest_sorted_live_rows_and_skip_tombstones() {
    let root = temp_root("latest-live");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"old-a".as_slice()), (b"b", b"old-b")],
    )
    .unwrap();
    let tombstone = crate::mvcc::tombstone_value();
    write_sst(
        base.join("00000000000000000002.sst"),
        [(b"a".as_slice(), b"new-a".as_slice()), (b"b", &tombstone)],
    )
    .unwrap();

    let mut progress = Vec::new();
    let manifest = build_base_page_index(&root, 1, |event| {
        progress.push(event);
        Ok(())
    })
    .unwrap();
    let read_manifest = read_base_page_index_manifest(&root).unwrap();
    let rows = read_indexed_base_rows(&root, 10).unwrap();

    assert_eq!(manifest.total_entries, 2);
    assert_eq!(manifest.live_entries, 1);
    assert_eq!(manifest.tombstone_entries, 1);
    assert_eq!(read_manifest.pages.len(), 2);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.get(b"a".as_slice()).unwrap(), b"new-a");
    assert!(matches!(
        progress.last(),
        Some(BasePageIndexBuildProgress::Complete {
            live_entries: 1,
            ..
        })
    ));
    cleanup(root);
}

#[test]
fn missing_index_fails_closed_for_bounded_read() {
    let root = temp_root("missing");
    let error = read_indexed_base_rows(&root, 1).unwrap_err();

    assert_eq!(error.code, MISSING_CODE);
    assert!(error.message.contains("manifest is missing"));
    cleanup(root);
}

#[test]
fn stale_ledger_head_fails_closed() {
    let root = temp_root("stale-head");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"value".as_slice())],
    )
    .unwrap();
    build_base_page_index(&root, 4, |_| Ok(())).unwrap();
    fs::create_dir_all(root.join("ledger_head")).unwrap();
    let anchor = LedgerHeadAnchor::new(1, [1_u8; 32]).unwrap();
    fs::write(
        root.join("ledger_head").join("current.json"),
        serde_json::to_vec(&anchor).unwrap(),
    )
    .unwrap();

    let error = read_indexed_base_rows(&root, 1).unwrap_err();

    assert_eq!(error.code, STALE_CODE);
    assert!(error.message.contains("current head"));
    cleanup(root);
}

#[test]
fn page_sha_mismatch_fails_closed() {
    let root = temp_root("page-sha");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"value".as_slice())],
    )
    .unwrap();
    let manifest = build_base_page_index(&root, 4, |_| Ok(())).unwrap();
    fs::write(
        root.join(BASE_PAGE_INDEX_DIR).join(&manifest.pages[0].path),
        b"{\"entries\":[]}",
    )
    .unwrap();

    let error = read_indexed_base_rows(&root, 1).unwrap_err();

    assert_eq!(error.code, CORRUPT_CODE);
    assert!(error.message.contains("sha256 mismatch"));
    cleanup(root);
}

#[test]
fn indexed_key_read_returns_requested_rows_and_tombstones() {
    let root = temp_root("keyed-read");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    let tombstone = crate::mvcc::tombstone_value();
    write_sst(
        base.join("00000000000000000001.sst"),
        [
            (b"a".as_slice(), b"live".as_slice()),
            (b"b".as_slice(), tombstone.as_slice()),
        ],
    )
    .unwrap();
    build_base_page_index(&root, 1, |_| Ok(())).unwrap();

    let rows = read_indexed_base_rows_for_keys(
        &root,
        &[b"a".to_vec(), b"b".to_vec(), b"missing".to_vec()],
    )
    .unwrap();

    assert_eq!(rows.get(b"a".as_slice()).unwrap(), &Some(b"live".to_vec()));
    assert_eq!(rows.get(b"b".as_slice()).unwrap(), &Some(tombstone));
    assert_eq!(rows.get(b"missing".as_slice()).unwrap(), &None);
    cleanup(root);
}

#[test]
fn row_page_visitor_stops_after_first_verified_page() {
    let root = temp_root("page-visitor-stop");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [
            (b"a".as_slice(), b"value-a".as_slice()),
            (b"b".as_slice(), b"value-b".as_slice()),
            (b"c".as_slice(), b"value-c".as_slice()),
        ],
    )
    .unwrap();
    build_base_page_index(&root, 1, |_| Ok(())).unwrap();

    let mut seen = Vec::new();
    let live_rows_read = visit_indexed_base_row_pages(
        &root,
        |offset, rows| -> std::result::Result<bool, calyx_core::CalyxError> {
            let keys = rows
                .iter()
                .map(|(key, _)| String::from_utf8(key.clone()).unwrap())
                .collect::<Vec<_>>();
            seen.push((offset, keys));
            Ok(false)
        },
    )
    .unwrap();

    assert_eq!(live_rows_read, 1);
    assert_eq!(seen, vec![(0, vec!["a".to_string()])]);
    cleanup(root);
}

#[test]
fn advancing_index_head_preserves_pages_when_base_files_unchanged() {
    let root = temp_root("advance-head");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"value".as_slice())],
    )
    .unwrap();
    build_base_page_index(&root, 4, |_| Ok(())).unwrap();
    fs::create_dir_all(root.join("ledger_head")).unwrap();
    let anchor = LedgerHeadAnchor::new(7, [7_u8; 32]).unwrap();
    fs::write(
        root.join("ledger_head").join("current.json"),
        serde_json::to_vec(&anchor).unwrap(),
    )
    .unwrap();

    assert!(advance_base_page_index_head_if_base_unchanged(&root).unwrap());
    let manifest = read_base_page_index_manifest(&root).unwrap();

    assert_eq!(manifest.ledger_head_height, 7);
    assert_eq!(manifest.base_sst_files, 1);
    assert_eq!(read_indexed_base_rows(&root, 1).unwrap().len(), 1);
    cleanup(root);
}

#[test]
fn advancing_index_head_refuses_base_file_count_drift() {
    let root = temp_root("advance-head-drift");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"value".as_slice())],
    )
    .unwrap();
    build_base_page_index(&root, 4, |_| Ok(())).unwrap();
    write_sst(
        base.join("00000000000000000002.sst"),
        [(b"b".as_slice(), b"new".as_slice())],
    )
    .unwrap();
    fs::create_dir_all(root.join("ledger_head")).unwrap();
    let anchor = LedgerHeadAnchor::new(8, [8_u8; 32]).unwrap();
    fs::write(
        root.join("ledger_head").join("current.json"),
        serde_json::to_vec(&anchor).unwrap(),
    )
    .unwrap();

    let error = advance_base_page_index_head_if_base_unchanged(&root).unwrap_err();

    assert_eq!(error.code, STALE_CODE);
    assert!(error.message.contains("current vault has 2"));
    cleanup(root);
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("calyx-base-page-index-{name}-{nanos}"))
}

fn cleanup(path: PathBuf) {
    fs::remove_dir_all(path).ok();
}
