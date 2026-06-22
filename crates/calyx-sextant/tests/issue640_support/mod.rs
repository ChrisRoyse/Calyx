use std::collections::BTreeMap;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::WalOptions;
use calyx_core::{CxFlags, InputRef, LedgerRef, Modality, SlotId, VaultId, VaultStore};
use calyx_sextant::{
    CALYX_SEXTANT_DIM_MISMATCH, CALYX_SEXTANT_EF_TOO_SMALL, CALYX_SEXTANT_INDEX_EMPTY,
    FusionStrategy, HnswIndex, InvertedIndex, Query, SearchEngine, SextantIndex, SlotIndexMap,
};
use serde_json::{Value, json};

#[path = "../sextant_support/mod.rs"]
mod sextant_support;
pub use sextant_support::digest_hex;
use sextant_support::{cx_usize_be as cx, dense};

pub const ISSUE: u64 = 640;
pub const DEFAULT_SCALE_CX: usize = 1_000_000;
pub const DEFAULT_DIM: usize = 128;
pub const DEFAULT_QUERY_COUNT: usize = 1_000;
pub const DEFAULT_INGEST_SAMPLES: usize = 1_024;
pub const LENS_COUNT: usize = 6;
const BATCHED_SLOT_COUNT: usize = 15;
const PIPELINE_STAGE1_SLOT: u16 = 100;
const K: usize = 10;
const EF: usize = 96;

#[derive(Clone, Debug)]
pub struct Series {
    pub latencies_us: Vec<u128>,
    pub p95_us: u128,
    pub p99_us: u128,
    pub max_us: u128,
}

impl Series {
    fn from(values: Vec<u128>) -> Self {
        let p95_us = percentile(&mut values.clone(), 95);
        let p99_us = percentile(&mut values.clone(), 99);
        let max_us = values.iter().copied().max().unwrap_or(0);
        Self {
            latencies_us: values,
            p95_us,
            p99_us,
            max_us,
        }
    }

    pub fn to_json(&self, target_p99_us: u128) -> Value {
        json!({
            "count": self.latencies_us.len(),
            "p95_us": self.p95_us,
            "p99_us": self.p99_us,
            "max_us": self.max_us,
            "target_p99_us": target_p99_us,
            "pass": target_p99_us == 0 || self.p99_us <= target_p99_us,
            "latencies_us": self.latencies_us
        })
    }
}

pub fn build_indexes(n: usize, dim: usize) -> (Vec<HnswIndex>, InvertedIndex) {
    let handles: Vec<_> = (0..LENS_COUNT)
        .map(|slot| thread::spawn(move || build_hnsw_slot(slot, n, dim)))
        .collect();
    let stage1 = thread::spawn(move || build_stage1(n)).join().unwrap();
    let indexes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    (indexes, stage1)
}

fn build_hnsw_slot(slot: usize, n: usize, dim: usize) -> HnswIndex {
    let mut index = HnswIndex::new(SlotId::new(slot as u16), dim as u32, ISSUE + slot as u64);
    for ordinal in 0..n {
        index
            .insert(
                cx(ordinal),
                dense(unit_vector(ordinal, dim, 0)),
                ordinal as u64 + 1,
            )
            .unwrap();
        if slot == 0 && ordinal > 0 && ordinal % 100_000 == 0 {
            eprintln!("issue640 build slot0 inserted {ordinal}/{n} cx");
        }
    }
    index
}

fn build_stage1(n: usize) -> InvertedIndex {
    let mut stage1 = InvertedIndex::new(SlotId::new(PIPELINE_STAGE1_SLOT));
    for ordinal in 0..n {
        stage1
            .insert_text(cx(ordinal), &stage1_doc_text(ordinal), ordinal as u64 + 1)
            .unwrap();
    }
    stage1
}

pub fn engine_from_indexes(indexes: Vec<HnswIndex>, stage1: InvertedIndex) -> SearchEngine {
    let map = SlotIndexMap::new();
    for index in indexes {
        map.register(index).unwrap();
    }
    map.register(stage1).unwrap();
    SearchEngine::new(map)
}

pub fn known_search_readback(engine: &SearchEngine, dim: usize, scale_cx: usize) -> Value {
    let target = (scale_cx / 2).max(1);
    let expected = cx(target);
    let mut query =
        Query::new(stage1_query_text(target)).with_vector(dense(unit_vector(target, dim, 0)));
    query.slots = vec![SlotId::new(0)];
    query.k = K;
    query.ef = Some(EF);
    query.fusion = Some(FusionStrategy::SingleLens {
        slot: SlotId::new(0),
    });
    let hits = engine.search(&query).unwrap();
    let actual_top = hits[0].cx_id;
    json!({
        "trigger": "query vector is byte-identical to stored synthetic vector",
        "expected_top_cx": expected.to_string(),
        "actual_top_cx": actual_top.to_string(),
        "actual_score": hits[0].score,
        "match": actual_top == expected
    })
}

pub fn measure_search(
    engine: &SearchEngine,
    dim: usize,
    query_ids: &[usize],
    slots: &[SlotId],
    strategy: FusionStrategy,
    explain: bool,
) -> Series {
    let mut latencies = Vec::with_capacity(query_ids.len());
    for &id in query_ids {
        let mut query =
            Query::new(stage1_query_text(id)).with_vector(dense(unit_vector(id, dim, 0)));
        query.slots = slots.to_vec();
        query.k = K;
        query.ef = Some(EF);
        query.recall_k = Some(K * 6);
        query.fusion = Some(strategy.clone());
        query.explain = explain;
        let started = Instant::now();
        let hits = engine.search(&query).unwrap();
        black_box(hits.len());
        assert!(!hits.is_empty(), "empty hits for {:?}", strategy);
        latencies.push(started.elapsed().as_micros());
    }
    Series::from(latencies)
}

pub fn search_edge_errors(dim: usize) -> Value {
    let empty = HnswIndex::new(SlotId::new(99), dim as u32, ISSUE);
    let empty_error = empty
        .search(&dense(unit_vector(0, dim, 0)), 1, Some(1))
        .unwrap_err()
        .code;
    let mut index = HnswIndex::new(SlotId::new(98), dim as u32, ISSUE);
    index
        .insert(cx(1), dense(unit_vector(1, dim, 0)), 1)
        .unwrap();
    index
        .insert(cx(2), dense(unit_vector(2, dim, 0)), 2)
        .unwrap();
    let ef_error = index
        .search(&dense(unit_vector(1, dim, 0)), 2, Some(1))
        .unwrap_err()
        .code;
    let dim_error = index
        .search(&dense(unit_vector(1, dim + 1, 0)), 1, Some(1))
        .unwrap_err()
        .code;
    json!({
        "empty_index": {"expected": CALYX_SEXTANT_INDEX_EMPTY, "actual": empty_error},
        "ef_too_small": {"expected": CALYX_SEXTANT_EF_TOO_SMALL, "actual": ef_error},
        "dim_mismatch": {"expected": CALYX_SEXTANT_DIM_MISMATCH, "actual": dim_error}
    })
}

pub fn measure_ingest(vault_dir: &Path, dim: usize, samples: usize) -> Value {
    let vault = AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue640-salt".to_vec(),
        vault_options(),
    )
    .unwrap();
    let one_slot = measure_ingest_series(&vault, dim, samples, 1, 10_000_000);
    let fifteen_slot = measure_ingest_series(&vault, dim, samples, BATCHED_SLOT_COUNT, 20_000_000);
    let read_id = vault.cx_id_for_input(b"issue640-cold-open-readback", 640);
    vault
        .put(constellation(
            &vault,
            b"issue640-cold-open-readback",
            dim,
            1,
            640,
        ))
        .unwrap();
    let before = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(read_id))
        .unwrap()
        .unwrap();
    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(
        vault_dir,
        vault_id(),
        b"issue640-salt".to_vec(),
        vault_options(),
    )
    .unwrap();
    let after = reopened
        .read_cf_at(reopened.snapshot(), ColumnFamily::Base, &base_key(read_id))
        .unwrap()
        .unwrap();
    json!({
        "one_slot": ingest_json(&one_slot, 5_000),
        "fifteen_slot": ingest_json(&fifteen_slot, 20_000),
        "cold_open_edge": {
            "cx": read_id.to_string(),
            "before_base_bytes": before.len(),
            "after_base_bytes": after.len(),
            "match": before == after
        }
    })
}

pub fn slot_ids(count: usize) -> Vec<SlotId> {
    (0..count).map(|slot| SlotId::new(slot as u16)).collect()
}

pub fn pipeline_slots() -> Vec<SlotId> {
    let mut slots = vec![SlotId::new(PIPELINE_STAGE1_SLOT)];
    slots.extend(slot_ids(LENS_COUNT));
    slots
}

pub fn query_ids(scale_cx: usize, count: usize) -> Vec<usize> {
    (0..count)
        .map(|idx| (idx * 104_729 + 17) % scale_cx.max(1))
        .collect()
}

pub fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn enforce_budgets(scale_cx: usize) -> bool {
    if let Ok(value) = std::env::var("CALYX_ISSUE640_ENFORCE_BUDGETS") {
        return value != "0";
    }
    !cfg!(debug_assertions) && scale_cx >= DEFAULT_SCALE_CX
}

pub fn fsv_root() -> PathBuf {
    if let Ok(root) = std::env::var("CALYX_ISSUE640_ROOT") {
        return PathBuf::from(root);
    }
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::env::temp_dir().join(format!("fsv-issue640-embedded-scale-{epoch}"))
}

pub fn shell_capture(command: &str, args: &[&str]) -> Value {
    match Command::new(command).args(args).output() {
        Ok(output) => json!({
            "status": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr)
        }),
        Err(error) => json!({"error": error.to_string()}),
    }
}

pub fn count_ext(path: &Path, ext: &str) -> u64 {
    walk_files(path)
        .into_iter()
        .filter(|file| file.extension().is_some_and(|actual| actual == ext))
        .count() as u64
}

pub fn dir_bytes(path: &Path) -> u64 {
    walk_files(path)
        .into_iter()
        .filter_map(|file| fs::metadata(file).ok().map(|meta| meta.len()))
        .sum()
}

fn measure_ingest_series(
    vault: &AsterVault,
    dim: usize,
    samples: usize,
    slot_count: usize,
    start: usize,
) -> Series {
    let mut latencies = Vec::with_capacity(samples);
    for offset in 0..samples {
        let input = format!("issue640-ingest-{slot_count}-{offset}");
        let cx = constellation(vault, input.as_bytes(), dim, slot_count, start + offset);
        let started = Instant::now();
        let stored = vault.put(cx).unwrap();
        black_box(stored);
        latencies.push(started.elapsed().as_micros());
    }
    Series::from(latencies)
}

fn ingest_json(series: &Series, target_p95_us: u128) -> Value {
    json!({
        "count": series.latencies_us.len(),
        "p95_us": series.p95_us,
        "p99_us": series.p99_us,
        "max_us": series.max_us,
        "target_p95_us": target_p95_us,
        "pass": series.p95_us <= target_p95_us,
        "latencies_us": series.latencies_us
    })
}

fn constellation(
    vault: &AsterVault,
    input: &[u8],
    dim: usize,
    slot_count: usize,
    ordinal: usize,
) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    for slot in 0..slot_count {
        slots.insert(
            SlotId::new(slot as u16),
            dense(unit_vector(ordinal, dim, slot)),
        );
    }
    calyx_core::Constellation {
        cx_id: vault.cx_id_for_input(input, 640),
        vault_id: vault_id(),
        panel_version: 640,
        created_at: 1_706_623_200 + ordinal as u64,
        input_ref: InputRef {
            hash: digest_parts(&[input]),
            pointer: Some(format!("synthetic://issue640/{ordinal}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: ordinal as u64 + 1,
            hash: digest_parts(&[b"issue640-ledger", &ordinal.to_be_bytes()]),
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn vault_options() -> VaultOptions {
    VaultOptions {
        wal_options: WalOptions {
            group_commit_window: std::time::Duration::from_millis(0),
            ..WalOptions::default()
        },
        memtable_byte_cap: 512 * 1024 * 1024,
        ..VaultOptions::default()
    }
}

fn stage1_doc_text(ordinal: usize) -> String {
    format!("issue640exact{ordinal} issue640bucket{}", ordinal % 1024)
}

fn stage1_query_text(ordinal: usize) -> String {
    format!("issue640exact{ordinal}")
}

#[test]
fn stage1_exact_terms_are_tokenizer_stable() {
    let query = stage1_query_text(17);
    assert!(query.chars().all(|ch| ch.is_ascii_alphanumeric()));
    assert!(
        stage1_doc_text(17)
            .split_whitespace()
            .any(|term| term == query)
    );
}

fn unit_vector(i: usize, dim: usize, salt: usize) -> Vec<f32> {
    let t = (i as f32 + salt as f32 * 0.031) * 0.013;
    let mut data = vec![0.0_f32; dim];
    data[0] = t.cos();
    data[1] = t.sin();
    for (axis, value) in data.iter_mut().enumerate().skip(2) {
        *value = (((i + salt * 13 + axis * 17) % 31) as f32 - 15.0) * 0.002;
    }
    normalize(data)
}

fn normalize(mut data: Vec<f32>) -> Vec<f32> {
    let norm = data.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut data {
        *value /= norm;
    }
    data
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn percentile(values: &mut [u128], percentile: usize) -> u128 {
    values.sort_unstable();
    values[((values.len() * percentile).div_ceil(100)).saturating_sub(1)]
}

fn walk_files(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walk_files(&path));
            } else {
                files.push(path);
            }
        }
    }
    files
}

fn digest_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(part);
    }
    *hasher.finalize().as_bytes()
}
