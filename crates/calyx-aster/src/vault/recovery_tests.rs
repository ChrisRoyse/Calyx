use super::*;
use calyx_core::{AbsentReason, CxFlags, InputRef, LedgerRef, Modality, SlotVector, VaultStore};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn open_recovers_manifested_rows_from_ssts_when_wal_history_is_absent() {
    let dir = test_dir("manifested-sst-without-wal");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    let cx = sample_constellation();
    let id = cx.cx_id;

    vault.put(cx.clone()).expect("durable put");
    vault.flush().expect("flush durable");
    fs::remove_file(dir.join("wal/00000000000000000000.wal")).expect("remove WAL history");

    let reopened =
        AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default()).expect("cold open");
    let got = reopened.get(id, reopened.snapshot()).unwrap();
    let mut expected = cx;
    expected.provenance = got.provenance.clone();

    assert_eq!(reopened.snapshot(), 1);
    assert_eq!(got.provenance.seq, 0);
    assert_ne!(got.provenance.hash, [0x51; 32]);
    assert_eq!(got, expected);
    cleanup(dir);
}

#[test]
fn durable_open_empty_dir_starts_at_zero() {
    let dir = test_dir("durable-empty");
    let vault = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open empty durable");

    assert_eq!(vault.snapshot(), 0);
    assert_eq!(vault.recovery_report().last_recovered_seq, 0);
    assert_eq!(vault.recovery_report().torn_tail, None);
    cleanup(dir);
}

#[test]
fn stale_durable_handle_refreshes_before_commit_sequence_allocation() {
    let dir = test_dir("stale-handle-commit-seq");
    let first = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open first durable");
    let stale = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open stale durable");
    let first_cx = sample_constellation();
    let mut second_cx = sample_constellation();
    second_cx.cx_id = CxId::from_bytes([0x42; 16]);
    second_cx.input_ref.hash = [0x42; 32];
    second_cx.input_ref.pointer = Some("synthetic://stale-handle-second".to_string());
    let first_id = first_cx.cx_id;
    let second_id = second_cx.cx_id;

    first.put(first_cx).expect("first handle put");
    first.flush().expect("first flush");
    stale
        .put(second_cx)
        .expect("stale handle put after external commit");
    stale.flush().expect("stale flush");

    let replay = crate::wal::replay_dir(dir.join("wal")).expect("replay stale handle wal");
    let reopened = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("reopen stale handle vault");

    assert_eq!(
        replay
            .records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(stale.snapshot(), 2);
    assert_eq!(reopened.snapshot(), 2);
    assert_eq!(
        reopened.get(first_id, reopened.snapshot()).unwrap().cx_id,
        first_id
    );
    assert_eq!(
        reopened.get(second_id, reopened.snapshot()).unwrap().cx_id,
        second_id
    );
    cleanup(dir);
}

#[test]
fn open_reports_torn_tail_through_recovery_report() {
    let fsv_root = calyx_fsv::fsv_root("CALYX_FSV_ROOT");
    let dir = fsv_root.as_ref().map_or_else(
        || test_dir("open-torn-tail-report"),
        |root| {
            let dir = root.join("open-torn-tail-report").join("vault");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create fsv vault");
            dir
        },
    );
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    let cx = sample_constellation();
    let id = cx.cx_id;
    vault.put(cx.clone()).expect("durable put");
    vault.flush().expect("flush durable");
    drop(vault);

    let wal_path = dir.join("wal/00000000000000000000.wal");
    let before_torn_bytes = fs::metadata(&wal_path).expect("wal metadata").len();
    let mut file = OpenOptions::new()
        .append(true)
        .open(&wal_path)
        .expect("open wal for torn append");
    file.write_all(b"CXW1partial").expect("write torn bytes");
    file.sync_data().expect("fsync torn bytes");
    drop(file);
    let after_torn_bytes = fs::metadata(&wal_path).expect("wal metadata").len();

    let reopened =
        AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default()).expect("cold open");
    let report = reopened.recovery_report();
    let tail = report.torn_tail.as_ref().expect("torn tail reported");
    let truncated_bytes = fs::metadata(&wal_path).expect("wal metadata").len();
    let got = reopened
        .get(id, reopened.snapshot())
        .expect("get recovered");

    assert_eq!(report.last_recovered_seq, 1);
    assert_eq!(tail.code, "CALYX_ASTER_TORN_WAL");
    assert_eq!(tail.offset, before_torn_bytes);
    assert_eq!(truncated_bytes, before_torn_bytes);
    assert!(after_torn_bytes > before_torn_bytes);
    assert_eq!(got.cx_id, id);
    if let Some(root) = fsv_root {
        let readback = serde_json::json!({
            "snapshot": reopened.snapshot(),
            "last_recovered_seq": report.last_recovered_seq,
            "torn_code": tail.code,
            "torn_offset": tail.offset,
            "before_torn_bytes": before_torn_bytes,
            "after_torn_bytes": after_torn_bytes,
            "truncated_bytes": truncated_bytes,
            "cx_id": id.to_string(),
        });
        fs::write(
            root.join("open-torn-tail-report-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    } else {
        cleanup(dir);
    }
}

#[test]
fn cold_open_fails_closed_on_mid_log_corruption_without_mutating_segments() {
    let dir = test_dir("mid-log-corruption");
    let mut options = VaultOptions::default();
    options.wal_options.max_segment_bytes = 1;
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", options.clone()).expect("open durable");
    let first = sample_constellation();
    let mut later = sample_constellation();
    later.cx_id = CxId::from_bytes([0x42; 16]);
    later.input_ref.hash = [0x42; 32];
    later.input_ref.pointer = Some("synthetic://mid-log-later".to_string());
    vault.put(first).expect("put first durable row");
    vault.put(later).expect("put later durable row");
    drop(vault);

    let mut segments = fs::read_dir(dir.join("wal"))
        .expect("read wal directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension().and_then(|value| value.to_str()) == Some("wal")).then_some(path)
        })
        .collect::<Vec<_>>();
    segments.sort();
    assert!(segments.len() >= 2, "expected WAL rotation: {segments:?}");
    let segment0 = &segments[0];
    let segment1 = &segments[1];
    let mut file = OpenOptions::new()
        .append(true)
        .open(segment0)
        .expect("open first segment for corrupt append");
    file.write_all(b"CXW1partial")
        .expect("append malformed header");
    file.sync_data().expect("fsync malformed header");
    drop(file);
    let segment0_before = fs::read(segment0).expect("read corrupt segment before open");
    let segment1_before = fs::read(segment1).expect("read later segment before open");

    let opened = AsterVault::open(&dir, vault_id(), b"salt", options);
    let segment0_after = fs::read(segment0).ok();
    let segment1_after = fs::read(segment1).ok();
    let segment0_preserved = segment0_after.as_deref() == Some(segment0_before.as_slice());
    let segment1_preserved = segment1_after.as_deref() == Some(segment1_before.as_slice());
    assert!(
        segment0_preserved && segment1_preserved,
        "cold open mutated durable WAL bytes: segment0_preserved={segment0_preserved} \
         segment1_preserved={segment1_preserved}"
    );
    let error = opened.expect_err("cold open must reject mid-log corruption");
    assert_eq!(error.code, "CALYX_ASTER_TORN_WAL");
    assert!(error.message.contains(&segment0.display().to_string()));
    cleanup(dir);
}

#[test]
fn cold_open_fails_closed_on_unrecognized_sst_name() {
    let dir = test_dir("unrecognized-sst-name");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    vault.put(sample_constellation()).expect("durable put");
    vault.flush().expect("flush durable");
    drop(vault);
    fs::write(dir.join("cf/base/garbage.sst"), b"not an aster name").expect("plant stray sst");

    let error = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect_err("cold open with unrecognized SST name");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(error.message.contains("garbage.sst"), "{}", error.message);
    cleanup(dir);
}

#[test]
fn cold_open_fails_closed_on_renamed_durable_sst() {
    let dir = test_dir("renamed-durable-sst");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    vault.put(sample_constellation()).expect("durable put");
    vault.flush().expect("flush durable");
    drop(vault);
    let base_dir = dir.join("cf/base");
    let durable_sst = fs::read_dir(&base_dir)
        .expect("read base dir")
        .map(|entry| entry.expect("entry").path())
        .find(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("sst")
                && path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|stem| stem.contains('-'))
        })
        .expect("durable batch SST present");
    // Simulate name corruption / partial rename of a real durable batch file.
    fs::rename(&durable_sst, base_dir.join("0000007-x.sst")).expect("corrupt sst name");

    let error = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect_err("cold open with corrupted SST name");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(error.message.contains("0000007-x.sst"), "{}", error.message);
    cleanup(dir);
}

fn sample_constellation() -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 3,
            data: vec![0.25, 0.5, 1.0],
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Absent {
            reason: AbsentReason::Deferred,
        },
    );
    Constellation {
        cx_id: CxId::from_bytes([0x31; 16]),
        vault_id: vault_id(),
        panel_version: 11,
        created_at: 1780831800,
        input_ref: InputRef {
            hash: [0x31; 32],
            pointer: Some("synthetic://manifested-sst-without-wal".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [0x51; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-vault-recovery-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}

fn wide_constellation(index: u32, slot_dim: u32) -> Constellation {
    let mut cx = sample_constellation();
    let mut id = [0u8; 16];
    id[..4].copy_from_slice(&index.to_be_bytes());
    cx.cx_id = CxId::from_bytes(id);
    cx.input_ref.hash = [(index % 251) as u8; 32];
    cx.input_ref.pointer = Some(format!("synthetic://stream-{index}"));
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: slot_dim,
            data: vec![0.25_f32; slot_dim as usize],
        },
    );
    cx.slots = slots;
    cx
}

/// Issue #1992 (round 3): a write-capable latest-readback open of a vault whose
/// manifest floor lags the WAL tip (the whole WAL is an uncheckpointed tail,
/// e.g. orphaned weave partials) must STREAM-restore the tail in bounded chunks
/// — never materialize it all into the heap — and still read every committed
/// row back. This is the streaming-recovery correctness proof.
#[test]
fn streaming_open_of_uncheckpointed_tail_reads_every_row_back() {
    let dir = test_dir("stream-uncheckpointed-tail");
    let mut options = VaultOptions::default();
    // Tiny segments so a modest write set rolls many segments; no flush, so the
    // manifest durable_seq stays at 0 and every record is a post-floor tail.
    options.wal_options.max_segment_bytes = 4096;
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", options).expect("open durable");
    let count = 40u32;
    for index in 1..=count {
        vault.put(wide_constellation(index, 64)).expect("put");
    }
    // Deliberately NO flush.
    let height_before = vault.latest_seq();
    assert_eq!(height_before, u64::from(count));
    assert!(
        wal_segment_paths(&dir).len() >= 2,
        "expected multi-segment uncheckpointed tail"
    );
    drop(vault);

    // Reopen through the streaming latest-readback path (wal-prune's profile).
    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"salt",
        VaultOptions::write_latest_readback(),
    )
    .expect("streaming reopen");
    assert_eq!(reopened.latest_seq(), height_before);
    for index in 1..=count {
        let mut id = [0u8; 16];
        id[..4].copy_from_slice(&index.to_be_bytes());
        let id = CxId::from_bytes(id);
        assert_eq!(
            reopened.get(id, reopened.snapshot()).unwrap().cx_id,
            id,
            "row {index} lost after streaming recovery"
        );
    }
    // Flush advances the manifest over the streamed tail, then a full reopen
    // reads everything back from durable state (belt-and-suspenders).
    reopened.flush().expect("flush");
    drop(reopened);
    let full =
        AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default()).expect("full reopen");
    assert_eq!(full.latest_seq(), height_before);
    for index in 1..=count {
        let mut id = [0u8; 16];
        id[..4].copy_from_slice(&index.to_be_bytes());
        let id = CxId::from_bytes(id);
        assert_eq!(full.get(id, full.snapshot()).unwrap().cx_id, id);
    }
    cleanup(dir);
}

/// Streaming recovery must heal a torn physical tail on the newest segment
/// (a SIGKILL mid-append) exactly as replay does, dropping only the torn
/// record and reading the rest back.
#[test]
fn streaming_open_heals_torn_newest_tail() {
    use std::io::Write;
    let dir = test_dir("stream-torn-tail");
    let mut options = VaultOptions::default();
    options.wal_options.max_segment_bytes = 4096;
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", options).expect("open durable");
    for index in 1..=8u32 {
        vault.put(wide_constellation(index, 32)).expect("put");
    }
    let height_before = vault.latest_seq();
    drop(vault);

    // Append a garbage (torn) record to the newest segment.
    let newest = wal_segment_paths(&dir).pop().expect("a segment");
    let mut file = OpenOptions::new()
        .append(true)
        .open(&newest)
        .expect("open newest segment");
    file.write_all(b"CXW1torn-partial-header")
        .expect("append torn");
    file.sync_data().expect("fsync torn");
    drop(file);

    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"salt",
        VaultOptions::write_latest_readback(),
    )
    .expect("streaming reopen heals torn tail");
    assert_eq!(reopened.latest_seq(), height_before);
    for index in 1..=8u32 {
        let mut id = [0u8; 16];
        id[..4].copy_from_slice(&index.to_be_bytes());
        let id = CxId::from_bytes(id);
        assert_eq!(reopened.get(id, reopened.snapshot()).unwrap().cx_id, id);
    }
    cleanup(dir);
}

/// Issue #1992 (round 3): streaming recovery over a >1.5 GiB uncheckpointed
/// tail must keep peak RSS to roughly one restored copy plus a flush chunk —
/// NOT the ~2x+ the pre-fix collect-all path held (recovery batches + tail
/// clone + Wal::open_after replay + staging). Periodic checkpoint flushing
/// drains staging so it does not add a second full copy. Linux-only.
#[cfg(target_os = "linux")]
#[test]
fn streaming_open_keeps_peak_rss_below_double_the_tail() {
    fn rss_kb() -> u64 {
        let status = fs::read_to_string("/proc/self/status").expect("status");
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                return rest.split_whitespace().next().unwrap().parse().unwrap();
            }
        }
        panic!("no VmRSS");
    }
    let dir = test_dir("stream-peak-rss");
    let mut options = VaultOptions::default();
    options.wal_options.max_segment_bytes = 64 * 1024 * 1024;
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", options).expect("open durable");
    // ~800 x ~2 MiB ≈ 1.6 GiB uncheckpointed tail across many segments.
    let count = 800u32;
    for index in 1..=count {
        vault
            .put(wide_constellation(index, 512 * 1024))
            .expect("put");
    }
    drop(vault);
    let wal_bytes: u64 = wal_segment_paths(&dir)
        .iter()
        .map(|p| fs::metadata(p).unwrap().len())
        .sum();
    assert!(
        wal_bytes > 1_536 * 1024 * 1024,
        "tail too small: {wal_bytes}"
    );

    let baseline = rss_kb();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let sampler = {
        let stop = stop.clone();
        let peak = peak.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                peak.fetch_max(rss_kb(), Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        })
    };
    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"salt",
        VaultOptions::write_latest_readback(),
    )
    .expect("streaming reopen");
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    let peak_delta_kb = peak.load(Ordering::Relaxed).saturating_sub(baseline);

    // One restored copy (~70% of WAL bytes) plus a <=512 MiB flush chunk stays
    // below the raw WAL byte count; the pre-fix 2x+ (staging holding a second
    // full copy alongside the batches Vec and tail clone) would exceed it.
    let wal_kb = wal_bytes / 1024;
    assert!(
        peak_delta_kb < wal_kb,
        "streaming open peak {peak_delta_kb} KiB >= WAL {wal_kb} KiB; staging not drained?"
    );
    assert_eq!(reopened.latest_seq(), u64::from(count));
    drop(reopened);
    cleanup(dir);
}

fn distinct_constellation(index: u8) -> Constellation {
    let mut cx = sample_constellation();
    cx.cx_id = CxId::from_bytes([index; 16]);
    cx.input_ref.hash = [index; 32];
    cx.input_ref.pointer = Some(format!("synthetic://wal-prune-row-{index}"));
    cx
}

fn wal_segment_paths(dir: &Path) -> Vec<PathBuf> {
    let mut segments = fs::read_dir(dir.join("wal"))
        .expect("read wal directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension().and_then(|value| value.to_str()) == Some("wal")).then_some(path)
        })
        .collect::<Vec<_>>();
    segments.sort();
    segments
}

/// Issue #1992 end-to-end: a real durable vault that has rolled many WAL
/// segments must be able to retire every segment the verified durable
/// coverage floor already covers, keep only the active tail, and still read
/// every committed row back after a full reopen (replay is seeded from the
/// manifest floor, never from assuming segment 0 exists).
#[test]
fn prune_covered_wal_segments_retires_history_and_reopen_reads_every_row_back() {
    let dir = test_dir("wal-prune-e2e");
    let mut options = VaultOptions::default();
    // Force one segment per commit so a small write set rolls many segments.
    options.wal_options.max_segment_bytes = 1;
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", options.clone()).expect("open durable");

    let rows = 8u8;
    for index in 1..=rows {
        vault
            .put(distinct_constellation(index))
            .expect("durable put");
        // Flush after each commit so the manifest durable coverage advances to
        // the committed tip (checkpoint coverage), making the segment prunable.
        vault.flush().expect("flush durable");
    }
    let height_before = vault.latest_seq();
    assert_eq!(height_before, u64::from(rows));

    let segments_before = wal_segment_paths(&dir);
    assert!(
        segments_before.len() >= 2,
        "expected WAL rotation across commits: {segments_before:?}"
    );

    // All rows read back on the live handle before pruning.
    for index in 1..=rows {
        let id = CxId::from_bytes([index; 16]);
        assert_eq!(vault.get(id, vault.snapshot()).unwrap().cx_id, id);
    }

    let config = crate::wal::WalPruneConfig {
        max_segments: 1024,
        max_bytes: u64::MAX,
    };
    let outcome = vault.prune_covered_wal_segments(config).expect("prune");
    assert!(
        outcome.segments_pruned >= 1,
        "expected covered segments to be retired: {outcome:?}"
    );
    assert!(outcome.bytes_reclaimed > 0);
    // Only the active tail segment survives; ledger height is unchanged.
    assert_eq!(outcome.segments_after, 1);
    assert_eq!(
        vault.latest_seq(),
        height_before,
        "prune moved ledger height"
    );
    assert_eq!(wal_segment_paths(&dir).len(), 1);

    // Idempotent: a second prune at the same coverage floor is a no-op.
    let again = vault
        .prune_covered_wal_segments(config)
        .expect("second prune");
    assert_eq!(again.segments_pruned, 0);

    drop(vault);

    // Replay-correctness proof: reopen the full vault (default restore path)
    // and read every committed row back with the WAL history gone.
    let reopened = AsterVault::open(&dir, vault_id(), b"salt", options).expect("reopen");
    assert_eq!(reopened.latest_seq(), height_before);
    for index in 1..=rows {
        let id = CxId::from_bytes([index; 16]);
        assert_eq!(
            reopened.get(id, reopened.snapshot()).unwrap().cx_id,
            id,
            "row {index} lost after WAL prune + reopen"
        );
    }
    cleanup(dir);
}

/// The vault primitive flushes before deriving its coverage floor, so even
/// uncheckpointed writes present at prune time are first advanced into durable
/// coverage (never lost) — the active tail segment is preserved and every row,
/// including the previously-uncheckpointed ones, reads back.
#[test]
fn prune_flushes_uncheckpointed_writes_into_coverage_before_retiring() {
    let dir = test_dir("wal-prune-uncheckpointed");
    let mut options = VaultOptions::default();
    options.wal_options.max_segment_bytes = 1;
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", options.clone()).expect("open durable");

    // One flushed row, then several committed WITHOUT an explicit flush so they
    // sit above the on-disk manifest coverage floor at the moment prune runs.
    vault.put(distinct_constellation(1)).expect("put first");
    vault.flush().expect("flush first");
    for index in 2..=4u8 {
        vault
            .put(distinct_constellation(index))
            .expect("put uncheckpointed");
    }
    let height_before = vault.latest_seq();

    let config = crate::wal::WalPruneConfig {
        max_segments: 1024,
        max_bytes: u64::MAX,
    };
    let outcome = vault.prune_covered_wal_segments(config).expect("prune");
    // No data lost: ledger height unchanged and the active tail survives.
    assert_eq!(vault.latest_seq(), height_before);
    assert!(outcome.segments_after >= 1);

    // Every row — including the ones uncheckpointed at prune time — reads back,
    // both live and after a full reopen with the pruned history gone.
    for index in 1..=4u8 {
        let id = CxId::from_bytes([index; 16]);
        assert_eq!(vault.get(id, vault.snapshot()).unwrap().cx_id, id);
    }
    drop(vault);
    let reopened = AsterVault::open(&dir, vault_id(), b"salt", options).expect("reopen");
    for index in 1..=4u8 {
        let id = CxId::from_bytes([index; 16]);
        assert_eq!(reopened.get(id, reopened.snapshot()).unwrap().cx_id, id);
    }
    cleanup(dir);
}
