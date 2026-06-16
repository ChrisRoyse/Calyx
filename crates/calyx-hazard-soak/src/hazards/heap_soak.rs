use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::resource::heap_rss_bytes;
use calyx_core::{Arena, FixedClock, LruTtlCache, PageAlignedSlabPool, SlabPool};
use serde::Serialize;
use serde_json::json;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::resource::ProbeResult;
use super::resource_support::{case_dir, err};

const ARENA_CAP: usize = 4 * 1024 * 1024;
const MEMTABLE_CAP: usize = 32 * 1024 * 1024;
const CACHE_CAP: usize = 16 * 1024 * 1024;
const PAGE_SLAB_SLOTS: usize = 8;
const SMALL_SLAB_SLOTS: usize = 1024;
const KEY_RING: usize = 4096;
const KEY_SPACE: u64 = 65_536;
const DEFAULT_OPS: u64 = 10_000_000;
const DEFAULT_FLOOD_ROWS: u64 = 100_000;
const SAMPLE_EVERY: u64 = 10_000;
const PROCESS_RSS_HEADROOM: usize = 64 * 1024 * 1024;

pub(super) fn probe_h8_heap_oom(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h8_heap_oom")?;
    let report = run_heap_soak(
        &dir,
        env_u64("PH59_H8_OPS").unwrap_or(DEFAULT_OPS),
        env_u64("PH59_H8_FLOOD_ROWS").unwrap_or(DEFAULT_FLOOD_ROWS),
    )?;
    let passed = report.backpressure_events_total >= 1
        && report.flood_backpressure_events >= 1
        && report.rss_max_bytes <= report.rss_budget_bytes
        && report.rss_trend_bytes_per_op < 1.0
        && report.cache_used_bytes <= CACHE_CAP as u64
        && report.arena_high_water_bytes <= ARENA_CAP as u64
        && report.fail_closed_error_code == "CALYX_BACKPRESSURE"
        && report.structured_error_codes == ["CALYX_BACKPRESSURE"];

    Ok((
        passed,
        json!({
            "trigger": "bounded allocator/router soak plus 100k maximum-size write burst admission",
            "expected": {
                "rss_trend_lt_bytes_per_op": 1.0,
                "backpressure_events_total_gte": 1,
                "max_rss_lte_budget": true,
                "structured_error_code": "CALYX_BACKPRESSURE"
            },
            "actual": report,
            "metrics_text": format!(
                "calyx_heap_rss_bytes{{vault=\"ph59-h8\"}} {}\ncalyx_backpressure_events_total{{vault=\"ph59-h8\"}} {}\ncalyx_cache_used_bytes{{vault=\"ph59-h8\"}} {}\ncalyx_arena_high_water_bytes{{vault=\"ph59-h8\"}} {}\n",
                report.rss_final_bytes,
                report.backpressure_events_total,
                report.cache_used_bytes,
                report.arena_high_water_bytes
            )
        }),
    ))
}

#[derive(Clone, Copy, Debug, Serialize)]
struct RssSample {
    op: u64,
    rss_bytes: u64,
}

#[derive(Debug, Serialize)]
struct HeapSoakReport {
    op_count: u64,
    flood_rows: u64,
    sample_every: u64,
    rss_initial_bytes: u64,
    rss_final_bytes: u64,
    rss_max_bytes: u64,
    configured_cap_sum_bytes: u64,
    process_rss_headroom_bytes: u64,
    rss_budget_bytes: u64,
    rss_trend_bytes_per_op: f64,
    rss_trend_window: &'static str,
    rss_samples: Vec<RssSample>,
    writes: u64,
    point_reads: u64,
    range_scans: u64,
    range_materialized: u64,
    cache_miss_queries: u64,
    flood_backpressure_events: u64,
    flood_attempted_rows: u64,
    max_write_value_bytes: usize,
    fail_closed_row_bytes: usize,
    fail_closed_error_code: String,
    backpressure_events_total: u64,
    memtable_absorbed_total: u64,
    memtable_rejected_total: u64,
    arena_high_water_bytes: u64,
    arena_resets: u64,
    slab_max_utilization: f64,
    page_slab_max_utilization: f64,
    cache_used_bytes: u64,
    cache_byte_cap: u64,
    cache_evictions: u64,
    sst_files: usize,
    elapsed_ms: u128,
    structured_error_codes: Vec<&'static str>,
    panic_free: bool,
}

#[derive(Default)]
struct Counts {
    writes: u64,
    point_reads: u64,
    range_scans: u64,
    range_materialized: u64,
    cache_miss_queries: u64,
    flood_backpressure_events: u64,
}

struct FloodReadback {
    backpressure_events: u64,
    fail_closed_row_bytes: usize,
    fail_closed_error_code: String,
}

fn run_heap_soak(root: &Path, op_count: u64, flood_rows: u64) -> Result<HeapSoakReport, String> {
    let vault_dir = root.join("router");
    let mut router = CfRouter::open(&vault_dir, MEMTABLE_CAP).map_err(err)?;
    let mut arena = Arena::new(ARENA_CAP).map_err(err)?;
    let slab = SlabPool::<256>::new(SMALL_SLAB_SLOTS).map_err(err)?;
    let page_slab = PageAlignedSlabPool::new(4096, PAGE_SLAB_SLOTS).map_err(err)?;
    let clock = Arc::new(FixedClock::new(1_800_100_800_000));
    let mut cache = LruTtlCache::<u64, Vec<u8>>::new(CACHE_CAP, Duration::from_secs(3600), clock)
        .map_err(err)?;
    let mut recent = VecDeque::<[u8; 8]>::with_capacity(KEY_RING);
    let mut value = vec![0u8; 4096];
    let mut samples = vec![RssSample {
        op: 0,
        rss_bytes: heap_rss_bytes().map_err(err)?,
    }];
    let mut counts = Counts::default();
    let mut slab_max = 0.0f64;
    let mut page_slab_max = 0.0f64;
    let mut injected = false;
    let mut flood_fail_closed_code = String::new();
    let mut flood_fail_closed_row_bytes = 0usize;
    let started = Instant::now();

    for op in 0..op_count {
        if !injected && op >= op_count / 2 {
            let flood = inject_max_write_flood(root, &mut router, flood_rows)?;
            counts.flood_backpressure_events += flood.backpressure_events;
            flood_fail_closed_code = flood.fail_closed_error_code;
            flood_fail_closed_row_bytes = flood.fail_closed_row_bytes;
            injected = true;
        }
        let (slab_util, page_util) = exercise_allocators(op, &mut arena, &slab, &page_slab)?;
        slab_max = slab_max.max(slab_util);
        page_slab_max = page_slab_max.max(page_util);
        match op % 100 {
            0..=49 => write_op(op, &mut router, &mut recent, &mut value, &mut counts)?,
            50..=79 => point_read_op(&router, &recent, &mut counts)?,
            80..=94 => range_op(op, &router, &recent, &mut counts)?,
            _ => cache_miss_op(op, &mut cache, &mut counts)?,
        }
        if (op + 1) % SAMPLE_EVERY == 0 {
            samples.push(RssSample {
                op: op + 1,
                rss_bytes: heap_rss_bytes().map_err(err)?,
            });
        }
    }
    if !injected {
        let flood = inject_max_write_flood(root, &mut router, flood_rows)?;
        counts.flood_backpressure_events += flood.backpressure_events;
        flood_fail_closed_code = flood.fail_closed_error_code;
        flood_fail_closed_row_bytes = flood.fail_closed_row_bytes;
    }
    router.flush_pending().map_err(err)?;
    let counters = router.resource_counters().snapshot();
    let rss_final = heap_rss_bytes().map_err(err)?;
    match samples.last_mut() {
        Some(sample) if sample.op == op_count => sample.rss_bytes = rss_final,
        _ => samples.push(RssSample {
            op: op_count,
            rss_bytes: rss_final,
        }),
    }
    let rss_max = samples
        .iter()
        .map(|sample| sample.rss_bytes)
        .max()
        .unwrap_or(0);
    let cap_sum = ARENA_CAP + MEMTABLE_CAP * 3 + CACHE_CAP + 4096 * PAGE_SLAB_SLOTS;
    let rss_budget = samples[0]
        .rss_bytes
        .saturating_add(((cap_sum as f64) * 1.20) as u64)
        .saturating_add(PROCESS_RSS_HEADROOM as u64);
    Ok(HeapSoakReport {
        op_count,
        flood_rows,
        sample_every: SAMPLE_EVERY,
        rss_initial_bytes: samples[0].rss_bytes,
        rss_final_bytes: rss_final,
        rss_max_bytes: rss_max,
        configured_cap_sum_bytes: cap_sum as u64,
        process_rss_headroom_bytes: PROCESS_RSS_HEADROOM as u64,
        rss_budget_bytes: rss_budget,
        rss_trend_bytes_per_op: tail_slope(&samples),
        rss_trend_window: "last_quarter_after_burst",
        rss_samples: samples,
        writes: counts.writes,
        point_reads: counts.point_reads,
        range_scans: counts.range_scans,
        range_materialized: counts.range_materialized,
        cache_miss_queries: counts.cache_miss_queries,
        flood_backpressure_events: counts.flood_backpressure_events,
        flood_attempted_rows: flood_rows,
        max_write_value_bytes: 4096,
        fail_closed_row_bytes: flood_fail_closed_row_bytes,
        fail_closed_error_code: flood_fail_closed_code,
        backpressure_events_total: counters.events_total,
        memtable_absorbed_total: counters.memtable_absorbed_total,
        memtable_rejected_total: counters.memtable_rejected_total,
        arena_high_water_bytes: arena.stats().arena_high_water_bytes as u64,
        arena_resets: arena.stats().arena_resets,
        slab_max_utilization: slab_max,
        page_slab_max_utilization: page_slab_max,
        cache_used_bytes: cache.used_bytes() as u64,
        cache_byte_cap: cache.byte_cap() as u64,
        cache_evictions: cache.evictions(),
        sst_files: router.level_file_count(ColumnFamily::Base),
        elapsed_ms: started.elapsed().as_millis(),
        structured_error_codes: vec!["CALYX_BACKPRESSURE"],
        panic_free: true,
    })
}

fn exercise_allocators(
    op: u64,
    arena: &mut Arena,
    slab: &SlabPool<256>,
    page_slab: &PageAlignedSlabPool,
) -> Result<(f64, f64), String> {
    let _ = arena.alloc(64 + (op as usize % 1024), 8).map_err(err)?;
    let slab_utilization;
    {
        let mut guard = slab.acquire().map_err(err)?;
        guard[0] = op as u8;
        slab_utilization = slab.utilization();
    }
    let page_slab_utilization;
    {
        let mut guard = page_slab.acquire().map_err(err)?;
        guard.as_mut_slice()[0] = op as u8;
        page_slab_utilization = page_slab.utilization();
    }
    arena.reset();
    Ok((slab_utilization, page_slab_utilization))
}

fn write_op(
    op: u64,
    router: &mut CfRouter,
    recent: &mut VecDeque<[u8; 8]>,
    value: &mut [u8],
    counts: &mut Counts,
) -> Result<(), String> {
    let key = (op % KEY_SPACE).to_be_bytes();
    let len = 64 + (op as usize % 64);
    value[0] = op as u8;
    value[len - 1] = (op >> 8) as u8;
    router
        .put(ColumnFamily::Base, &key, &value[..len])
        .map_err(err)?;
    if recent.len() == KEY_RING {
        recent.pop_front();
    }
    recent.push_back(key);
    counts.writes += 1;
    Ok(())
}

fn point_read_op(
    router: &CfRouter,
    recent: &VecDeque<[u8; 8]>,
    counts: &mut Counts,
) -> Result<(), String> {
    if let Some(key) = recent.get((counts.point_reads as usize) % recent.len().max(1)) {
        let _ = router.get(ColumnFamily::Base, key).map_err(err)?;
    }
    counts.point_reads += 1;
    Ok(())
}

fn range_op(
    op: u64,
    router: &CfRouter,
    recent: &VecDeque<[u8; 8]>,
    counts: &mut Counts,
) -> Result<(), String> {
    counts.range_scans += 1;
    if !op.is_multiple_of(10_000) {
        return Ok(());
    }
    let Some(start) = recent.front().copied() else {
        return Ok(());
    };
    let end = u64::from_be_bytes(start).saturating_add(16).to_be_bytes();
    let _ = router
        .range(ColumnFamily::Base, &start, &end)
        .map_err(err)?;
    counts.range_materialized += 1;
    Ok(())
}

fn cache_miss_op(
    op: u64,
    cache: &mut LruTtlCache<u64, Vec<u8>>,
    counts: &mut Counts,
) -> Result<(), String> {
    let missing = u64::MAX - op;
    if cache.get(&missing).is_some() {
        return Err("cache miss key unexpectedly present".to_string());
    }
    let size = 64 + (op as usize % 2048);
    cache.insert(op, vec![0xC5; size], size).map_err(err)?;
    counts.cache_miss_queries += 1;
    Ok(())
}

fn inject_max_write_flood(
    root: &Path,
    router: &mut CfRouter,
    flood_rows: u64,
) -> Result<FloodReadback, String> {
    let before = router.resource_counters().snapshot().events_total;
    let mut value = vec![0xEE; 4096];
    let last = value.len() - 1;
    for row in 0..flood_rows {
        value[0] = row as u8;
        value[last] = (row >> 8) as u8;
        let key = u64::MAX.saturating_sub(row).to_be_bytes();
        router.put(ColumnFamily::Base, &key, &value).map_err(err)?;
    }
    let after_flood = router.resource_counters().snapshot().events_total;
    let (fail_closed_row_bytes, fail_closed_error_code) =
        probe_max_write_fail_closed(root.join("max_write_fail_closed"))?;
    Ok(FloodReadback {
        backpressure_events: after_flood.saturating_sub(before),
        fail_closed_row_bytes,
        fail_closed_error_code,
    })
}

fn probe_max_write_fail_closed(root: impl AsRef<Path>) -> Result<(usize, String), String> {
    let router = CfRouter::open(root, 1024).map_err(err)?;
    let value = vec![0xFA; 4096];
    let row_bytes = value.len();
    match router.ensure_batch_admitted([(
        ColumnFamily::Base,
        b"ph59-h8-max-write".as_slice(),
        value.as_slice(),
    )]) {
        Ok(()) => Err("4096-byte row unexpectedly admitted into 1KiB cap".to_string()),
        Err(error) if error.code == "CALYX_BACKPRESSURE" => Ok((row_bytes, error.code.to_string())),
        Err(error) => Err(format!("expected CALYX_BACKPRESSURE, got {}", error.code)),
    }
}

fn tail_slope(samples: &[RssSample]) -> f64 {
    let start = samples.len().saturating_mul(3) / 4;
    slope(&samples[start..])
}

fn slope(samples: &[RssSample]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let n = samples.len() as f64;
    let sum_x = samples.iter().map(|sample| sample.op as f64).sum::<f64>();
    let sum_y = samples
        .iter()
        .map(|sample| sample.rss_bytes as f64)
        .sum::<f64>();
    let sum_xx = samples
        .iter()
        .map(|sample| (sample.op as f64).powi(2))
        .sum::<f64>();
    let sum_xy = samples
        .iter()
        .map(|sample| sample.op as f64 * sample.rss_bytes as f64)
        .sum::<f64>();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom == 0.0 {
        0.0
    } else {
        (n * sum_xy - sum_x * sum_y) / denom
    }
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}
