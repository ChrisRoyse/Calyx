use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_ledger::LedgerHeadAnchor;

use super::*;
use crate::manifest::{ImmutableRef, ManifestStore, VaultManifest};
use crate::sst::{SstReader, write_sst};
use crate::vault::encode::{WriteRow, encode_write_batch};
use crate::wal::{Wal, WalOptions};

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
fn ledger_head_advance_alone_keeps_index_fresh() {
    // Issue #1861: an Answer/Guard/Assay-only ledger append advances the global
    // ledger head and writes non-Base WAL rows, but does not change any Base
    // constellation byte. The byte-identical Base index must stay fresh -- no
    // false CALYX_BASE_PAGE_INDEX_STALE, no forced rebuild.
    let root = temp_root("ledger-head-advance-fresh");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"value".as_slice())],
    )
    .unwrap();
    let manifest = build_base_page_index(&root, 4, |_| Ok(())).unwrap();

    // Simulate the exact bug scenario: a non-Base ledger/guard append lands in
    // the WAL and the global ledger head advances.
    let mut wal = Wal::open(root.join("wal"), WalOptions::default()).unwrap();
    wal.append(
        &encode_write_batch(&[
            WriteRow {
                cf: ColumnFamily::Ledger,
                key: b"answer-seq".to_vec(),
                value: b"answer-entry".to_vec(),
            },
            WriteRow {
                cf: ColumnFamily::Guard,
                key: b"guard-key".to_vec(),
                value: b"guard-profile".to_vec(),
            },
        ])
        .unwrap(),
    )
    .unwrap();
    drop(wal);
    fs::create_dir_all(root.join("ledger_head")).unwrap();
    let anchor = LedgerHeadAnchor::new(77_401, [9_u8; 32]).unwrap();
    fs::write(
        root.join("ledger_head").join("current.json"),
        serde_json::to_vec(&anchor).unwrap(),
    )
    .unwrap();

    // The read is accepted fresh; the on-disk manifest digest is unchanged.
    let rows = read_indexed_base_rows(&root, 1).unwrap();
    assert_eq!(rows.get(b"a".as_slice()).unwrap(), b"value");
    let after = read_base_page_index_manifest(&root).unwrap();
    assert_eq!(
        after.base_source_digest_hex,
        manifest.base_source_digest_hex
    );
    assert_eq!(after.version, INDEX_VERSION);
    cleanup(root);
}

#[test]
fn base_put_after_build_stales_index() {
    // A real Base put (a new Base SST) must change the freshness digest and
    // force a rebuild via CALYX_BASE_PAGE_INDEX_STALE.
    let root = temp_root("base-put-stale");
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
        [(b"b".as_slice(), b"new-base".as_slice())],
    )
    .unwrap();

    let error = read_indexed_base_rows(&root, 1).unwrap_err();

    assert_eq!(error.code, STALE_CODE);
    assert!(error.message.contains("Base source digest"));
    assert!(error.message.contains("Base column family changed"));
    cleanup(root);
}

#[test]
fn base_wal_tombstone_after_build_stales_index() {
    // A Base put+tombstone that lands in the WAL tail (no flush, durable_seq
    // unchanged) still changes the Base-source digest and must stale the index.
    let root = temp_root("base-wal-tombstone-stale");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"value".as_slice())],
    )
    .unwrap();
    build_base_page_index(&root, 4, |_| Ok(())).unwrap();

    let tombstone = crate::mvcc::tombstone_value();
    let mut wal = Wal::open(root.join("wal"), WalOptions::default()).unwrap();
    wal.append(
        &encode_write_batch(&[WriteRow {
            cf: ColumnFamily::Base,
            key: b"a".to_vec(),
            value: tombstone,
        }])
        .unwrap(),
    )
    .unwrap();
    drop(wal);

    let error = read_indexed_base_rows(&root, 1).unwrap_err();

    assert_eq!(error.code, STALE_CODE);
    assert!(error.message.contains("Base source digest"));
    cleanup(root);
}

#[test]
fn prior_version_manifest_fails_stale_until_rebuilt() {
    // A v3 (pre-digest) manifest is structurally valid but must be rejected as
    // stale by the version gate so it is rebuilt into the digest contract.
    let root = temp_root("prior-version-stale");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"value".as_slice())],
    )
    .unwrap();
    let mut manifest = build_base_page_index(&root, 4, |_| Ok(())).unwrap();
    // Downgrade the committed manifest to the prior on-disk version.
    manifest.version = PRIOR_INDEX_VERSION;
    manifest.base_source_digest_hex = String::new();
    manifest.base_durable_seq = 0;
    fs::write(
        root.join(BASE_PAGE_INDEX_DIR)
            .join(BASE_PAGE_INDEX_MANIFEST),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let error = read_indexed_base_rows(&root, 1).unwrap_err();

    assert_eq!(error.code, STALE_CODE);
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
fn selected_key_visitor_reads_each_touched_page_once_and_deduplicates_keys() {
    let root = temp_root("selected-key-visitor");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [
            (b"a".as_slice(), b"value-a".as_slice()),
            (b"b".as_slice(), b"value-b".as_slice()),
            (b"c".as_slice(), b"value-c".as_slice()),
            (b"d".as_slice(), b"value-d".as_slice()),
        ],
    )
    .unwrap();
    build_base_page_index(&root, 2, |_| Ok(())).unwrap();
    let mut seen = Vec::new();

    let stats = visit_indexed_base_rows_for_keys(
        &root,
        &[
            b"d".to_vec(),
            b"a".to_vec(),
            b"b".to_vec(),
            b"b".to_vec(),
            b"z".to_vec(),
        ],
        |key, value| -> std::result::Result<(), calyx_core::CalyxError> {
            seen.push((key.to_vec(), value));
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(stats.unique_keys, 4);
    assert_eq!(stats.touched_pages, 2);
    assert_eq!(stats.source_files, 1);
    assert_eq!(stats.live_rows, 3);
    assert_eq!(stats.missing_rows, 1);
    seen.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        seen,
        vec![
            (b"a".to_vec(), Some(b"value-a".to_vec())),
            (b"b".to_vec(), Some(b"value-b".to_vec())),
            (b"d".to_vec(), Some(b"value-d".to_vec())),
            (b"z".to_vec(), None),
        ]
    );
    cleanup(root);
}

#[test]
fn selected_key_visitor_empty_request_reads_no_pages() {
    let root = temp_root("selected-key-empty");
    build_base_page_index(&root, 2, |_| Ok(())).unwrap();

    let stats = visit_indexed_base_rows_for_keys(
        &root,
        &[],
        |_, _| -> std::result::Result<(), calyx_core::CalyxError> {
            panic!("empty request must not invoke visitor")
        },
    )
    .unwrap();

    assert_eq!(stats, SelectedBaseRowsVisit::default());
    cleanup(root);
}

#[test]
fn selected_wal_rows_use_exact_offsets_and_open_one_source_file() {
    let root = temp_root("selected-wal-offsets");
    let mut wal = Wal::open(root.join("wal"), WalOptions::default()).unwrap();
    let large_unrelated_slot = vec![7_u8; 2 * 1024 * 1024];
    let payload = encode_write_batch(&[
        WriteRow {
            cf: ColumnFamily::Base,
            key: b"a".to_vec(),
            value: b"base-a".to_vec(),
        },
        WriteRow {
            cf: ColumnFamily::slot_raw(calyx_core::SlotId::new(3)),
            key: b"a".to_vec(),
            value: large_unrelated_slot,
        },
        WriteRow {
            cf: ColumnFamily::Base,
            key: b"b".to_vec(),
            value: b"base-b".to_vec(),
        },
    ])
    .unwrap();
    wal.append(&payload).unwrap();
    drop(wal);
    let manifest = build_base_page_index(&root, 8, |_| Ok(())).unwrap();
    let page = read_page(&root, &manifest.pages[0]).unwrap();
    assert!(page.entries.iter().all(|entry| matches!(
        entry.source,
        BasePageIndexSource::Wal {
            row_offset: Some(_),
            ..
        }
    )));
    let mut seen = BTreeMap::new();

    let stats = visit_indexed_base_rows_for_keys(
        &root,
        &[b"b".to_vec(), b"a".to_vec()],
        |key, value| -> std::result::Result<(), CalyxError> {
            seen.insert(key.to_vec(), value.unwrap());
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(stats.source_files, 1);
    assert_eq!(stats.live_rows, 2);
    assert_eq!(seen[b"a".as_slice()], b"base-a");
    assert_eq!(seen[b"b".as_slice()], b"base-b");
    cleanup(root);
}

#[test]
fn checkpointed_wal_payload_is_skipped_and_sst_remains_the_source() {
    let root = temp_root("checkpointed-wal-skip");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"base-a".as_slice())],
    )
    .unwrap();
    let mut wal = Wal::open(root.join("wal"), WalOptions::default()).unwrap();
    let ack = wal
        .append(
            &encode_write_batch(&[
                WriteRow {
                    cf: ColumnFamily::Base,
                    key: b"a".to_vec(),
                    value: b"base-a".to_vec(),
                },
                WriteRow {
                    cf: ColumnFamily::slot_raw(calyx_core::SlotId::new(3)),
                    key: b"a".to_vec(),
                    value: vec![9_u8; 2 * 1024 * 1024],
                },
            ])
            .unwrap(),
        )
        .unwrap();
    drop(wal);
    let store = ManifestStore::open(&root);
    let mut durable = store.load_current().unwrap();
    durable.manifest_seq += 1;
    durable.durable_seq = ack.seq;
    store.write_current(&durable).unwrap();

    let manifest = build_base_page_index(&root, 8, |_| Ok(())).unwrap();
    let page = read_page(&root, &manifest.pages[0]).unwrap();

    assert_eq!(manifest.wal_records, 0);
    assert!(matches!(
        page.entries[0].source,
        BasePageIndexSource::Sst {
            record_offset: Some(_),
            ..
        }
    ));
    assert_eq!(
        read_indexed_base_rows_for_keys(&root, &[b"a".to_vec()]).unwrap()[b"a".as_slice()],
        Some(b"base-a".to_vec())
    );
    cleanup(root);
}

#[test]
fn generation_without_exact_source_offsets_fails_stale() {
    let root = temp_root("legacy-source-offset");
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"value".as_slice())],
    )
    .unwrap();
    let mut manifest = build_base_page_index(&root, 4, |_| Ok(())).unwrap();
    let page_path = root.join(BASE_PAGE_INDEX_DIR).join(&manifest.pages[0].path);
    let mut page: BasePageIndexPage =
        serde_json::from_slice(&fs::read(&page_path).unwrap()).unwrap();
    let BasePageIndexSource::Sst { record_offset, .. } = &mut page.entries[0].source else {
        panic!("fixture must use an SST source")
    };
    *record_offset = None;
    let page_bytes = serde_json::to_vec(&page).unwrap();
    fs::write(&page_path, &page_bytes).unwrap();
    manifest.pages[0].sha256_hex = sha256_hex(&page_bytes);
    fs::write(
        root.join(BASE_PAGE_INDEX_DIR)
            .join(BASE_PAGE_INDEX_MANIFEST),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let error = read_indexed_base_rows_for_keys(&root, &[b"a".to_vec()]).unwrap_err();

    assert_eq!(error.code, STALE_CODE);
    assert!(error.message.contains("exact record offset"));
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
fn interruption_after_page_write_keeps_previous_commit_point_readable() {
    let (root, rows, snapshot, previous) = publication_fixture("interrupt-page");
    let error = write_index_with_hook(
        &root,
        1,
        rows,
        snapshot,
        |event| {
            if matches!(event, BasePageIndexBuildProgress::PageWritten { .. }) {
                return Err(CalyxError::disk_pressure(
                    "test interruption after durable page write",
                ));
            }
            Ok(())
        },
        |_| Ok(()),
    )
    .unwrap_err();

    assert_eq!(error.code, "CALYX_DISK_PRESSURE");
    // Crash safety: the previous commit-point manifest bytes survive intact.
    assert_eq!(read_base_page_index_manifest(&root).unwrap(), previous);
    // The fixture wrote a new Base SST (the interrupted generation's source), so
    // under the digest freshness contract (issue #1861) the surviving prior
    // index is correctly detected stale rather than serving a partial mix.
    assert_eq!(
        read_indexed_base_rows(&root, 1).unwrap_err().code,
        STALE_CODE
    );
    cleanup(root);
}

#[test]
fn interruption_before_commit_point_keeps_previous_generation_readable() {
    for boundary in [
        PublicationBoundary::GenerationManifestSynced,
        PublicationBoundary::GenerationPublished,
    ] {
        let (root, rows, snapshot, previous) =
            publication_fixture(&format!("interrupt-{boundary:?}"));
        let error = write_index_with_hook(
            &root,
            1,
            rows,
            snapshot,
            |_| Ok(()),
            |observed| {
                if observed == boundary {
                    return Err(CalyxError::disk_pressure(format!(
                        "test interruption at {boundary:?}"
                    )));
                }
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(error.code, "CALYX_DISK_PRESSURE");
        // Crash safety: the previous commit-point manifest bytes survive intact.
        assert_eq!(read_base_page_index_manifest(&root).unwrap(), previous);
        // The interrupted generation's new Base SST correctly stales the
        // surviving prior index under the digest freshness contract.
        assert_eq!(
            read_indexed_base_rows(&root, 1).unwrap_err().code,
            STALE_CODE
        );
        cleanup(root);
    }
}

#[test]
fn interruption_after_commit_point_exposes_only_complete_new_generation() {
    let (root, rows, snapshot, previous) = publication_fixture("interrupt-after-commit");
    let error = write_index_with_hook(
        &root,
        1,
        rows,
        snapshot,
        |_| Ok(()),
        |boundary| {
            if boundary == PublicationBoundary::CommitPointPublished {
                return Err(CalyxError::disk_pressure(
                    "test interruption after commit-point publication",
                ));
            }
            Ok(())
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "CALYX_DISK_PRESSURE");
    let published = read_base_page_index_manifest(&root).unwrap();
    assert_ne!(published.generation, previous.generation);
    assert_eq!(published.version, INDEX_VERSION);
    assert_eq!(
        read_indexed_base_rows(&root, 1).unwrap()[b"a".as_slice()],
        b"new"
    );
    cleanup(root);
}

fn publication_fixture(
    name: &str,
) -> (
    PathBuf,
    BTreeMap<Vec<u8>, IndexedValue>,
    BuildSnapshot,
    BasePageIndexManifest,
) {
    let root = temp_root(name);
    let base = root.join("cf").join(ColumnFamily::Base.name());
    fs::create_dir_all(&base).unwrap();
    write_sst(
        base.join("00000000000000000001.sst"),
        [(b"a".as_slice(), b"old".as_slice())],
    )
    .unwrap();
    let previous = build_base_page_index(&root, 1, |_| Ok(())).unwrap();
    let new_sst = base.join("00000000000000000002.sst");
    write_sst(&new_sst, [(b"a".as_slice(), b"new".as_slice())]).unwrap();
    let order = sst_order_key(&new_sst).unwrap().unwrap();
    let record_offset = SstReader::open(&new_sst)
        .unwrap()
        .iter_with_offsets()
        .unwrap()[0]
        .0;
    let rows = BTreeMap::from([(
        b"a".to_vec(),
        IndexedValue {
            value_sha256_hex: sha256_hex(b"new"),
            tombstoned: false,
            source: BasePageIndexSource::Sst {
                path: relative_path(&root, &new_sst),
                order_epoch: order.epoch,
                order_seq: order.seq,
                order_class_rank: order.class_rank,
                order_index: order.index,
                record_offset: Some(record_offset),
            },
        },
    )]);
    let (base_source_digest_hex, base_durable_seq, base_sst_files) =
        compute_base_source_digest(&root).unwrap();
    let snapshot = BuildSnapshot {
        ledger_head_height: 0,
        ledger_head_tip_hash_hex: hex_bytes(&[0_u8; 32]),
        base_source_digest_hex,
        base_durable_seq,
        base_sst_files,
        wal_records: 0,
    };
    (root, rows, snapshot, previous)
}

/// Full-state verification harness for issue #1861. Runs only when
/// `CALYX_ISSUE1861_FSV_DIR` is set, writing every artifact to that directory so
/// the on-disk manifest bytes and page files can be read back independently
/// from the shell. Exercises the real consumer function `read_indexed_base_rows`
/// (the same call weave coverage scan and vault-health make) against real vault
/// bytes: real Base SSTs, a real WAL, and a real persisted index.
#[test]
fn issue1861_full_state_verification() {
    let Ok(dir) = std::env::var("CALYX_ISSUE1861_FSV_DIR") else {
        return;
    };
    let evidence_root = PathBuf::from(dir);
    fs::create_dir_all(&evidence_root).unwrap();

    let build_vault = |name: &str| -> PathBuf {
        let root = evidence_root.join(name);
        fs::remove_dir_all(&root).ok();
        let panel_path = root.join("panel").join("panel.json");
        fs::create_dir_all(panel_path.parent().unwrap()).unwrap();
        let panel_bytes = b"{}";
        fs::write(&panel_path, panel_bytes).unwrap();
        let panel_ref = ImmutableRef::from_bytes("panel/panel.json", panel_bytes).unwrap();
        let manifest = VaultManifest::new(1, 0, panel_ref, Vec::new()).unwrap();
        ManifestStore::open(&root).write_current(&manifest).unwrap();
        let base = root.join("cf").join(ColumnFamily::Base.name());
        fs::create_dir_all(&base).unwrap();
        write_sst(
            base.join("00000000000000000001.sst"),
            [
                (b"cx-0001".as_slice(), b"base-value-0001".as_slice()),
                (b"cx-0002".as_slice(), b"base-value-0002".as_slice()),
            ],
        )
        .unwrap();
        root
    };
    let manifest_digest = |root: &PathBuf| -> String {
        read_base_page_index_manifest(root)
            .unwrap()
            .base_source_digest_hex
    };

    // Scenario (a): Answer/Guard-only ledger append must NOT stale the index.
    let a = build_vault("scenario_a_answer_only");
    let built = build_base_page_index(&a, 1024, |_| Ok(())).unwrap();
    println!(
        "[a] BEFORE: built index digest={} live_entries={} base_sst_files={} durable_seq={}",
        built.base_source_digest_hex,
        built.live_entries,
        built.base_sst_files,
        built.base_durable_seq
    );
    let before_rows = read_indexed_base_rows(&a, 10).unwrap();
    println!(
        "[a] BEFORE: read_indexed_base_rows accepted fresh, {} live rows",
        before_rows.len()
    );
    let mut wal = Wal::open(a.join("wal"), WalOptions::default()).unwrap();
    wal.append(
        &encode_write_batch(&[
            WriteRow {
                cf: ColumnFamily::Ledger,
                key: b"answer-77401".to_vec(),
                value: b"answer-entry-body".to_vec(),
            },
            WriteRow {
                cf: ColumnFamily::Guard,
                key: b"guard-profile".to_vec(),
                value: b"calibration".to_vec(),
            },
        ])
        .unwrap(),
    )
    .unwrap();
    drop(wal);
    fs::create_dir_all(a.join("ledger_head")).unwrap();
    fs::write(
        a.join("ledger_head").join("current.json"),
        serde_json::to_vec(&LedgerHeadAnchor::new(77_401, [9_u8; 32]).unwrap()).unwrap(),
    )
    .unwrap();
    let after_rows = read_indexed_base_rows(&a, 10).unwrap();
    println!(
        "[a] AFTER answer-only append + head->77401: read_indexed_base_rows STILL accepted fresh, {} live rows; on-disk digest={} (unchanged={})",
        after_rows.len(),
        manifest_digest(&a),
        manifest_digest(&a) == built.base_source_digest_hex
    );
    assert_eq!(before_rows, after_rows);
    assert_eq!(manifest_digest(&a), built.base_source_digest_hex);

    // Scenario (b): real Base mutation (put via new SST, then WAL tombstone) must stale.
    let b = build_vault("scenario_b_base_mutation");
    let built_b = build_base_page_index(&b, 1024, |_| Ok(())).unwrap();
    println!("[b] BEFORE: digest={}", built_b.base_source_digest_hex);
    let base_b = b.join("cf").join(ColumnFamily::Base.name());
    write_sst(
        base_b.join("00000000000000000002.sst"),
        [(b"cx-0003".as_slice(), b"new-base-row".as_slice())],
    )
    .unwrap();
    let put_err = read_indexed_base_rows(&b, 10).unwrap_err();
    println!(
        "[b] AFTER Base put (new SST): code={} message={}",
        put_err.code, put_err.message
    );
    assert_eq!(put_err.code, STALE_CODE);
    // Rebuild over the put, then tombstone via WAL and prove stale again.
    let rebuilt_b = build_base_page_index(&b, 1024, |_| Ok(())).unwrap();
    println!(
        "[b] rebuilt over put: digest={}",
        rebuilt_b.base_source_digest_hex
    );
    let mut wal_b = Wal::open(b.join("wal"), WalOptions::default()).unwrap();
    wal_b
        .append(
            &encode_write_batch(&[WriteRow {
                cf: ColumnFamily::Base,
                key: b"cx-0001".to_vec(),
                value: crate::mvcc::tombstone_value(),
            }])
            .unwrap(),
        )
        .unwrap();
    drop(wal_b);
    let tomb_err = read_indexed_base_rows(&b, 10).unwrap_err();
    println!(
        "[b] AFTER Base tombstone (WAL): code={} message={}",
        tomb_err.code, tomb_err.message
    );
    assert_eq!(tomb_err.code, STALE_CODE);

    // Scenario (c): one-byte corruption of a page must fail closed CORRUPT.
    let c = build_vault("scenario_c_corruption");
    let built_c = build_base_page_index(&c, 1024, |_| Ok(())).unwrap();
    let page_path = c.join(BASE_PAGE_INDEX_DIR).join(&built_c.pages[0].path);
    let mut page_bytes = fs::read(&page_path).unwrap();
    let flip_at = page_bytes.len() / 2;
    println!(
        "[c] BEFORE: page {} byte[{}]={:#04x}",
        built_c.pages[0].path, flip_at, page_bytes[flip_at]
    );
    page_bytes[flip_at] ^= 0x01;
    fs::write(&page_path, &page_bytes).unwrap();
    let corrupt_err = read_indexed_base_rows(&c, 10).unwrap_err();
    println!(
        "[c] AFTER 1-byte flip: code={} message={}",
        corrupt_err.code, corrupt_err.message
    );
    assert_eq!(corrupt_err.code, CORRUPT_CODE);

    println!("[fsv] evidence_root={}", evidence_root.display());
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("calyx-base-page-index-{name}-{nanos}"));
    let panel_path = root.join("panel").join("panel.json");
    fs::create_dir_all(panel_path.parent().unwrap()).unwrap();
    let panel_bytes = b"{}";
    fs::write(&panel_path, panel_bytes).unwrap();
    let panel_ref = ImmutableRef::from_bytes("panel/panel.json", panel_bytes).unwrap();
    let manifest = VaultManifest::new(1, 0, panel_ref, Vec::new()).unwrap();
    ManifestStore::open(&root).write_current(&manifest).unwrap();
    root
}

fn cleanup(path: PathBuf) {
    fs::remove_dir_all(path).ok();
}
