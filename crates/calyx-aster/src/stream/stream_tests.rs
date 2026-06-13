use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{CxId, LensId, Modality, SlotId, SlotVector, SystemClock, VaultId, VaultStore};
use calyx_forge::quant::QuantLevel;

use super::*;
use crate::cf::ColumnFamily;
use crate::dedup::DedupPolicy;
use crate::vault::{AsterVault, VaultOptions};

const SLOT_DIM: usize = 4;
const SALT: &[u8] = b"stream-ingest-test-salt";

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-stream-ingest-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

/// A durable vault (real ledger hook + on-disk WAL) with dedup disabled so each
/// distinct event persists as a new constellation.
fn durable_vault(dir: &PathBuf) -> Arc<AsterVault<SystemClock>> {
    let options = VaultOptions {
        dedup_policy: Some(DedupPolicy::Off),
        ..VaultOptions::default()
    };
    Arc::new(AsterVault::open(dir, vault_id(), SALT.to_vec(), options).expect("open durable vault"))
}

fn config() -> QuantizeOnlineConfig {
    QuantizeOnlineConfig::new(LensId::from_bytes([0x5A; 16]), QuantLevel::Bits3p5)
}

/// Distinct dense-slot event whose content is a deterministic function of `index`.
fn event_input(index: usize) -> IngestInput {
    let data: Vec<f32> = (0..SLOT_DIM)
        .map(|i| ((index * SLOT_DIM + i) as f32) * 0.125 - 0.5)
        .collect();
    IngestInput::new(
        format!("stream-event-{index}").into_bytes(),
        41,
        Modality::Text,
    )
    .with_slot(
        SlotId::new(0),
        SlotVector::Dense {
            dim: SLOT_DIM as u32,
            data,
        },
    )
}

fn cx_for(vault: &AsterVault<SystemClock>, index: usize) -> CxId {
    let input = event_input(index);
    vault.cx_id_for_input(&input.raw_bytes, input.panel_version)
}

fn scan(vault: &AsterVault<SystemClock>, cf: ColumnFamily) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault.scan_cf_at(vault.snapshot(), cf).expect("scan cf")
}

fn count_stream_batch_ledger_rows(vault: &AsterVault<SystemClock>) -> usize {
    let needle = STREAM_BATCH_MARKER.as_bytes();
    scan(vault, ColumnFamily::Ledger)
        .into_iter()
        .filter(|(_, value)| value.windows(needle.len()).any(|window| window == needle))
        .count()
}

#[test]
fn ten_events_persist_with_quantized_metadata() {
    let dir = test_dir("ten-events");
    let vault = durable_vault(&dir);
    // SoT BEFORE: empty vault.
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 0);

    let ingester = StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(64, 0));
    for index in 0..10 {
        ingester
            .send(event_input(index), EpochSecs(1_000 + index as i64))
            .expect("send within capacity");
    }
    let stats = ingester.drain_and_close().expect("clean shutdown");

    assert_eq!(stats.ingested, 10);
    assert_eq!(stats.quantized, 10, "one dense slot quantized per event");
    assert_eq!(stats.backpressured, 0);
    assert!(stats.batches >= 1);

    // SoT AFTER: exactly 10 base rows, each carrying the quantized metadata tag.
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 10);
    for index in 0..10 {
        let cx_id = cx_for(&vault, index);
        let constellation = vault.get(cx_id, vault.snapshot()).expect("readback cx");
        assert_eq!(
            constellation.metadata.get("quantized").map(String::as_str),
            Some("true"),
            "event {index} must be tagged quantized"
        );
        let quant_hex = constellation
            .metadata
            .get("quant_slot_0")
            .expect("quantized slot bytes present in metadata");
        assert!(!quant_hex.is_empty());
        assert!(quant_hex.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    // One STREAM_BATCH Ledger entry per microbatch, written through the real hook.
    assert_eq!(count_stream_batch_ledger_rows(&vault), stats.batches);

    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn quantized_metadata_is_bit_identical_across_runs() {
    // A25: re-running the same event through a fresh vault yields byte-identical
    // quantized slot bytes (content-addressed seed, never random).
    let run = |tag: &str, index: usize| -> String {
        let dir = test_dir(tag);
        let value = {
            let vault = durable_vault(&dir);
            let ingester =
                StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(8, 0));
            ingester
                .send(event_input(index), EpochSecs(500))
                .expect("send");
            ingester.drain_and_close().expect("shutdown");
            let cx_id = cx_for(&vault, index);
            vault
                .get(cx_id, vault.snapshot())
                .expect("readback")
                .metadata
                .get("quant_slot_0")
                .expect("quant bytes")
                .clone()
        };
        let _ = fs::remove_dir_all(&dir);
        value
    };
    assert_eq!(
        run("bitid-a", 3),
        run("bitid-b", 3),
        "same seed -> bit-identical quantized bytes"
    );
}

#[test]
fn zero_events_writes_nothing() {
    let dir = test_dir("zero-events");
    let vault = durable_vault(&dir);
    let ingester = StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(8, 0));
    let stats = ingester.drain_and_close().expect("clean shutdown");

    assert_eq!(stats.ingested, 0);
    assert_eq!(stats.batches, 0);
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 0);
    assert_eq!(
        count_stream_batch_ledger_rows(&vault),
        0,
        "no microbatch -> no STREAM_BATCH ledger entry"
    );
    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn backpressure_trips_exactly_at_capacity() {
    let dir = test_dir("backpressure");
    let vault = durable_vault(&dir);
    // Capacity 5, no refill: the 6th send must fail closed.
    let ingester = StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(5, 0));
    for index in 0..5 {
        ingester
            .send(event_input(index), EpochSecs(1_000 + index as i64))
            .unwrap_or_else(|_| panic!("send {index} within capacity"));
    }
    let err = ingester
        .send(event_input(5), EpochSecs(1_005))
        .expect_err("6th send must be backpressured");
    assert_eq!(err.code, CALYX_STREAM_BACKPRESSURE);

    let stats = ingester.drain_and_close().expect("shutdown");
    assert_eq!(stats.backpressured, 1);
    assert_eq!(stats.ingested, 5, "only the admitted events were persisted");
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 5);
    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn explicit_past_event_time_is_not_silently_restamped() {
    let dir = test_dir("backfill");
    let vault = durable_vault(&dir);
    let ingester = StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(8, 0));
    // Explicit past event time (seconds); the wall clock is "now".
    let past = EpochSecs(1_234);
    ingester.send(event_input(0), past).expect("send backfill");
    let stats = ingester.drain_and_close().expect("shutdown");
    assert_eq!(stats.ingested, 1);

    let cx_id = cx_for(&vault, 0);
    let constellation = vault.get(cx_id, vault.snapshot()).expect("readback");
    assert_eq!(
        constellation.created_at, 1_234,
        "created_at must equal the explicit event time, not the clock"
    );
    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nan_slot_fails_closed_at_send_and_writes_nothing() {
    let dir = test_dir("nan");
    let vault = durable_vault(&dir);
    let ingester = StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(8, 0));

    let mut input = event_input(0);
    if let Some(SlotVector::Dense { data, .. }) = input.slots.get_mut(&SlotId::new(0)) {
        data[2] = f32::NAN;
    }
    let err = ingester
        .send(input, EpochSecs(1_000))
        .expect_err("NaN slot must fail closed");
    assert_eq!(err.code, CALYX_FORGE_INPUT_NAN);

    let stats = ingester.drain_and_close().expect("shutdown");
    assert_eq!(stats.ingested, 0, "rejected event is never persisted");
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 0);
    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}
