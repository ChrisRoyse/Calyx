use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{
    CalyxError, Clock, Input, Lens, LensId, LruTtlCache, Modality, Result as CalyxResult,
    SlotShape, SlotVector, Ts, VaultId,
};
use calyx_registry::frozen::sha256_digest;
use calyx_registry::{FrozenLensContract, IngestPanelReadout, LensDType, NormPolicy};
use serde::Serialize;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::resource_support::{err, key};

#[derive(Clone, Debug)]
pub(super) struct MutableClock {
    now: Arc<AtomicU64>,
}

impl MutableClock {
    pub(super) fn new(now: Ts) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(now)),
        }
    }

    pub(super) fn set(&self, now: Ts) {
        self.now.store(now, Ordering::SeqCst);
    }

    pub(super) fn advance(&self, delta_ms: Ts) {
        self.now.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

impl Clock for MutableClock {
    fn now(&self) -> Ts {
        self.now.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ReadRate {
    pub reads: u64,
    pub hits: u64,
    pub elapsed_ms: u128,
    pub ops_per_sec: f64,
}

pub(super) fn read_rate<C: Clock>(
    vault: &AsterVault<C>,
    prefix: &str,
    start: u64,
    end: u64,
    duration: Duration,
) -> Result<ReadRate, String> {
    if start >= end {
        return Ok(ReadRate {
            reads: 0,
            hits: 0,
            elapsed_ms: 0,
            ops_per_sec: 0.0,
        });
    }
    let snapshot = vault.latest_seq();
    let started = Instant::now();
    let mut reads = 0_u64;
    let mut hits = 0_u64;
    while started.elapsed() < duration {
        for id in start..end {
            if started.elapsed() >= duration {
                break;
            }
            if vault
                .read_cf_at(snapshot, ColumnFamily::Base, &key(prefix, id))
                .map_err(err)?
                .is_some()
            {
                hits += 1;
            }
            reads += 1;
        }
    }
    let elapsed = started.elapsed();
    Ok(ReadRate {
        reads,
        hits,
        elapsed_ms: elapsed.as_millis(),
        ops_per_sec: reads as f64 / elapsed.as_secs_f64().max(0.001),
    })
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct CacheStats {
    pub len: usize,
    pub used_bytes: usize,
    pub byte_cap: usize,
    pub hit_rate: f64,
    pub evictions: u64,
    pub expired_total: u64,
}

struct Flight {
    value: Mutex<Option<Vec<u8>>>,
    ready: Condvar,
}

pub(super) struct SingleFlightCache {
    cache: Mutex<LruTtlCache<String, Vec<u8>>>,
    flights: Mutex<HashMap<String, Arc<Flight>>>,
}

impl SingleFlightCache {
    pub(super) fn new(
        byte_cap: usize,
        ttl: Duration,
        jitter: Duration,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, String> {
        Ok(Self {
            cache: Mutex::new(LruTtlCache::with_jitter(byte_cap, ttl, jitter, clock).map_err(err)?),
            flights: Mutex::new(HashMap::new()),
        })
    }

    pub(super) fn insert_seed(
        &self,
        key: &str,
        value: Vec<u8>,
        size_bytes: usize,
    ) -> Result<(), String> {
        self.cache
            .lock()
            .expect("cache lock poisoned")
            .insert(key.to_string(), value, size_bytes)
            .map(|_| ())
            .map_err(err)
    }

    pub(super) fn get_or_compute<F>(
        &self,
        key: &str,
        size_bytes: usize,
        compute: F,
    ) -> Result<Vec<u8>, String>
    where
        F: FnOnce() -> Vec<u8>,
    {
        if let Some(value) = self
            .cache
            .lock()
            .expect("cache lock poisoned")
            .get(&key.to_string())
            .cloned()
        {
            return Ok(value);
        }

        let (flight, leader) = {
            let mut flights = self.flights.lock().expect("flight map poisoned");
            if let Some(existing) = flights.get(key) {
                (Arc::clone(existing), false)
            } else {
                let flight = Arc::new(Flight {
                    value: Mutex::new(None),
                    ready: Condvar::new(),
                });
                flights.insert(key.to_string(), Arc::clone(&flight));
                (flight, true)
            }
        };

        if !leader {
            let mut value = flight.value.lock().expect("flight value poisoned");
            while value.is_none() {
                value = flight.ready.wait(value).expect("flight wait poisoned");
            }
            return Ok(value.as_ref().expect("flight completed").clone());
        }

        let value = compute();
        self.cache
            .lock()
            .expect("cache lock poisoned")
            .insert(key.to_string(), value.clone(), size_bytes)
            .map_err(err)?;
        {
            let mut slot = flight.value.lock().expect("flight value poisoned");
            *slot = Some(value.clone());
        }
        flight.ready.notify_all();
        self.flights
            .lock()
            .expect("flight map poisoned")
            .remove(key);
        Ok(value)
    }

    pub(super) fn stats(&self) -> CacheStats {
        let cache = self.cache.lock().expect("cache lock poisoned");
        CacheStats {
            len: cache.len(),
            used_bytes: cache.used_bytes(),
            byte_cap: cache.byte_cap(),
            hit_rate: cache.hit_rate(),
            evictions: cache.evictions(),
            expired_total: cache.expired_total(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TtlJitterReadback {
    pub configured_jitter_ms: u64,
    pub entries: usize,
    pub live_at_base_ttl: usize,
    pub expired_at_base_ttl: usize,
    pub spread_observed: bool,
}

pub(super) fn ttl_jitter_readback() -> Result<TtlJitterReadback, String> {
    let clock = MutableClock::new(10_000);
    let mut cache = LruTtlCache::with_jitter(
        16 * 1024,
        Duration::from_millis(100),
        Duration::from_millis(80),
        Arc::new(clock.clone()),
    )
    .map_err(err)?;
    for idx in 0..64_u64 {
        cache
            .insert(format!("j{idx:02}"), vec![idx as u8; 8], 8)
            .map_err(err)?;
    }
    clock.set(10_100);
    let mut live = 0_usize;
    for idx in 0..64_u64 {
        if cache.get(&format!("j{idx:02}")).is_some() {
            live += 1;
        }
    }
    Ok(TtlJitterReadback {
        configured_jitter_ms: 80,
        entries: 64,
        live_at_base_ttl: live,
        expired_at_base_ttl: 64 - live,
        spread_observed: live > 0 && live < 64,
    })
}

pub(super) fn quota_vault_id(idx: usize) -> VaultId {
    VaultId::from_str(&format!("01ARZ3NDEKTSV4RRFFQ69G5F{idx:02}")).expect("valid vault id")
}

pub(super) fn try_lock_for<T>(
    lock: &Mutex<T>,
    timeout: Duration,
) -> std::result::Result<(), &'static str> {
    let started = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(_guard) => return Ok(()),
            Err(std::sync::TryLockError::WouldBlock) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(std::sync::TryLockError::WouldBlock) => return Err("CALYX_LOCK_TIMEOUT"),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err("CALYX_LOCK_POISONED"),
        }
    }
}

#[derive(Clone)]
pub(super) struct LengthLens {
    contract: FrozenLensContract,
    calls: Arc<AtomicUsize>,
    offset: f32,
}

impl LengthLens {
    pub(super) fn new(name: &str, offset: f32) -> Self {
        Self {
            contract: contract(name),
            calls: Arc::new(AtomicUsize::new(0)),
            offset,
        }
    }

    pub(super) fn contract(&self) -> FrozenLensContract {
        self.contract.clone()
    }

    pub(super) fn calls(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.calls)
    }
}

impl Lens for LengthLens {
    fn id(&self) -> LensId {
        self.contract.lens_id()
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(1)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, input: &Input) -> CalyxResult<SlotVector> {
        Ok(SlotVector::Dense {
            dim: 1,
            data: vec![input.bytes.len() as f32 + self.offset],
        })
    }

    fn measure_batch(&self, inputs: &[Input]) -> CalyxResult<Vec<SlotVector>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        inputs.iter().map(|input| self.measure(input)).collect()
    }
}

#[derive(Clone)]
pub(super) struct TimeoutLens {
    contract: FrozenLensContract,
    calls: Arc<AtomicUsize>,
    fail: Arc<AtomicBool>,
    timeout: Duration,
}

impl TimeoutLens {
    pub(super) fn new(name: &str, timeout: Duration) -> Self {
        Self {
            contract: contract(name),
            calls: Arc::new(AtomicUsize::new(0)),
            fail: Arc::new(AtomicBool::new(true)),
            timeout,
        }
    }

    pub(super) fn contract(&self) -> FrozenLensContract {
        self.contract.clone()
    }

    pub(super) fn calls(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.calls)
    }

    pub(super) fn restore(&self) {
        self.fail.store(false, Ordering::SeqCst);
    }
}

impl Lens for TimeoutLens {
    fn id(&self) -> LensId {
        self.contract.lens_id()
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(1)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, input: &Input) -> CalyxResult<SlotVector> {
        if self.fail.load(Ordering::SeqCst) {
            std::thread::sleep(self.timeout);
            return Err(CalyxError::lens_unreachable(
                "synthetic per-lens timeout fired",
            ));
        }
        Ok(SlotVector::Dense {
            dim: 1,
            data: vec![input.bytes.len() as f32 + 100.0],
        })
    }

    fn measure_batch(&self, inputs: &[Input]) -> CalyxResult<Vec<SlotVector>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        inputs.iter().map(|input| self.measure(input)).collect()
    }
}

pub(super) fn dense_lengths(readout: &IngestPanelReadout, lens_id: LensId) -> Vec<f32> {
    readout
        .outcomes
        .iter()
        .find(|outcome| outcome.lens_id == lens_id)
        .map(|outcome| {
            outcome
                .vectors
                .iter()
                .map(|vector| match vector {
                    SlotVector::Dense { data, .. } => data[0],
                    SlotVector::Absent { .. } => f32::NAN,
                    _ => f32::NAN,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn contract(name: &str) -> FrozenLensContract {
    FrozenLensContract::new(
        name,
        sha256_digest(&[name.as_bytes(), b"weights"]),
        sha256_digest(&[name.as_bytes(), b"corpus"]),
        SlotShape::Dense(1),
        Modality::Text,
        LensDType::F32,
        NormPolicy::None,
    )
}
