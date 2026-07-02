//! Regression tests for issue #1138: router flush ordinals and commit seqs
//! are incomparable domains, so latest-only reads must never let an older
//! flush shadow a newer durable batch, and ambiguous legacy layouts must
//! fail closed instead of serving stale rows.

use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn latest_only_options() -> VaultOptions {
    VaultOptions {
        restore_mvcc_rows: false,
        restore_ledger_hook: false,
        read_only: true,
        ..VaultOptions::default()
    }
}

/// The #1138 inversion, end to end. A tiny memtable cap forces one router
/// flush per row during commit 1, so the flush ordinal chain (1..=N) runs
/// numerically past commit seq 2. Commit 2 rewrites a key; the crash window
/// between `durable.flush()` (durable batches + manifest, first) and
/// `flush_all_cfs()` (router memtable flush, second) is simulated by removing
/// the router flush that carries the rewrite — its rows all have durable
/// homes, so the layout is physically valid. The newest value then lives ONLY
/// in the durable batch at commit seq 2, and pre-#1138 ordering sorted that
/// batch BEHIND the stale high-ordinal flush, silently returning the old
/// value on every latest-only read.
#[test]
fn latest_only_read_prefers_newer_durable_batch_over_older_flush() {
    let dir = test_dir("flush-vs-batch");
    let options = VaultOptions {
        memtable_byte_cap: 512,
        ..VaultOptions::default()
    };
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt", options.clone()).expect("open durable");

    // Commit 1: filler rows force flushes; the target key goes last so its
    // stale value lands in the highest-ordinal flush.
    let stale = vec![0x41_u8; 400];
    let mut rows = (0..4)
        .map(|index| {
            (
                ColumnFamily::Kv,
                format!("filler-{index}").into_bytes(),
                vec![0x42_u8; 400],
            )
        })
        .collect::<Vec<_>>();
    rows.push((ColumnFamily::Kv, b"target-key".to_vec(), stale.clone()));
    vault.write_cf_batch(rows).expect("commit 1");
    vault
        .flush_all_cfs()
        .expect("flush stale target to router SST");

    // Commit 2 rewrites the target, then a full flush writes the durable
    // batches, the manifest, and one more router flush with the fresh value.
    vault
        .write_cf_batch([(ColumnFamily::Kv, b"target-key".to_vec(), b"fresh".to_vec())])
        .expect("commit 2");
    vault.flush().expect("durable flush");
    drop(vault);

    // Simulate the crash window: the last router flush (the only flush-chain
    // copy of the fresh value) is lost; the durable batch at commit seq 2
    // survives. Prove the precondition physically before relying on it.
    let kv_dir = dir.join("cf").join(ColumnFamily::Kv.name());
    let newest_flush = newest_flush_file(&kv_dir);
    assert!(
        sst_contains(&newest_flush, b"target-key", b"fresh"),
        "test precondition: fresh value flushed to {}",
        newest_flush.display()
    );
    fs::remove_file(&newest_flush).expect("drop newest router flush");
    let fresh_homes = sst_files_containing(&kv_dir, b"target-key", b"fresh");
    assert!(
        !fresh_homes.is_empty()
            && fresh_homes.iter().all(|path| {
                matches!(
                    crate::storage_names::classify_sst(path),
                    Ok(Some(crate::storage_names::SstName::DurableBatch {
                        seq: 2,
                        ..
                    }))
                )
            }),
        "test precondition: fresh value must live only in the seq-2 durable batch: {fresh_homes:?}"
    );
    let stale_homes = sst_files_containing(&kv_dir, b"target-key", &stale);
    assert!(
        stale_homes.iter().any(|path| {
            matches!(
                crate::storage_names::classify_sst(path),
                Ok(Some(crate::storage_names::SstName::Flush {
                    watermark: 1,
                    ..
                }))
            )
        }),
        "test precondition: stale value must live in a watermark-1 flush: {stale_homes:?}"
    );

    let reader = AsterVault::open(&dir, vault_id(), b"salt", latest_only_options())
        .expect("latest-only open");
    let got = reader
        .read_cf_at(reader.snapshot(), ColumnFamily::Kv, b"target-key")
        .expect("latest read")
        .expect("target key visible");
    assert_eq!(
        got, b"fresh",
        "latest-only read returned the stale flushed value: the newer durable batch was shadowed \
         by an older router flush (issue #1138)"
    );
    drop(reader);
    cleanup(dir);
}

/// A vault whose CF holds legacy flush files with ordinals numerically above
/// a commit-domain seq must refuse latest-only opens with the typed #1138
/// error instead of guessing an order.
#[test]
fn latest_only_open_fails_closed_on_ambiguous_legacy_layout() {
    let dir = test_dir("ambiguous-legacy");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    vault
        .write_cf_batch([(ColumnFamily::Kv, b"k".to_vec(), b"v".to_vec())])
        .expect("commit 1");
    vault.flush().expect("durable flush");
    drop(vault);

    // Plant a legacy-shaped flush whose ordinal exceeds the commit seqs of
    // the durable batches written above (pre-#1138 writer crash artifact).
    let kv_dir = dir.join("cf").join(ColumnFamily::Kv.name());
    crate::sst::write_sst(
        kv_dir.join("00000000000000000099.sst"),
        [(b"k".as_slice(), b"legacy".as_slice())],
    )
    .expect("plant legacy flush");

    let error = AsterVault::open(&dir, vault_id(), b"salt", latest_only_options())
        .expect_err("ambiguous layout must fail closed");
    assert_eq!(error.code, "CALYX_ASTER_SST_ORDER_AMBIGUOUS");
    cleanup(dir);
}

fn newest_flush_file(dir: &std::path::Path) -> PathBuf {
    fs::read_dir(dir)
        .expect("read CF dir")
        .map(|entry| entry.expect("CF entry").path())
        .filter(|path| {
            matches!(
                crate::storage_names::classify_sst(path),
                Ok(Some(crate::storage_names::SstName::Flush { .. }))
            )
        })
        .max_by_key(|path| {
            crate::storage_names::sst_order_key(path)
                .expect("canonical flush name")
                .expect("order key")
        })
        .expect("at least one flush SST")
}

fn sst_contains(path: &std::path::Path, key: &[u8], value: &[u8]) -> bool {
    crate::sst::SstReader::open(path)
        .expect("open SST")
        .get(key)
        .expect("read SST")
        .is_some_and(|got| got == value)
}

fn sst_files_containing(dir: &std::path::Path, key: &[u8], value: &[u8]) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .expect("read CF dir")
        .map(|entry| entry.expect("CF entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sst"))
        .filter(|path| sst_contains(path, key, value))
        .collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-issue1138-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}
