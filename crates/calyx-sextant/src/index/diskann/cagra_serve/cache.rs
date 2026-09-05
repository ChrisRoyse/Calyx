use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Instant, UNIX_EPOCH};

use calyx_core::Result;

use super::cache_config::{env_u64, env_usize};
use super::cuda::Asset as CagraAsset;
use super::partition_asset::Asset as PartitionAsset;
use super::partitioned;
use super::telemetry::{TELEMETRY, elapsed_us, telemetry_snapshot};
use super::{
    CAGRA_PARTITIONED_MAX_SCRATCH_BYTES, CagraPartitionSearchRequest, CagraSearchRequest,
    CagraServingDiagnostics, CagraServingRegion,
};
use crate::error::{
    CALYX_SEXTANT_GPU_CACHE_EXHAUSTED, CALYX_SEXTANT_GPU_SERVING_UNAVAILABLE, sextant_error,
};
use crate::index::diskann::cagra_sidecar_path;

const CACHE_MIB_ENV: &str = "CALYX_SEXTANT_CAGRA_CACHE_MIB";
const CACHE_ENTRIES_ENV: &str = "CALYX_SEXTANT_CAGRA_CACHE_ENTRIES";

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Key {
    path: PathBuf,
    len: u64,
    modified_ns: u128,
    dataset_digest: Option<[u8; 32]>,
    global_ids_digest: Option<[u8; 32]>,
}

type Signal = Arc<(Mutex<bool>, Condvar)>;

enum Entry {
    Loading {
        reserved: u64,
        tick: u64,
        signal: Signal,
    },
    Ready {
        asset: Arc<Mutex<CachedAsset>>,
        bytes: u64,
        tick: u64,
    },
}

enum CachedAsset {
    Cagra(CagraAsset),
    Partition(PartitionAsset),
}

#[derive(Clone, Copy)]
enum LoadKind {
    Cagra,
    Partition,
}

impl CachedAsset {
    fn resident_bytes(&self) -> u64 {
        match self {
            Self::Cagra(asset) => asset.resident_bytes(),
            Self::Partition(asset) => asset.resident_bytes(),
        }
    }
}

struct Cache {
    entries: HashMap<Key, Entry>,
    generations: HashMap<PathBuf, Key>,
    lru: BTreeSet<(u64, Key)>,
    max_bytes: u64,
    max_entries: usize,
    resident_bytes: u64,
    tick: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    invalidations: u64,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

pub(super) fn search(request: CagraSearchRequest<'_>) -> Result<Vec<Vec<(u32, f32)>>> {
    let sidecar = cagra_sidecar_path(request.graph_path);
    let (key, reserved) = key(&sidecar)?;
    let asset = acquire(key.clone(), reserved, LoadKind::Cagra, None)?;
    let mut cached = asset
        .lock()
        .map_err(|_| unavailable("CAGRA cache asset lock poisoned"))?;
    let CachedAsset::Cagra(asset) = &mut *cached else {
        return Err(unavailable("CAGRA cache representation mismatch"));
    };
    let pairs = request.query_count.saturating_mul(request.k);
    let required =
        asset.projected_search_bytes(request.queries.len(), pairs, request.allowed_ids.is_some());
    reserve_asset_growth(&key, required)?;
    asset.search(
        request.metric,
        request.queries,
        request.query_count,
        request.k,
        request.ef_search,
        request.allowed_ids,
    )
}

pub(super) fn search_partitioned(
    request: CagraPartitionSearchRequest<'_>,
) -> Result<Vec<(u64, f32)>> {
    let prepare_started = Instant::now();
    let mut keepalive = Vec::with_capacity(request.regions.len());
    let mut views = Vec::with_capacity(request.regions.len());
    for region in request.regions {
        if region.serving.metric != request.metric {
            return Err(unavailable("partitioned CUDA dataset metric mismatch"));
        }
        let (key, reserved) = region_key(region.serving);
        let asset = acquire(
            key.clone(),
            reserved,
            LoadKind::Partition,
            Some((region.global_ids, region.serving.global_ids_digest)),
        )?;
        let mut cached = asset
            .lock()
            .map_err(|_| unavailable("CAGRA cache asset lock poisoned"))?;
        let CachedAsset::Partition(locked) = &mut *cached else {
            return Err(unavailable(
                "partitioned CUDA cache representation mismatch",
            ));
        };
        if let Some(required) =
            locked.global_ids_required(region.global_ids, region.serving.global_ids_digest)?
        {
            reserve_asset_growth(&key, required)?;
        }
        views.push(locked.region(region.global_ids, region.serving.global_ids_digest)?);
        drop(cached);
        keepalive.push(asset);
    }
    TELEMETRY.partitioned_prepare_us.fetch_add(
        elapsed_us(prepare_started),
        std::sync::atomic::Ordering::Relaxed,
    );
    let execute_started = Instant::now();
    let result = partitioned::search(&views, request.query, request.metric, request.k);
    TELEMETRY.partitioned_execute_us.fetch_add(
        elapsed_us(execute_started),
        std::sync::atomic::Ordering::Relaxed,
    );
    drop(keepalive);
    result
}

pub(super) fn diagnostics() -> CagraServingDiagnostics {
    let telemetry = telemetry_snapshot();
    let cache = cache().lock().expect("CAGRA cache lock poisoned");
    let [pool_reserved, pool_reserved_max, pool_used, pool_used_max] =
        PartitionAsset::pool_diagnostics();
    CagraServingDiagnostics {
        backend: "cuda-dense-generation-cache-v2",
        cache_entries: cache.entries.len(),
        cache_max_entries: cache.max_entries,
        resident_bytes: cache.resident_bytes,
        resident_max_bytes: cache.max_bytes,
        cache_hits: cache.hits,
        cache_misses: cache.misses,
        cache_evictions: cache.evictions,
        cache_invalidations: cache.invalidations,
        batches: telemetry.batches,
        queries: telemetry.queries,
        cagra_kernel_launches: telemetry.cagra_kernel_launches,
        exact_filter_kernel_launches: telemetry.exact_filter_kernel_launches,
        partitioned_exact_kernel_launches: telemetry.partitioned_exact_kernel_launches,
        partitioned_merge_kernel_launches: telemetry.partitioned_merge_kernel_launches,
        partitioned_prepare_us: telemetry.partitioned_prepare_us,
        partitioned_execute_us: telemetry.partitioned_execute_us,
        partitioned_scratch_bytes: telemetry.partitioned_scratch_bytes,
        partitioned_scratch_max_bytes: CAGRA_PARTITIONED_MAX_SCRATCH_BYTES,
        partitioned_i8_dataset_loads: telemetry.partitioned_i8_dataset_loads,
        partitioned_f32_dataset_loads: telemetry.partitioned_f32_dataset_loads,
        partitioned_pool_reserved_bytes: pool_reserved,
        partitioned_pool_reserved_max_bytes: pool_reserved_max,
        partitioned_pool_used_bytes: pool_used,
        partitioned_pool_used_max_bytes: pool_used_max,
        query_uploads: telemetry.query_uploads,
        filter_uploads: telemetry.filter_uploads,
        h2d_bytes: telemetry.h2d_bytes,
        d2h_bytes: telemetry.d2h_bytes,
        final_readback_pairs: telemetry.final_readback_pairs,
        intermediate_readback_pairs: 0,
        failures: telemetry.failures,
    }
}

pub(super) fn configure(max_bytes: u64, max_entries: usize) -> Result<()> {
    if max_bytes == 0 || max_entries == 0 {
        return Err(exhausted(format!(
            "CAGRA cache configuration requires non-zero bytes and entries; received bytes={max_bytes} entries={max_entries}"
        )));
    }
    let mut guard = cache()
        .lock()
        .map_err(|_| unavailable("CAGRA cache lock poisoned"))?;
    while guard.resident_bytes > max_bytes || guard.entries.len() > max_entries {
        let victim = guard.lru.iter().find_map(|(_, key)| {
            guard
                .entries
                .get(key)
                .is_some_and(evictable)
                .then(|| key.clone())
        });
        let Some(victim) = victim else {
            return Err(exhausted(format!(
                "CAGRA cache cannot reconfigure to bytes={max_bytes} entries={max_entries}: resident={} entries={}; active entries are still in use",
                guard.resident_bytes,
                guard.entries.len()
            )));
        };
        if let Some(entry) = take_entry(&mut guard, &victim) {
            guard.resident_bytes = guard.resident_bytes.saturating_sub(entry_bytes(&entry));
            guard.evictions += 1;
            drop(entry);
            PartitionAsset::reclaim_unused()?;
        }
    }
    guard.max_bytes = max_bytes;
    guard.max_entries = max_entries;
    Ok(())
}

fn acquire(
    key: Key,
    reserved: u64,
    kind: LoadKind,
    partition_init: Option<(&[u64], [u8; 32])>,
) -> Result<Arc<Mutex<CachedAsset>>> {
    loop {
        let mut guard = cache()
            .lock()
            .map_err(|_| unavailable("CAGRA cache lock poisoned"))?;
        guard.tick = guard.tick.wrapping_add(1);
        let tick = guard.tick;
        let ready = if let Some(entry) = guard.entries.get_mut(&key) {
            match entry {
                Entry::Ready {
                    asset, tick: last, ..
                } => {
                    let previous = *last;
                    *last = tick;
                    Some((asset.clone(), previous))
                }
                Entry::Loading { signal, .. } => {
                    let signal = signal.clone();
                    drop(guard);
                    wait_for_load(&signal)?;
                    continue;
                }
            }
        } else {
            None
        };
        if let Some((asset, previous)) = ready {
            guard.lru.remove(&(previous, key.clone()));
            guard.lru.insert((tick, key));
            guard.hits += 1;
            return Ok(asset);
        }
        invalidate_older_generation(&mut guard, &key)?;

        reserve(&mut guard, reserved, true)?;
        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        guard.entries.insert(
            key.clone(),
            Entry::Loading {
                reserved,
                tick,
                signal: signal.clone(),
            },
        );
        guard.lru.insert((tick, key.clone()));
        guard.resident_bytes = guard.resident_bytes.saturating_add(reserved);
        guard.misses += 1;
        drop(guard);

        let loaded = match kind {
            LoadKind::Cagra => CagraAsset::load(&key.path).map(CachedAsset::Cagra),
            LoadKind::Partition => match partition_init {
                Some((global_ids, digest)) => {
                    PartitionAsset::load(&key.path, global_ids, digest).map(CachedAsset::Partition)
                }
                None => Err(unavailable("partitioned cache load missing global IDs")),
            },
        };
        let mut guard = cache()
            .lock()
            .map_err(|_| unavailable("CAGRA cache lock poisoned"))?;
        match loaded {
            Ok(asset) => {
                let actual = asset.resident_bytes();
                if actual > reserved && reserve(&mut guard, actual - reserved, false).is_err() {
                    take_entry(&mut guard, &key);
                    guard.resident_bytes = guard.resident_bytes.saturating_sub(reserved);
                    notify(&signal);
                    return Err(exhausted(format!(
                        "loaded CAGRA asset {} needs {actual} bytes, cache cap is {}",
                        key.path.display(),
                        guard.max_bytes
                    )));
                }
                guard.resident_bytes = guard
                    .resident_bytes
                    .saturating_add(actual.saturating_sub(reserved));
                guard.resident_bytes = guard
                    .resident_bytes
                    .saturating_sub(reserved.saturating_sub(actual));
                let asset = Arc::new(Mutex::new(asset));
                guard.entries.insert(
                    key,
                    Entry::Ready {
                        asset: asset.clone(),
                        bytes: actual,
                        tick,
                    },
                );
                notify(&signal);
                return Ok(asset);
            }
            Err(error) => {
                take_entry(&mut guard, &key);
                guard.resident_bytes = guard.resident_bytes.saturating_sub(reserved);
                notify(&signal);
                return Err(error);
            }
        }
    }
}

fn key(sidecar: &Path) -> Result<(Key, u64)> {
    let canonical = sidecar.canonicalize().map_err(|error| {
        unavailable(format!(
            "required CAGRA serving asset {} is unavailable: {error}",
            sidecar.display()
        ))
    })?;
    let metadata = canonical.metadata().map_err(|error| {
        unavailable(format!("stat CAGRA asset {}: {error}", canonical.display()))
    })?;
    if metadata.len() == 0 {
        return Err(unavailable(format!(
            "required CAGRA serving asset {} is empty",
            canonical.display()
        )));
    }
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    let len = metadata.len();
    Ok((
        Key {
            path: canonical,
            len,
            modified_ns,
            dataset_digest: None,
            global_ids_digest: None,
        },
        len,
    ))
}

fn region_key(region: &CagraServingRegion) -> (Key, u64) {
    let asset = &region.asset;
    (
        Key {
            path: asset.path.clone(),
            len: asset.len,
            modified_ns: asset.modified_ns,
            dataset_digest: Some(asset.generation_digest),
            global_ids_digest: Some(region.global_ids_digest),
        },
        asset.len,
    )
}

fn invalidate_older_generation(cache: &mut Cache, key: &Key) -> Result<()> {
    let stale = cache.generations.insert(key.path.clone(), key.clone());
    if let Some(stale) = stale
        && stale != *key
        && cache.entries.get(&stale).is_some_and(evictable)
        && let Some(entry) = take_entry(cache, &stale)
    {
        cache.resident_bytes = cache.resident_bytes.saturating_sub(entry_bytes(&entry));
        cache.invalidations += 1;
        drop(entry);
        PartitionAsset::reclaim_unused()?;
    }
    Ok(())
}

fn take_entry(cache: &mut Cache, key: &Key) -> Option<Entry> {
    let entry = cache.entries.remove(key)?;
    cache.lru.remove(&(entry_tick(&entry), key.clone()));
    if cache.generations.get(&key.path) == Some(key) {
        cache.generations.remove(&key.path);
    }
    Some(entry)
}

fn reserve(cache: &mut Cache, additional: u64, needs_entry: bool) -> Result<()> {
    while cache.resident_bytes.saturating_add(additional) > cache.max_bytes
        || (needs_entry && cache.entries.len() >= cache.max_entries)
    {
        let victim = cache.lru.iter().find_map(|(_, key)| {
            cache
                .entries
                .get(key)
                .is_some_and(evictable)
                .then(|| key.clone())
        });
        let Some(victim) = victim else {
            return Err(exhausted(format!(
                "CAGRA cache cannot reserve {additional} bytes: resident={} cap={} entries={}/{}; all entries are in use",
                cache.resident_bytes,
                cache.max_bytes,
                cache.entries.len(),
                cache.max_entries
            )));
        };
        if let Some(entry) = take_entry(cache, &victim) {
            cache.resident_bytes = cache.resident_bytes.saturating_sub(entry_bytes(&entry));
            cache.evictions += 1;
            drop(entry);
            PartitionAsset::reclaim_unused()?;
        }
    }
    Ok(())
}

fn reserve_asset_growth(key: &Key, required: u64) -> Result<()> {
    let mut guard = cache()
        .lock()
        .map_err(|_| unavailable("CAGRA cache lock poisoned"))?;
    let current = match guard.entries.get(key) {
        Some(Entry::Ready { bytes, .. }) => *bytes,
        _ => return Err(unavailable("CAGRA cache entry disappeared while in use")),
    };
    let additional = required.saturating_sub(current);
    if additional == 0 {
        return Ok(());
    }
    reserve(&mut guard, additional, false)?;
    guard.resident_bytes = guard.resident_bytes.saturating_add(additional);
    if let Some(Entry::Ready { bytes, .. }) = guard.entries.get_mut(key) {
        *bytes = required;
        Ok(())
    } else {
        Err(unavailable("CAGRA cache entry disappeared during growth"))
    }
}

fn evictable(entry: &Entry) -> bool {
    matches!(entry, Entry::Ready { asset, .. } if Arc::strong_count(asset) == 1)
}

fn entry_bytes(entry: &Entry) -> u64 {
    match entry {
        Entry::Loading { reserved, .. } => *reserved,
        Entry::Ready { bytes, .. } => *bytes,
    }
}

fn entry_tick(entry: &Entry) -> u64 {
    match entry {
        Entry::Loading { tick, .. } | Entry::Ready { tick, .. } => *tick,
    }
}

fn wait_for_load(signal: &Signal) -> Result<()> {
    let (lock, condvar) = signal.as_ref();
    let mut finished = lock
        .lock()
        .map_err(|_| unavailable("CAGRA cache load signal poisoned"))?;
    while !*finished {
        finished = condvar
            .wait(finished)
            .map_err(|_| unavailable("CAGRA cache load wait poisoned"))?;
    }
    Ok(())
}

fn notify(signal: &Signal) {
    let (lock, condvar) = signal.as_ref();
    if let Ok(mut finished) = lock.lock() {
        *finished = true;
        condvar.notify_all();
    }
}

/// Cache budget to use when the operator has not set [`CACHE_MIB_ENV`].
///
/// The old default was a flat 8 GiB, which is not a budget at all -- it is
/// larger than the free memory on a shared card, so the cache believed it had
/// room and kept pinning partitions until cuVS raised a bare
/// `out_of_memory` (#2058: 3/3 query failures at 2,963 MiB free, 3/3 successes
/// at the same free memory once the budget was bounded to 1,536 MiB).
///
/// Size against what the device actually has free instead. Half of free memory
/// leaves room for the driver context, the query workspace, and any co-tenant
/// that grows while we are serving. Undershooting is cheap -- the cache evicts
/// and reloads, costing latency -- whereas overshooting is a hard query
/// failure, so the asymmetry is deliberate.
fn default_cache_bytes() -> u64 {
    const FLOOR_MIB: u64 = 512;
    const CEILING_MIB: u64 = 8 * 1024;
    let mib = match calyx_forge::vram::host_reservation::device_free_mib(0) {
        Ok(free_mib) => (free_mib / 2).clamp(FLOOR_MIB, CEILING_MIB),
        // NVML is unreadable, so the honest budget is the smallest one that can
        // still serve. Never widen to the ceiling here: an unknown device is
        // precisely the case where a large budget cannot be justified.
        Err(error) => {
            eprintln!(
                "CALYX_SEXTANT_CAGRA_CACHE phase=device_free_unreadable floor_mib={FLOOR_MIB} detail={}",
                error
            );
            FLOOR_MIB
        }
    };
    mib.saturating_mul(1024 * 1024)
}

fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| {
        // env_u64 already rejects zero and unparsable values, so a sentinel of 0
        // means "no usable override" and the device-derived budget applies.
        let override_mib = env_u64(CACHE_MIB_ENV, 0);
        let (max_bytes, source) = if override_mib > 0 {
            (override_mib.saturating_mul(1024 * 1024), CACHE_MIB_ENV)
        } else {
            (default_cache_bytes(), "device_free_half")
        };
        eprintln!(
            "CALYX_SEXTANT_CAGRA_CACHE phase=configured max_bytes={max_bytes} source={source}"
        );
        Mutex::new(Cache {
            entries: HashMap::new(),
            generations: HashMap::new(),
            lru: BTreeSet::new(),
            max_bytes,
            max_entries: env_usize(CACHE_ENTRIES_ENV, 32_768).max(1),
            resident_bytes: 0,
            tick: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            invalidations: 0,
        })
    })
}

fn unavailable(detail: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_SEXTANT_GPU_SERVING_UNAVAILABLE, detail)
}

fn exhausted(detail: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_SEXTANT_GPU_CACHE_EXHAUSTED, detail)
}
