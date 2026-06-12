use super::*;
use crate::cf::ColumnFamily;
use calyx_core::SlotId;
use proptest::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn compaction_swaps_active_shards_without_breaking_pinned_reads() {
    let dir = test_dir("snapshot-safe");
    let first = dir.join("l0-a.sst");
    let second = dir.join("l0-b.sst");
    write_sst(&first, [(b"a".as_slice(), b"one".as_slice())]).expect("write first");
    write_sst(
        &second,
        [
            (b"a".as_slice(), b"two".as_slice()),
            (b"b".as_slice(), b"bee".as_slice()),
        ],
    )
    .expect("write second");
    let catalog = Arc::new(CompactionCatalog::new(vec![
        SstShard::new(ColumnFamily::Base, &first, 0).unwrap(),
        SstShard::new(ColumnFamily::Base, &second, 0).unwrap(),
    ]));
    let pinned = catalog.pin_snapshot();
    let keep_reading = Arc::new(AtomicBool::new(true));
    let read_count = Arc::new(AtomicU64::new(0));
    let reader_flag = keep_reading.clone();
    let reader_count = read_count.clone();
    let reader = thread::spawn(move || {
        while reader_flag.load(Ordering::Relaxed) {
            assert_eq!(
                pinned.get(ColumnFamily::Base, b"a").unwrap().unwrap(),
                b"two"
            );
            assert_eq!(
                pinned.get(ColumnFamily::Base, b"b").unwrap().unwrap(),
                b"bee"
            );
            reader_count.fetch_add(1, Ordering::Relaxed);
        }
    });
    while read_count.load(Ordering::Relaxed) == 0 {
        thread::yield_now();
    }

    let output = dir.join("l1-merged.sst");
    let result = catalog
        .compact_cf(ColumnFamily::Base, &output, CompactionThrottle::unlimited())
        .expect("compact");
    keep_reading.store(false, Ordering::Relaxed);
    reader.join().expect("reader joins");
    let new_snapshot = catalog.pin_snapshot();

    assert!(read_count.load(Ordering::Relaxed) > 0);
    assert!(matches!(result, CompactionResult::Compacted(_)));
    assert_eq!(new_snapshot.shard_count(), 1);
    assert_eq!(
        new_snapshot.get(ColumnFamily::Base, b"a").unwrap().unwrap(),
        b"two"
    );
    cleanup(dir);
}

#[test]
fn throttle_skips_compaction_when_input_exceeds_run_budget() {
    let dir = test_dir("throttle");
    let first = dir.join("l0-a.sst");
    write_sst(&first, [(b"a".as_slice(), b"one".as_slice())]).expect("write first");
    let shard = SstShard::new(ColumnFamily::Base, &first, 0).unwrap();
    let result = compact_shards(
        ColumnFamily::Base,
        &[shard],
        dir.join("out.sst"),
        CompactionThrottle::max_input_bytes(1),
    )
    .expect("compact skipped");

    assert!(matches!(result, CompactionResult::Skipped { .. }));
    assert!(!dir.join("out.sst").exists());
    cleanup(dir);
}

#[test]
fn compaction_report_tracks_debt_and_write_amplification() {
    let dir = test_dir("report");
    let first = dir.join("l0-a.sst");
    let second = dir.join("l0-b.sst");
    let first_value = vec![b'a'; 8192];
    let second_value = vec![b'b'; 8192];
    write_sst(&first, [(b"a".as_slice(), first_value.as_slice())]).expect("write first");
    write_sst(&second, [(b"b".as_slice(), second_value.as_slice())]).expect("write second");
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

    assert_eq!(report.input_files, 2);
    assert!(report.input_bytes > 0);
    assert!(report.output_bytes > 0);
    assert!(report.write_amp_milli > 0);
    assert!(report.write_amp_milli <= CompactionSchedulerOptions::default().max_write_amp_milli);
    assert_eq!(report.staging_parent, dir);
    if let Some(root) = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from) {
        fs::write(
            root.join("compaction-write-amp-readback.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "input_files": report.input_files,
                "input_bytes": report.input_bytes,
                "output_bytes": report.output_bytes,
                "logical_bytes": report.logical_bytes,
                "write_amp_milli": report.write_amp_milli,
                "max_write_amp_milli": CompactionSchedulerOptions::default().max_write_amp_milli,
                "within_bound": report.write_amp_milli
                    <= CompactionSchedulerOptions::default().max_write_amp_milli,
            }))
            .unwrap(),
        )
        .unwrap();
    }
    cleanup(dir);
}

proptest! {
    #[test]
    fn compaction_debt_matches_scaled_pending_bytes(
        bytes in proptest::collection::vec(0_u64..1_000_000, 0..64),
        target_bytes in 0_u64..1_000_000,
    ) {
        let shards = bytes
            .iter()
            .map(|bytes| SstShard {
                cf: ColumnFamily::Base,
                path: PathBuf::from("synthetic.sst"),
                level: 0,
                bytes: *bytes,
            })
            .collect::<Vec<_>>();
        let pending = bytes.iter().fold(0_u64, |sum, bytes| sum.saturating_add(*bytes));
        let target = target_bytes.max(1);
        let debt = CompactionDebt::measure(&shards, target_bytes);

        prop_assert_eq!(debt.pending_bytes, pending);
        prop_assert_eq!(debt.target_bytes, target);
        prop_assert_eq!(
            debt.score_milli,
            pending.saturating_mul(WRITE_AMP_SCALE) / target
        );
    }
}

#[test]
fn tiering_policy_places_hot_and_cold_cfs() {
    let dir = test_dir("tiering");
    let hot = dir.join("hot");
    let cold = dir.join("archive");
    let policy = TieringPolicy::new(&hot, &cold, [SlotId::new(0)], 7);

    let active = policy.place_cf(ColumnFamily::slot(SlotId::new(0)), 7);
    let raw = policy.place_cf(ColumnFamily::slot_raw(SlotId::new(0)), 7);
    let retired = policy.place_cf(ColumnFamily::slot(SlotId::new(9)), 7);
    let old_panel = policy.place_cf(ColumnFamily::slot(SlotId::new(0)), 6);
    let base = policy.place_cf(ColumnFamily::Base, 6);
    let ledger = policy.place_cf(ColumnFamily::Ledger, 6);

    assert_eq!(active.tier, StorageTier::Hot);
    assert!(active.absolute_dir().starts_with(&hot));
    assert_eq!(raw.tier, StorageTier::Cold);
    assert_eq!(retired.tier, StorageTier::Cold);
    assert_eq!(old_panel.tier, StorageTier::Cold);
    assert_eq!(base.tier, StorageTier::Hot);
    assert_eq!(ledger.tier, StorageTier::Hot);
    assert!(raw.absolute_dir().starts_with(&cold));
    cleanup(dir);
}

#[test]
fn tiered_writer_uses_archive_parent_for_cold_raw_sidecar() {
    let dir = test_dir("tiered-write");
    let hot = dir.join("hot");
    let cold = dir.join("archive");
    let policy = TieringPolicy::new(&hot, &cold, [SlotId::new(0)], 7);
    let written = policy
        .write_tiered_sst(
            ColumnFamily::slot_raw(SlotId::new(0)),
            7,
            "00000000000000000001.sst",
            [(b"k".as_slice(), b"raw-f32".as_slice())],
        )
        .expect("write cold raw");

    assert_eq!(written.placement.tier, StorageTier::Cold);
    assert!(written.path.starts_with(&cold));
    assert_eq!(written.path.parent().unwrap(), written.staging_parent);
    assert_eq!(
        SstReader::open(&written.path)
            .unwrap()
            .iter()
            .unwrap()
            .len(),
        1
    );
    cleanup(dir);
}

#[test]
fn catalog_reports_shard_count_for_one_cf() {
    let dir = test_dir("shard-count");
    let base = dir.join("base.sst");
    let ledger = dir.join("ledger.sst");
    write_sst(&base, [(b"a".as_slice(), b"one".as_slice())]).expect("write base");
    write_sst(
        &ledger,
        [(b"\0\0\0\0\0\0\0\x01".as_slice(), b"l".as_slice())],
    )
    .expect("write ledger");
    let catalog = CompactionCatalog::new(vec![
        SstShard::new(ColumnFamily::Base, &base, 0).unwrap(),
        SstShard::new(ColumnFamily::Ledger, &ledger, 0).unwrap(),
    ]);

    assert_eq!(catalog.shard_count_for_cf(ColumnFamily::Base), 1);
    assert_eq!(catalog.shard_count_for_cf(ColumnFamily::Anchors), 0);
    cleanup(dir);
}

#[test]
fn scheduler_compacts_debt_and_stops_cleanly() {
    let dir = test_dir("scheduler");
    let first = dir.join("l0-a.sst");
    let second = dir.join("l0-b.sst");
    write_sst(&first, [(b"a".as_slice(), b"one".as_slice())]).expect("write first");
    write_sst(&second, [(b"b".as_slice(), b"two".as_slice())]).expect("write second");
    let catalog = Arc::new(CompactionCatalog::new(vec![
        SstShard::new(ColumnFamily::Base, &first, 0).unwrap(),
        SstShard::new(ColumnFamily::Base, &second, 0).unwrap(),
    ]));
    let options = CompactionSchedulerOptions {
        interval_ms: 1,
        debt_trigger_score_milli: 0,
        output_root: dir.join("scheduled"),
        ..CompactionSchedulerOptions::default()
    };

    let scheduler = CompactionScheduler::start(catalog.clone(), options);
    let deadline = Instant::now() + Duration::from_secs(2);
    while catalog.shard_count_for_cf(ColumnFamily::Base) != 1 {
        assert!(
            Instant::now() < deadline,
            "scheduler did not compact before deadline"
        );
        thread::yield_now();
    }
    scheduler.stop().expect("scheduler joins");

    assert_eq!(catalog.shard_count_for_cf(ColumnFamily::Base), 1);
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
