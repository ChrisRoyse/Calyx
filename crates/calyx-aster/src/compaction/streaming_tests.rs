use super::*;
use crate::cf::ColumnFamily;
use crate::sst::{SstReader, write_sst};
use crate::storage_names::{SstName, classify_sst};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

proptest! {
    #[test]
    fn streaming_compaction_matches_btreemap_for_random_overlaps(
        shard_rows in proptest::collection::vec(
            proptest::collection::vec((0_u8..32, proptest::collection::vec(any::<u8>(), 0..24)), 0..24),
            2..6,
        )
    ) {
        let dir = test_dir("streaming-equivalence-random");
        let mut shards = Vec::new();
        let mut expected = BTreeMap::new();
        for (shard_index, rows) in shard_rows.iter().enumerate() {
            let mut shard_map = BTreeMap::<Vec<u8>, Vec<u8>>::new();
            for (key, value) in rows {
                shard_map.insert(vec![*key], value.clone());
            }
            for (key, value) in &shard_map {
                expected.insert(key.clone(), value.clone());
            }
            let path = dir.join(format!("input-{shard_index:04}.sst"));
            let entries = shard_map
                .iter()
                .map(|(key, value)| (key.as_slice(), value.as_slice()));
            write_sst(&path, entries).expect("write input shard");
            shards.push(SstShard::new(ColumnFamily::Base, &path, shard_index as u8).unwrap());
        }

        let result = compact_shards(
            ColumnFamily::Base,
            &shards,
            dir.join("merged.sst"),
            CompactionThrottle::unlimited(),
        )
        .expect("compact");
        let CompactionResult::Compacted(report) = result else {
            panic!("expected compaction");
        };

        prop_assert_eq!(report_rows(&report), expected_rows(&expected));
        prop_assert_eq!(report.logical_bytes, expected.values().map(|value| value.len() as u64).sum::<u64>());
        cleanup(dir);
    }
}

#[test]
fn streaming_compaction_preserves_overlap_empty_and_tombstone_values() {
    let dir = test_dir("streaming-overlap-edge-values");
    let first = dir.join("first.sst");
    let second = dir.join("second.sst");
    let third = dir.join("third.sst");
    let tombstone = crate::mvcc::tombstone_value();
    write_sst(
        &first,
        [
            (b"a".as_slice(), b"old-a".as_slice()),
            (b"b".as_slice(), b"old-b".as_slice()),
            (b"c".as_slice(), b"old-c".as_slice()),
        ],
    )
    .expect("write first");
    write_sst(
        &second,
        [
            (b"a".as_slice(), b"new-a".as_slice()),
            (b"b".as_slice(), b"".as_slice()),
        ],
    )
    .expect("write second");
    write_sst(
        &third,
        [
            (b"c".as_slice(), tombstone.as_slice()),
            (b"d".as_slice(), b"new-d".as_slice()),
        ],
    )
    .expect("write third");

    let shards = vec![
        SstShard::new(ColumnFamily::Base, &first, 0).unwrap(),
        SstShard::new(ColumnFamily::Base, &second, 0).unwrap(),
        SstShard::new(ColumnFamily::Base, &third, 0).unwrap(),
    ];
    let result = compact_shards(
        ColumnFamily::Base,
        &shards,
        dir.join("merged.sst"),
        CompactionThrottle::unlimited(),
    )
    .expect("compact");
    let CompactionResult::Compacted(report) = result else {
        panic!("expected compaction");
    };

    assert_eq!(
        report_rows(&report),
        vec![
            (b"a".to_vec(), b"new-a".to_vec()),
            (b"b".to_vec(), Vec::new()),
            (b"c".to_vec(), tombstone),
            (b"d".to_vec(), b"new-d".to_vec()),
        ]
    );
    cleanup(dir);
}

#[test]
fn output_rolling_writes_canonical_bounded_ssts_and_reads_back_all_rows() {
    let dir = test_dir("rolling-readback");
    let first = dir.join("first.sst");
    let second = dir.join("second.sst");
    let value_old = vec![b'o'; 32];
    let value_new = vec![b'n'; 32];
    let mut first_rows = BTreeMap::<Vec<u8>, Vec<u8>>::new();
    let mut second_rows = BTreeMap::<Vec<u8>, Vec<u8>>::new();
    let mut expected = BTreeMap::<Vec<u8>, Vec<u8>>::new();
    for index in 0..24_u8 {
        let key = format!("k{index:03}").into_bytes();
        first_rows.insert(key.clone(), value_old.clone());
        expected.insert(key, value_old.clone());
    }
    for index in 8..32_u8 {
        let key = format!("k{index:03}").into_bytes();
        second_rows.insert(key.clone(), value_new.clone());
        expected.insert(key, value_new.clone());
    }
    write_sst(
        &first,
        first_rows
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice())),
    )
    .expect("write first");
    write_sst(
        &second,
        second_rows
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice())),
    )
    .expect("write second");
    let shards = vec![
        SstShard::new(ColumnFamily::Base, &first, 0).unwrap(),
        SstShard::new(ColumnFamily::Base, &second, 0).unwrap(),
    ];
    let target_bytes = 260;
    let result = compact_shards_with_target(
        ColumnFamily::Base,
        &shards,
        dir.join("00000000000000000042-9999.sst"),
        CompactionThrottle::unlimited(),
        target_bytes,
    )
    .expect("compact");
    let CompactionResult::Compacted(report) = result else {
        panic!("expected compaction");
    };

    assert!(report.output_paths.len() > 1);
    for path in &report.output_paths {
        let bytes = fs::metadata(path).unwrap().len();
        assert!(
            bytes <= target_bytes,
            "{} wrote {bytes} bytes over target {target_bytes}",
            path.display()
        );
        assert!(matches!(
            classify_sst(path).unwrap(),
            Some(SstName::DurableBatch { seq: 42, .. })
        ));
    }
    assert_eq!(
        report.output_bytes,
        report
            .output_paths
            .iter()
            .map(|path| fs::metadata(path).unwrap().len())
            .sum::<u64>()
    );
    let physical_rows = report_rows(&report);
    assert_eq!(physical_rows, expected_rows(&expected));
    write_rolling_fsv(&report, target_bytes, physical_rows.len(), expected.len());

    let output_shards = report
        .output_paths
        .iter()
        .map(|path| SstShard::new(ColumnFamily::Base, path, 1).unwrap())
        .collect::<Vec<_>>();
    let catalog = CompactionCatalog::new(output_shards);
    for (key, value) in expected {
        assert_eq!(
            catalog
                .pin_snapshot()
                .get(ColumnFamily::Base, &key)
                .unwrap(),
            Some(value)
        );
    }
    cleanup(dir);
}

#[test]
fn zero_row_compaction_writes_empty_output_readback() {
    let dir = test_dir("zero-row");
    let first = dir.join("empty-a.sst");
    let second = dir.join("empty-b.sst");
    write_sst(&first, std::iter::empty::<(&[u8], &[u8])>()).expect("write first");
    write_sst(&second, std::iter::empty::<(&[u8], &[u8])>()).expect("write second");
    let shards = vec![
        SstShard::new(ColumnFamily::Base, &first, 0).unwrap(),
        SstShard::new(ColumnFamily::Base, &second, 0).unwrap(),
    ];
    let result = compact_shards(
        ColumnFamily::Base,
        &shards,
        dir.join("merged.sst"),
        CompactionThrottle::unlimited(),
    )
    .expect("compact");
    let CompactionResult::Compacted(report) = result else {
        panic!("expected compaction");
    };

    assert_eq!(report.output_paths.len(), 1);
    assert!(report_rows(&report).is_empty());
    cleanup(dir);
}

#[test]
fn oversized_row_fails_closed_without_output() {
    let dir = test_dir("oversized-row");
    let first = dir.join("first.sst");
    let second = dir.join("second.sst");
    let large = vec![b'x'; 64];
    write_sst(&first, [(b"a".as_slice(), large.as_slice())]).expect("write first");
    write_sst(&second, [(b"b".as_slice(), b"ok".as_slice())]).expect("write second");
    let shards = vec![
        SstShard::new(ColumnFamily::Base, &first, 0).unwrap(),
        SstShard::new(ColumnFamily::Base, &second, 0).unwrap(),
    ];
    let output = dir.join("00000000000000000007-9999.sst");
    let error = compact_shards_with_target(
        ColumnFamily::Base,
        &shards,
        &output,
        CompactionThrottle::unlimited(),
        80,
    )
    .expect_err("oversized row must fail closed");

    assert_eq!(error.code, "CALYX_ASTER_COMPACTION_ROW_EXCEEDS_TARGET");
    assert!(!output.exists());
    cleanup(dir);
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-compaction-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}

fn report_rows(report: &CompactionReport) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut rows = Vec::new();
    for path in &report.output_paths {
        rows.extend(
            SstReader::open(path)
                .unwrap()
                .iter()
                .unwrap()
                .into_iter()
                .map(|entry| (entry.key, entry.value)),
        );
    }
    rows
}

fn expected_rows(rows: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<(Vec<u8>, Vec<u8>)> {
    rows.iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn write_rolling_fsv(
    report: &CompactionReport,
    target_bytes: u64,
    physical_row_count: usize,
    expected_row_count: usize,
) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let output_readback = report
        .output_paths
        .iter()
        .map(|path| {
            let bytes = fs::read(path).unwrap();
            serde_json::json!({
                "path": path.display().to_string(),
                "bytes": bytes.len(),
                "blake3": blake3::hash(&bytes).to_string(),
                "canonical": format!("{:?}", classify_sst(path).unwrap()),
                "within_target": (bytes.len() as u64) <= target_bytes,
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        root.join("issue1212-streaming-compaction-readback.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "target_bytes": target_bytes,
            "input_files": report.input_files,
            "output_files": report.output_paths.len(),
            "output_bytes": report.output_bytes,
            "logical_bytes": report.logical_bytes,
            "physical_row_count": physical_row_count,
            "expected_row_count": expected_row_count,
            "all_outputs_within_target": report
                .output_paths
                .iter()
                .all(|path| fs::metadata(path).unwrap().len() <= target_bytes),
            "outputs": output_readback,
        }))
        .unwrap(),
    )
    .unwrap();
}
