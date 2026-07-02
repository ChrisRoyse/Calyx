use super::*;
use std::path::PathBuf;

fn sst(name: &str) -> PathBuf {
    PathBuf::from("/vault/cf/base").join(name)
}

/// Guard: every static CF round-trips name → parse_cf_dir_name. A new
/// `ColumnFamily` that forgets to register its directory name here fails
/// this test immediately instead of only on a vault reopen.
#[test]
fn every_static_cf_dir_name_round_trips() {
    for cf in ColumnFamily::STATIC {
        let parsed = parse_cf_dir_name(&cf.name())
            .unwrap_or_else(|e| panic!("CF {:?} dir name {} not parseable: {e}", cf, cf.name()));
        assert_eq!(
            parsed, cf,
            "parse_cf_dir_name must invert name() for {cf:?}"
        );
    }
    // Slot CFs (quantized + raw) also round-trip.
    for cf in [
        ColumnFamily::slot(calyx_core::SlotId::new(0)),
        ColumnFamily::slot_raw(calyx_core::SlotId::new(42)),
    ] {
        assert_eq!(parse_cf_dir_name(&cf.name()).unwrap(), cf);
    }
}

#[test]
fn canonical_sst_names_classify() {
    assert_eq!(
        classify_sst(&sst("00000000000000000007.sst")).unwrap(),
        Some(SstName::RouterLegacy { ordinal: 7 })
    );
    assert_eq!(
        classify_sst(&sst("00000000000000000007-0003.sst")).unwrap(),
        Some(SstName::DurableBatch { seq: 7, index: 3 })
    );
    assert_eq!(
        classify_sst(&sst("00000000000000000007-9999.sst")).unwrap(),
        Some(SstName::DurableBatch {
            seq: 7,
            index: 9999
        })
    );
    assert_eq!(
        classify_sst(&sst("00000000000000000007-12345.sst")).unwrap(),
        Some(SstName::DurableBatch {
            seq: 7,
            index: 12345
        })
    );
    assert_eq!(
        classify_sst(&sst("compacted-00000000000000000042.sst")).unwrap(),
        Some(SstName::Compacted { seq: 42 })
    );
    assert_eq!(
        classify_sst(&sst("flush-00000000000000000042-0007.sst")).unwrap(),
        Some(SstName::Flush {
            watermark: 42,
            ordinal: 7
        })
    );
    assert_eq!(
        classify_sst(&sst(&flush_sst_file_name(42, 7))).unwrap(),
        Some(SstName::Flush {
            watermark: 42,
            ordinal: 7
        })
    );
}

#[test]
fn sst_order_key_uses_sequence_not_filename_prefix() {
    let compacted_6 = sst_order_key(&sst("compacted-00000000000000000006.sst"))
        .unwrap()
        .unwrap();
    let flush_7 = sst_order_key(&sst("flush-00000000000000000007-0009.sst"))
        .unwrap()
        .unwrap();
    let durable_7 = sst_order_key(&sst("00000000000000000007-0000.sst"))
        .unwrap()
        .unwrap();
    let compacted_7 = sst_order_key(&sst("compacted-00000000000000000007.sst"))
        .unwrap()
        .unwrap();

    assert!(compacted_6 < flush_7);
    assert!(flush_7 < durable_7);
    assert!(durable_7 < compacted_7);
}

/// Legacy flush files (epoch 0) sort before every commit-domain file, and the
/// commit-anchored flush chain orders by watermark against durable batches —
/// the #1138 inversion (durable batch shadowed by a higher-ordinal flush)
/// cannot be expressed any more.
#[test]
fn legacy_epoch_sorts_before_commit_domain_and_flush_orders_by_watermark() {
    let legacy_5 = sst_order_key(&sst("00000000000000000005.sst"))
        .unwrap()
        .unwrap();
    let durable_4 = sst_order_key(&sst("00000000000000000004-0000.sst"))
        .unwrap()
        .unwrap();
    let flush_wm3_ord5 = sst_order_key(&sst("flush-00000000000000000003-0005.sst"))
        .unwrap()
        .unwrap();

    // Epoch 0 first: the legacy file no longer shadows the durable batch.
    assert!(legacy_5 < durable_4);
    // Commit-anchored flush at watermark 3 sorts before durable batch seq 4
    // even though its flush ordinal (5) is numerically larger.
    assert!(flush_wm3_ord5 < durable_4);
    // Within the flush chain, same watermark orders by ordinal.
    let flush_wm3_ord6 = sst_order_key(&sst("flush-00000000000000000003-0006.sst"))
        .unwrap()
        .unwrap();
    assert!(flush_wm3_ord5 < flush_wm3_ord6);
}

#[test]
fn ambiguity_gate_rejects_legacy_ordinal_above_commit_seq() {
    // The #1138 repro layout: flush ordinal 5 vs durable batch commit seq 4.
    let files = [
        sst("00000000000000000005.sst"),
        sst("00000000000000000004-0000.sst"),
    ];
    let error = ensure_unambiguous_sst_order(files.iter().map(PathBuf::as_path)).unwrap_err();
    assert_eq!(error.code.to_string(), "CALYX_ASTER_SST_ORDER_AMBIGUOUS");

    // Mature-vault layout (tiny ordinals, large commit seqs) stays accepted.
    let ok = [
        sst("00000000000000000003.sst"),
        sst("00000000000000000900-0000.sst"),
        sst("compacted-00000000000000000900.sst"),
    ];
    ensure_unambiguous_sst_order(ok.iter().map(PathBuf::as_path)).unwrap();

    // Equal ordinal/seq keeps today's order (flush-class first) — accepted.
    let equal = [
        sst("00000000000000000004.sst"),
        sst("00000000000000000004-0000.sst"),
    ];
    ensure_unambiguous_sst_order(equal.iter().map(PathBuf::as_path)).unwrap();

    // Commit-anchored flush files never trip the gate: they share the ordinal
    // chain with legacy files and the commit domain with durable batches.
    let flushes = [
        sst("00000000000000000005.sst"),
        sst("flush-00000000000000000003-0006.sst"),
    ];
    ensure_unambiguous_sst_order(flushes.iter().map(PathBuf::as_path)).unwrap();

    // Legacy-only and commit-only sets are trivially unambiguous.
    ensure_unambiguous_sst_order(
        [sst("00000000000000000005.sst")]
            .iter()
            .map(PathBuf::as_path),
    )
    .unwrap();
    ensure_unambiguous_sst_order(std::iter::empty()).unwrap();
}

#[test]
fn non_sst_files_are_not_claimed() {
    assert_eq!(classify_sst(&sst(".append.lock")).unwrap(), None);
    assert_eq!(classify_sst(&sst("notes.txt")).unwrap(), None);
    assert_eq!(
        classify_sst(&sst(".00000000000000000007.sst.tmp")).unwrap(),
        None
    );
}

#[test]
fn noncanonical_sst_names_fail_closed() {
    for name in [
        "1.sst",                                    // missing zero padding
        "00000000000000000007-1.sst",               // index missing zero padding
        "00000000000000000007-01000.sst",           // over-wide zero-padded index
        "compacted-1.sst",                          // compacted seq missing padding
        "99999999999999999999.sst",                 // 20 digits but > u64::MAX
        "0000000000000000000a.sst",                 // non-digit
        "soak-00.sst",                              // legacy CLI soak name
        "compact-1764950000000.sst",                // legacy CLI compact name
        "tiered.sst",                               // legacy CLI tier name
        "00000000000000000007-.sst",                // empty index
        "flush-1-0001.sst",                         // flush watermark missing padding
        "flush-00000000000000000007-1.sst",         // flush ordinal missing padding
        "flush-00000000000000000007.sst",           // flush missing ordinal
        "flush-99999999999999999999-0001.sst",      // flush watermark > u64::MAX
        "flush-00000000000000000007-0001-0002.sst", // extra segment
        "garbage.sst",
    ] {
        let error = classify_sst(&sst(name)).unwrap_err();
        assert_eq!(
            error.code.to_string(),
            "CALYX_ASTER_CORRUPT_SHARD",
            "{name}"
        );
    }
}

#[test]
fn wal_names_classify_and_fail_closed() {
    assert_eq!(
        wal_segment_index(Path::new("/v/wal/00000000000000000003.wal")).unwrap(),
        Some(3)
    );
    assert_eq!(
        wal_segment_index(Path::new("/v/wal/.append.lock")).unwrap(),
        None
    );
    for name in [
        "3.wal",
        "0000000000000000000x.wal",
        "99999999999999999999.wal",
    ] {
        let error = wal_segment_index(&PathBuf::from("/v/wal").join(name)).unwrap_err();
        assert_eq!(
            error.code.to_string(),
            "CALYX_ASTER_CORRUPT_SHARD",
            "{name}"
        );
    }
}

#[test]
fn cf_dir_names_round_trip_and_fail_closed() {
    for cf in [
        ColumnFamily::Base,
        ColumnFamily::Recurrence,
        ColumnFamily::Document,
        ColumnFamily::slot(SlotId::new(0)),
        ColumnFamily::slot_raw(SlotId::new(7)),
        ColumnFamily::slot(SlotId::new(123)),
    ] {
        assert_eq!(parse_cf_dir_name(&cf.name()).unwrap(), cf);
    }
    for name in ["slot_5", "slot_xyz", "slot_99999", "unknown_cf", "Slot_05"] {
        let error = parse_cf_dir_name(name).unwrap_err();
        assert_eq!(
            error.code.to_string(),
            "CALYX_ASTER_CORRUPT_SHARD",
            "{name}"
        );
    }
}
