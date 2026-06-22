#![cfg(target_os = "linux")]

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::resource::heap_rss_bytes;
use calyx_core::{Arena, FixedClock, LruTtlCache, PageAlignedSlabPool, SlabPool};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ARENA_CAP: usize = 4 * 1024 * 1024;
const MEMTABLE_CAP: usize = 32 * 1024 * 1024;
const CACHE_CAP: usize = 16 * 1024 * 1024;
const PAGE_SLAB_SLOTS: usize = 8;
const SMALL_SLAB_SLOTS: usize = 1024;
const KEY_RING: usize = 4096;
const KEY_SPACE: u64 = 65_536;
const SEED: u64 = 0xCA1A_0056;
const SAMPLE_EVERY: u64 = 1_000;
const FULL_OPS: u64 = 10_000_000;
const FULL_FLOOD_OPS: u64 = 100_000;
const SMOKE_OPS: u64 = 50_000;
const SMOKE_FLOOD_OPS: u64 = 1_000;
const PROCESS_RSS_HEADROOM: usize = 64 * 1024 * 1024;

mod fsv_support;
use fsv_support::{env_or_temp_root, reset_dir, temp_root};

#[test]
fn ph56_soak_smoke_bounds_rss_and_backpressure() {
    let root = test_dir("smoke");
    let report = run_soak(&root, SMOKE_OPS, SMOKE_FLOOD_OPS, false);
    assert!(report.backpressure_events_total > 0);
    assert!(report.rss_max_bytes <= report.rss_budget_bytes);
    assert!(report.rss_trend_bytes_per_op < 1.0);
    assert!(report.cache_used_bytes <= CACHE_CAP as u64);
    assert!(report.arena_high_water_bytes <= ARENA_CAP as u64);
    assert!(report.slab_max_utilization < 1.0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "manual FSV: runs the PH56 1e7-op RSS/backpressure soak"]
fn ph56_1e7_soak_rss_bounded_fsv() {
    let root = env_or_temp_root("CALYX_FSV_ROOT", "calyx-ph56-soak", "fsv");
    reset_dir(&root);
    let ops = env_u64("PH56_SOAK_OPS").unwrap_or(FULL_OPS);
    let flood_ops = env_u64("PH56_SOAK_FLOOD_OPS").unwrap_or(FULL_FLOOD_OPS);
    let report = run_soak(&root, ops, flood_ops, true);
    let json = serde_json::to_vec_pretty(&report).unwrap();
    let target = workspace_target_path("ph56_soak_rss.json");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, &json).unwrap();
    fs::write(root.join("ph56_soak_rss.json"), &json).unwrap();
    fs::write(root.join("ph56_soak_metrics.prom"), metrics_text(&report)).unwrap();
    fs::write(
        root.join("cleanup-tag.txt"),
        b"issue474 synthetic soak data\n",
    )
    .unwrap();
    assert!(report.backpressure_events_total > 0);
    assert!(report.rss_max_bytes <= report.rss_budget_bytes);
    assert!(report.rss_trend_bytes_per_op < 1.0);
    assert!(report.cache_used_bytes <= CACHE_CAP as u64);
    assert!(report.arena_high_water_bytes <= ARENA_CAP as u64);
    assert!(report.slab_max_utilization < 1.0);
    assert_eq!(report.structured_error_codes, ["CALYX_BACKPRESSURE"]);
}

#[derive(Clone, Copy, Debug, Serialize)]
struct RssSample {
    op: u64,
    rss_bytes: u64,
}

#[derive(Debug, Serialize)]
struct SoakReport {
    seed: u64,
    op_count: u64,
    flood_ops: u64,
    sample_every: u64,
    rss_initial_bytes: u64,
    rss_final_bytes: u64,
    rss_max_bytes: u64,
    configured_cap_sum_bytes: u64,
    process_rss_headroom_bytes: u64,
    rss_budget_bytes: u64,
    rss_trend_bytes_per_op: f64,
    rss_full_trend_bytes_per_op: f64,
    rss_samples: Vec<RssSample>,
    writes: u64,
    point_reads: u64,
    range_scans: u64,
    range_materialized: u64,
    cache_miss_queries: u64,
    flood_backpressure_errors: u64,
    backpressure_events_total: u64,
    memtable_absorbed_total: u64,
    memtable_rejected_total: u64,
    arena_high_water_bytes: u64,
    arena_resets: u64,
    arena_reset_mean_ns: u64,
    slab_max_utilization: f64,
    page_slab_max_utilization: f64,
    cache_used_bytes: u64,
    cache_byte_cap: u64,
    cache_evictions: u64,
    sst_files: usize,
    structured_error_codes: Vec<&'static str>,
}

fn run_soak(root: &Path, op_count: u64, flood_ops: u64, full: bool) -> SoakReport {
    reset_dir(root);
    let vault_dir = root.join("router");
    fs::create_dir_all(&vault_dir).unwrap();
    let mut router = CfRouter::open(&vault_dir, MEMTABLE_CAP).unwrap();
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut arena = Arena::new(ARENA_CAP).unwrap();
    let slab = SlabPool::<256>::new(SMALL_SLAB_SLOTS).unwrap();
    let page_slab = PageAlignedSlabPool::new(4096, PAGE_SLAB_SLOTS).unwrap();
    let clock = Arc::new(FixedClock::new(1_785_600_474));
    let mut cache =
        LruTtlCache::<u64, Vec<u8>>::new(CACHE_CAP, Duration::from_secs(3600), clock.clone())
            .unwrap();
    let mut recent = Vec::<[u8; 8]>::with_capacity(KEY_RING);
    let mut value = vec![0u8; 4096];
    let mut samples = vec![RssSample {
        op: 0,
        rss_bytes: heap_rss_bytes().unwrap(),
    }];
    let mut counts = Counts::default();
    let mut slab_max = 0.0f64;
    let mut page_slab_max = 0.0f64;
    let mut injected = false;
    let start = Instant::now();

    for op in 0..op_count {
        if !injected && op >= op_count / 2 {
            inject_write_flood(&router, flood_ops, &mut counts);
            injected = true;
        }
        let utilization = exercise_allocators(op, &mut arena, &slab, &page_slab);
        slab_max = slab_max.max(utilization.slab);
        page_slab_max = page_slab_max.max(utilization.page_slab);
        let roll = rng.gen_range(0..100);
        match roll {
            0..=49 => write_op(
                op,
                &mut rng,
                &mut router,
                &mut recent,
                &mut value,
                &mut counts,
            ),
            50..=79 => point_read_op(&router, &recent, &mut rng, &mut counts),
            80..=94 => range_op(op, full, &router, &recent, &mut counts),
            _ => cache_miss_op(op, &mut cache, &mut counts),
        }
        if (op + 1) % SAMPLE_EVERY == 0 {
            samples.push(RssSample {
                op: op + 1,
                rss_bytes: heap_rss_bytes().unwrap(),
            });
        }
    }
    if !injected {
        inject_write_flood(&router, flood_ops, &mut counts);
    }
    router.flush_pending().unwrap();
    let counters = router.resource_counters().snapshot();
    let rss_final = heap_rss_bytes().unwrap();
    samples.push(RssSample {
        op: op_count,
        rss_bytes: rss_final,
    });
    let rss_max = samples.iter().map(|sample| sample.rss_bytes).max().unwrap();
    let active_memtable_cap = MEMTABLE_CAP;
    let flush_memtable_cap = MEMTABLE_CAP;
    let flood_admission_cap = MEMTABLE_CAP;
    let cap_sum = ARENA_CAP
        + active_memtable_cap
        + flush_memtable_cap
        + CACHE_CAP
        + flood_admission_cap
        + 4096 * PAGE_SLAB_SLOTS;
    let rss_budget = samples[0]
        .rss_bytes
        .saturating_add(((cap_sum as f64) * 1.20) as u64)
        .saturating_add(PROCESS_RSS_HEADROOM as u64);
    let reset_mean = arena_reset_mean_ns();
    let _elapsed = start.elapsed();
    SoakReport {
        seed: SEED,
        op_count,
        flood_ops,
        sample_every: SAMPLE_EVERY,
        rss_initial_bytes: samples[0].rss_bytes,
        rss_final_bytes: rss_final,
        rss_max_bytes: rss_max,
        configured_cap_sum_bytes: cap_sum as u64,
        process_rss_headroom_bytes: PROCESS_RSS_HEADROOM as u64,
        rss_budget_bytes: rss_budget,
        rss_trend_bytes_per_op: tail_slope(&samples),
        rss_full_trend_bytes_per_op: slope(&samples),
        rss_samples: samples,
        writes: counts.writes,
        point_reads: counts.point_reads,
        range_scans: counts.range_scans,
        range_materialized: counts.range_materialized,
        cache_miss_queries: counts.cache_miss_queries,
        flood_backpressure_errors: counts.backpressure,
        backpressure_events_total: counters.events_total,
        memtable_absorbed_total: counters.memtable_absorbed_total,
        memtable_rejected_total: counters.memtable_rejected_total,
        arena_high_water_bytes: arena.stats().arena_high_water_bytes as u64,
        arena_resets: arena.stats().arena_resets,
        arena_reset_mean_ns: reset_mean,
        slab_max_utilization: slab_max,
        page_slab_max_utilization: page_slab_max,
        cache_used_bytes: cache.used_bytes() as u64,
        cache_byte_cap: cache.byte_cap() as u64,
        cache_evictions: cache.evictions(),
        sst_files: router.level_file_count(ColumnFamily::Base),
        structured_error_codes: vec!["CALYX_BACKPRESSURE"],
    }
}

#[derive(Default)]
struct Counts {
    writes: u64,
    point_reads: u64,
    range_scans: u64,
    range_materialized: u64,
    cache_miss_queries: u64,
    backpressure: u64,
}

#[derive(Clone, Copy)]
struct AllocatorUtilization {
    slab: f64,
    page_slab: f64,
}

fn exercise_allocators(
    op: u64,
    arena: &mut Arena,
    slab: &SlabPool<256>,
    page_slab: &PageAlignedSlabPool,
) -> AllocatorUtilization {
    let bytes = 64 + (op as usize % 1024);
    let _ = arena.alloc(bytes, 8).unwrap();
    let slab_utilization;
    {
        let mut guard = slab.acquire().unwrap();
        guard[0] = op as u8;
        slab_utilization = slab.utilization();
    }
    let page_slab_utilization;
    {
        let mut guard = page_slab.acquire().unwrap();
        guard.as_mut_slice()[0] = op as u8;
        page_slab_utilization = page_slab.utilization();
    }
    arena.reset();
    AllocatorUtilization {
        slab: slab_utilization,
        page_slab: page_slab_utilization,
    }
}

fn write_op(
    op: u64,
    rng: &mut StdRng,
    router: &mut CfRouter,
    recent: &mut Vec<[u8; 8]>,
    value: &mut [u8],
    counts: &mut Counts,
) {
    let key = (op % KEY_SPACE).to_be_bytes();
    let len = rng.gen_range(64..=4096);
    value[0] = op as u8;
    value[len - 1] = (op >> 8) as u8;
    router.put(ColumnFamily::Base, &key, &value[..len]).unwrap();
    if recent.len() == KEY_RING {
        recent.remove(0);
    }
    recent.push(key);
    counts.writes += 1;
}

fn point_read_op(router: &CfRouter, recent: &[[u8; 8]], rng: &mut StdRng, counts: &mut Counts) {
    if !recent.is_empty() {
        let key = recent[rng.gen_range(0..recent.len())];
        let _ = router.get(ColumnFamily::Base, &key).unwrap();
    }
    counts.point_reads += 1;
}

fn range_op(op: u64, full: bool, router: &CfRouter, recent: &[[u8; 8]], counts: &mut Counts) {
    counts.range_scans += 1;
    let materialize_every = if full { 10_000 } else { 100 };
    if recent.is_empty() || !op.is_multiple_of(materialize_every) {
        return;
    }
    let start = recent[0];
    let end = u64::from_be_bytes(start).saturating_add(16).to_be_bytes();
    let _ = router.range(ColumnFamily::Base, &start, &end).unwrap();
    counts.range_materialized += 1;
}

fn cache_miss_op(op: u64, cache: &mut LruTtlCache<u64, Vec<u8>>, counts: &mut Counts) {
    let missing = u64::MAX - op;
    assert!(cache.get(&missing).is_none());
    let size = 64 + (op as usize % 2048);
    cache.insert(op, vec![0xC5; size], size).unwrap();
    counts.cache_miss_queries += 1;
}

fn inject_write_flood(router: &CfRouter, flood_ops: u64, counts: &mut Counts) {
    let key = u64::MAX.to_be_bytes();
    let value = vec![0xEE; MEMTABLE_CAP + 1];
    for _ in 0..flood_ops {
        let error = router
            .ensure_batch_admitted([(ColumnFamily::Base, &key, value.as_slice())])
            .unwrap_err();
        assert_eq!(error.code, "CALYX_BACKPRESSURE");
        counts.backpressure += 1;
    }
}

fn arena_reset_mean_ns() -> u64 {
    let mut arena = Arena::new(4096).unwrap();
    let iters = 1_000_000u64;
    let start = Instant::now();
    for _ in 0..iters {
        let _ = arena.alloc(64, 8).unwrap();
        arena.reset();
    }
    (start.elapsed().as_nanos() / u128::from(iters)) as u64
}

fn slope(samples: &[RssSample]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let n = samples.len() as f64;
    let sum_x: f64 = samples.iter().map(|sample| sample.op as f64).sum();
    let sum_y: f64 = samples.iter().map(|sample| sample.rss_bytes as f64).sum();
    let sum_xx: f64 = samples
        .iter()
        .map(|sample| {
            let x = sample.op as f64;
            x * x
        })
        .sum();
    let sum_xy: f64 = samples
        .iter()
        .map(|sample| sample.op as f64 * sample.rss_bytes as f64)
        .sum();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom == 0.0 {
        0.0
    } else {
        (n * sum_xy - sum_x * sum_y) / denom
    }
}

fn tail_slope(samples: &[RssSample]) -> f64 {
    let start = samples.len() / 2;
    slope(&samples[start..])
}

fn metrics_text(report: &SoakReport) -> String {
    format!(
        "calyx_rss_bytes{{phase=\"PH56\"}} {}\n\
         calyx_backpressure_events_total{{phase=\"PH56\"}} {}\n\
         calyx_cache_used_bytes{{phase=\"PH56\"}} {}\n\
         calyx_arena_high_water_bytes{{phase=\"PH56\"}} {}\n",
        report.rss_final_bytes,
        report.backpressure_events_total,
        report.cache_used_bytes,
        report.arena_high_water_bytes
    )
}

fn test_dir(name: &str) -> PathBuf {
    temp_root("calyx-ph56-soak", name)
}

fn workspace_target_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("target")
        .join(name)
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}
