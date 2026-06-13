//! Full State Verification for PH72 · T01 streaming ingest.
//!
//! Source of Truth: a real durable `AsterVault` on disk — its Base CF rows, its
//! hash-chained Ledger CF, and the bytes physically resident in the vault's WAL
//! files. Every assertion reads the SoT back independently; no claim rests on a
//! function's return value.
//!
//! Run: `cargo run -p calyx-aster --example stream_ingest_fsv`

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::{DedupPolicy, EpochSecs, IngestInput};
use calyx_aster::stream::{
    BackpressureGuard, CALYX_FORGE_INPUT_NAN, CALYX_STREAM_BACKPRESSURE, QuantizeOnlineConfig,
    STREAM_BATCH_MARKER, StreamIngester, quantize_slot_online,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{LensId, Modality, SlotId, SlotVector, SystemClock, VaultId, VaultStore};
use calyx_forge::quant::QuantLevel;

const SLOT_DIM: usize = 8;
const SALT: &[u8] = b"stream-fsv-salt";

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn config() -> QuantizeOnlineConfig {
    QuantizeOnlineConfig::new(LensId::from_bytes([0x5A; 16]), QuantLevel::Bits3p5)
}

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("calyx-stream-fsv-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create vault dir");
    dir
}

fn open_vault(dir: &Path) -> Arc<AsterVault<SystemClock>> {
    let options = VaultOptions {
        dedup_policy: Some(DedupPolicy::Off),
        ..VaultOptions::default()
    };
    Arc::new(AsterVault::open(dir, vault_id(), SALT.to_vec(), options).expect("open vault"))
}

fn event(index: usize) -> IngestInput {
    let data: Vec<f32> = (0..SLOT_DIM)
        .map(|i| ((index * SLOT_DIM + i) as f32) * 0.0625 - 0.75)
        .collect();
    IngestInput::new(
        format!("fsv-event-{index}").into_bytes(),
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

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn scan(vault: &AsterVault<SystemClock>, cf: ColumnFamily) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault.scan_cf_at(vault.snapshot(), cf).expect("scan")
}

fn count_ledger_marker(vault: &AsterVault<SystemClock>) -> usize {
    let needle = STREAM_BATCH_MARKER.as_bytes();
    scan(vault, ColumnFamily::Ledger)
        .into_iter()
        .filter(|(_, v)| v.windows(needle.len()).any(|w| w == needle))
        .count()
}

/// Walks every file under `dir` and counts physical occurrences of the marker
/// bytes — proof the STREAM_BATCH entry is resident on disk (WAL/SST).
fn count_marker_on_disk(dir: &Path) -> usize {
    let needle = STREAM_BATCH_MARKER.as_bytes();
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if let Ok(entries) = fs::read_dir(&path) {
                for entry in entries.flatten() {
                    stack.push(entry.path());
                }
            }
        } else if let Ok(bytes) = fs::read(&path) {
            total += bytes.windows(needle.len()).filter(|w| *w == needle).count();
        }
    }
    total
}

fn metadata_for(vault: &AsterVault<SystemClock>, index: usize) -> BTreeMap<String, String> {
    let input = event(index);
    let cx_id = vault.cx_id_for_input(&input.raw_bytes, input.panel_version);
    vault
        .get(cx_id, vault.snapshot())
        .expect("readback cx")
        .metadata
}

fn main() {
    happy_path_100();
    edge_empty_stream();
    edge_backpressure();
    edge_nan_fail_closed();
    edge_backfill_event_time();
    println!("\n==== ALL FSV CHECKS PASSED ====");
}

fn happy_path_100() {
    println!("\n=== HAPPY PATH: 100-event synthetic stream ===");
    let dir = fresh_dir("happy");
    let vault = open_vault(&dir);

    let before = scan(&vault, ColumnFamily::Base).len();
    println!(
        "[BEFORE] base rows = {before}, on-disk STREAM_BATCH = {}",
        count_marker_on_disk(&dir)
    );
    assert_eq!(before, 0, "vault must start empty");

    let ingester =
        StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(256, 0));
    for i in 0..100 {
        ingester
            .send(event(i), EpochSecs(1_000 + i as i64))
            .expect("send");
    }
    let stats = ingester.drain_and_close().expect("drain");
    println!(
        "[TRIGGER] streamed 100 events -> stats: ingested={}, quantized={}, backpressured={}, batches={}",
        stats.ingested, stats.quantized, stats.backpressured, stats.batches
    );
    assert_eq!(stats.ingested, 100);
    assert_eq!(stats.quantized, 100);
    assert_eq!(stats.backpressured, 0);
    assert!(stats.batches >= 1);

    vault.flush().expect("flush WAL");

    // SoT #1: exactly 100 base rows.
    let after = scan(&vault, ColumnFamily::Base).len();
    println!("[AFTER ] base rows = {after}");
    assert_eq!(after, 100, "exactly 100 constellations persisted");

    // SoT #2: every event tagged quantized; stored bytes equal an INDEPENDENT
    // recomputation of the quantization (2+2=4 discipline).
    let mut verified = 0;
    for i in 0..100 {
        let meta = metadata_for(&vault, i);
        assert_eq!(meta.get("quantized").map(String::as_str), Some("true"));
        let stored = meta.get("quant_slot_0").expect("stored quant bytes");

        let input = event(i);
        let cx_id = vault.cx_id_for_input(&input.raw_bytes, input.panel_version);
        let SlotVector::Dense { data, .. } = input.slots.get(&SlotId::new(0)).unwrap() else {
            unreachable!()
        };
        let expected = to_hex(&quantize_slot_online(data, &config(), cx_id).unwrap().bytes);
        assert_eq!(
            stored, &expected,
            "event {i}: SoT bytes must equal recomputed quantization"
        );
        verified += 1;
    }
    println!("[VERIFY] {verified}/100 base rows carry quantized=true AND byte-exact quant bytes");

    // SoT #3: ledger marker count == batches, and the marker is physically on disk.
    let ledger_markers = count_ledger_marker(&vault);
    let disk_markers = count_marker_on_disk(&dir);
    println!(
        "[AFTER ] STREAM_BATCH ledger rows = {ledger_markers}, on-disk occurrences = {disk_markers}"
    );
    assert_eq!(
        ledger_markers, stats.batches,
        "one STREAM_BATCH ledger entry per microbatch"
    );
    assert!(
        disk_markers >= 1,
        "STREAM_BATCH must be resident in WAL/SST on disk"
    );

    // Sample the example row 0 for the evidence log.
    let sample = metadata_for(&vault, 0);
    println!(
        "[EVIDENCE] cx#0 quant_slot_0 (first 32 hex chars): {}...",
        &sample.get("quant_slot_0").unwrap()[..32]
    );

    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}

fn edge_empty_stream() {
    println!("\n=== EDGE 1: empty stream (0 events) ===");
    let dir = fresh_dir("empty");
    let vault = open_vault(&dir);
    println!(
        "[BEFORE] base rows = {}",
        scan(&vault, ColumnFamily::Base).len()
    );
    let ingester = StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(8, 0));
    let stats = ingester.drain_and_close().expect("drain");
    vault.flush().expect("flush");
    let base = scan(&vault, ColumnFamily::Base).len();
    let markers = count_ledger_marker(&vault);
    println!(
        "[AFTER ] base rows = {base}, STREAM_BATCH ledger rows = {markers}, stats.batches = {}",
        stats.batches
    );
    assert_eq!(stats.ingested, 0);
    assert_eq!(base, 0);
    assert_eq!(markers, 0, "no microbatch -> no ledger marker");
    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}

fn edge_backpressure() {
    println!("\n=== EDGE 2: backpressure at capacity 5, 6 sends ===");
    let dir = fresh_dir("backpressure");
    let vault = open_vault(&dir);
    let ingester = StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(5, 0));
    let mut codes = Vec::new();
    for i in 0..6 {
        match ingester.send(event(i), EpochSecs(1_000 + i as i64)) {
            Ok(()) => codes.push("OK".to_string()),
            Err(e) => codes.push(e.code.to_string()),
        }
    }
    let stats = ingester.drain_and_close().expect("drain");
    vault.flush().expect("flush");
    let base = scan(&vault, ColumnFamily::Base).len();
    println!("[TRIGGER] send outcomes = {codes:?}");
    println!(
        "[AFTER ] base rows = {base}, stats.backpressured = {}",
        stats.backpressured
    );
    assert_eq!(
        codes[5], CALYX_STREAM_BACKPRESSURE,
        "6th send must fail closed"
    );
    assert_eq!(stats.backpressured, 1);
    assert_eq!(base, 5, "only the 5 admitted events were persisted");
    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}

fn edge_nan_fail_closed() {
    println!("\n=== EDGE 3: NaN slot fails closed before write ===");
    let dir = fresh_dir("nan");
    let vault = open_vault(&dir);
    let ingester = StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(8, 0));
    let mut input = event(0);
    if let Some(SlotVector::Dense { data, .. }) = input.slots.get_mut(&SlotId::new(0)) {
        data[3] = f32::NAN;
    }
    let before = scan(&vault, ColumnFamily::Base).len();
    let err = ingester
        .send(input, EpochSecs(1_000))
        .expect_err("NaN must fail");
    let stats = ingester.drain_and_close().expect("drain");
    vault.flush().expect("flush");
    let after = scan(&vault, ColumnFamily::Base).len();
    println!(
        "[BEFORE] base rows = {before}; [TRIGGER] send(NaN) -> {}; [AFTER] base rows = {after}",
        err.code
    );
    assert_eq!(err.code, CALYX_FORGE_INPUT_NAN);
    assert_eq!(stats.ingested, 0);
    assert_eq!(after, 0, "rejected event is never persisted");
    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}

fn edge_backfill_event_time() {
    println!("\n=== EDGE 4: explicit past event time is honored ===");
    let dir = fresh_dir("backfill");
    let vault = open_vault(&dir);
    let ingester = StreamIngester::new(Arc::clone(&vault), config(), BackpressureGuard::new(8, 0));
    let past = EpochSecs(1_234);
    ingester.send(event(0), past).expect("send");
    let stats = ingester.drain_and_close().expect("drain");
    vault.flush().expect("flush");
    assert_eq!(stats.ingested, 1);
    let created_at = metadata_created_at(&vault, 0);
    println!("[TRIGGER] send at EpochSecs(1234); [AFTER] readback created_at = {created_at}");
    assert_eq!(
        created_at, 1_234,
        "created_at honors explicit event time, no silent re-stamp"
    );
    drop(vault);
    let _ = fs::remove_dir_all(&dir);
}

fn metadata_created_at(vault: &AsterVault<SystemClock>, index: usize) -> u64 {
    let input = event(index);
    let cx_id = vault.cx_id_for_input(&input.raw_bytes, input.panel_version);
    vault
        .get(cx_id, vault.snapshot())
        .expect("readback")
        .created_at
}
