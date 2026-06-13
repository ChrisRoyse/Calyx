//! Per-vault resource quotas with backpressure (PH60 · T04).
//!
//! [`QuotaGuard`] enforces per-vault rate limits — ingest (CXs/s), query
//! (queries/s), and IO (bytes/s) — so one heavy tenant cannot starve the
//! others (noisy-neighbor / DoS defense, PRD `30 §1`, `30 §3`). When a quota is
//! exceeded the charge is rejected with [`CALYX_QUOTA_EXCEEDED`] and the window
//! stays tripped (backpressure) until it rolls over — the caller must retry
//! after the window, never silently drop (A16, A26).
//!
//! ## Why one `Mutex`, not lock-free atomics
//!
//! A fixed-window limiter must reset `{window_start, ingest, query, io}` *and*
//! read-modify-write a counter as one indivisible step. Doing that across
//! several independent atomics is a TOCTOU race: two threads can both observe
//! the old window, both reset, and lose each other's increments — silently
//! **over-admitting** past the limit, the exact failure the guard exists to
//! prevent. A single `Mutex` around the whole window makes check-reset-charge
//! atomic, so the "quota is never exceeded" invariant holds under real
//! concurrency (proven by `concurrent_charges_never_over_admit`). Contention is
//! per-vault and each charge is O(1), so the lock is effectively free.

use calyx_core::{CalyxError, Result, VaultId};
use std::sync::Mutex;

/// A per-vault quota was exceeded; the operation is denied and backpressured.
pub const CALYX_QUOTA_EXCEEDED: &str = "CALYX_QUOTA_EXCEEDED";

/// Quota window length: one second, in nanoseconds.
pub const WINDOW_NS: u64 = 1_000_000_000;

/// Per-vault rate limits. `Default` is generous but finite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuotaConfig {
    /// Max constellations ingested per 1-second window.
    pub max_ingest_cx_per_sec: u32,
    /// Max queries per 1-second window.
    pub max_query_per_sec: u32,
    /// Max IO bytes per 1-second window.
    pub max_io_bytes_per_sec: u64,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            max_ingest_cx_per_sec: 1_000,
            max_query_per_sec: 500,
            max_io_bytes_per_sec: 256 * 1024 * 1024, // 256 MiB/s
        }
    }
}

/// Which counter a charge applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resource {
    Ingest,
    Query,
    Io,
}

/// The mutable window state, guarded as one unit so check-reset-charge is atomic.
#[derive(Debug)]
struct Window {
    start_ns: u64,
    ingest_cx: u64,
    query: u64,
    io_bytes: u64,
    /// Config in force for the current window. Refreshed from `pending` only on
    /// window rollover, so a mid-window `update_config` applies next window.
    active: QuotaConfig,
    /// Staged config; becomes `active` at the next window boundary.
    pending: QuotaConfig,
}

impl Window {
    /// Rolls the window over if `now_ns` is at/after the next boundary, applying
    /// any pending config. Boundary is inclusive: a delta of exactly `WINDOW_NS`
    /// starts a fresh window (no off-by-one).
    fn advance(&mut self, now_ns: u64) {
        if now_ns.saturating_sub(self.start_ns) >= WINDOW_NS {
            self.start_ns = now_ns;
            self.ingest_cx = 0;
            self.query = 0;
            self.io_bytes = 0;
            self.active = self.pending;
        }
    }
}

/// Per-vault quota tracker. Share across request handlers via `Arc<QuotaGuard>`.
#[derive(Debug)]
pub struct QuotaGuard {
    vault_id: VaultId,
    window: Mutex<Window>,
}

impl QuotaGuard {
    /// Builds a guard for `vault_id` with `config` active from the first charge.
    pub fn new(vault_id: VaultId, config: QuotaConfig) -> Self {
        Self {
            vault_id,
            window: Mutex::new(Window {
                start_ns: 0,
                ingest_cx: 0,
                query: 0,
                io_bytes: 0,
                active: config,
                pending: config,
            }),
        }
    }

    /// The vault this guard meters.
    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    /// Charges `cx_count` constellations against the ingest quota at `now_ns`.
    ///
    /// # Errors
    /// [`CALYX_QUOTA_EXCEEDED`] if the window total would exceed
    /// `max_ingest_cx_per_sec`. Backpressure: retry after the window rolls over.
    pub fn charge_ingest(&self, cx_count: u32, now_ns: u64) -> Result<()> {
        self.charge(Resource::Ingest, u64::from(cx_count), now_ns)
    }

    /// Charges `count` queries against the query quota at `now_ns`.
    pub fn charge_query(&self, count: u32, now_ns: u64) -> Result<()> {
        self.charge(Resource::Query, u64::from(count), now_ns)
    }

    /// Charges `bytes` against the IO-bytes quota at `now_ns`.
    pub fn charge_io(&self, bytes: u64, now_ns: u64) -> Result<()> {
        self.charge(Resource::Io, bytes, now_ns)
    }

    /// Stages a new config; it takes effect at the next window boundary (not the
    /// current window), so an in-flight window keeps its admitted budget stable.
    pub fn update_config(&self, config: QuotaConfig) {
        let mut w = self.lock();
        w.pending = config;
    }

    /// Snapshot of `(ingest_cx, query, io_bytes)` charged in the current window —
    /// the Source-of-Truth read for FSV.
    pub fn counters(&self) -> (u64, u64, u64) {
        let w = self.lock();
        (w.ingest_cx, w.query, w.io_bytes)
    }

    /// The config currently in force (after any pending rollover already applied).
    pub fn active_config(&self) -> QuotaConfig {
        self.lock().active
    }

    fn charge(&self, resource: Resource, amount: u64, now_ns: u64) -> Result<()> {
        // A zero-cost charge consumes nothing and can never exceed a quota,
        // even on an already-tripped window.
        if amount == 0 {
            return Ok(());
        }
        let mut w = self.lock();
        w.advance(now_ns);
        // Read the limit (a copy) before taking the mutable counter borrow.
        let (limit, name) = match resource {
            Resource::Ingest => (u64::from(w.active.max_ingest_cx_per_sec), "ingest_cx"),
            Resource::Query => (u64::from(w.active.max_query_per_sec), "query"),
            Resource::Io => (w.active.max_io_bytes_per_sec, "io_bytes"),
        };
        let counter = match resource {
            Resource::Ingest => &mut w.ingest_cx,
            Resource::Query => &mut w.query,
            Resource::Io => &mut w.io_bytes,
        };
        // Add-then-check: the tripping charge is counted, so the window stays
        // tripped (every later same-window charge also fails — fail closed).
        let new_total = counter.saturating_add(amount);
        *counter = new_total;
        if new_total > limit {
            return Err(quota_exceeded(format!(
                "vault {} {name} quota exceeded: {new_total} > {limit} per 1s window",
                self.vault_id,
            )));
        }
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Window> {
        self.window
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn quota_exceeded(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_QUOTA_EXCEEDED,
        message: message.into(),
        remediation: "retry after the current 1-second quota window expires",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use ulid::Ulid;

    const T: u64 = 1_000_000_000; // synthetic "now" in ns (1s mark)

    fn vault() -> VaultId {
        VaultId::from_ulid(Ulid::from_bytes([0xC3; 16]))
    }

    fn guard(config: QuotaConfig) -> QuotaGuard {
        QuotaGuard::new(vault(), config)
    }

    #[test]
    fn under_then_over_limit_in_same_window() {
        let g = guard(QuotaConfig::default()); // 1000 CX/s
        assert!(g.charge_ingest(500, T).is_ok());
        println!("after 500: counters = {:?}", g.counters());
        // cumulative 1100 > 1000 -> exceeded.
        let err = g.charge_ingest(600, T).unwrap_err();
        println!(
            "charge_ingest(600) = Err({}); counters = {:?}",
            err.code,
            g.counters()
        );
        assert_eq!(err.code, "CALYX_QUOTA_EXCEEDED");
        assert_eq!(
            g.counters().0,
            1100,
            "tripping charge is counted (window stays tripped)"
        );
    }

    #[test]
    fn window_rollover_resets_counters() {
        let g = guard(QuotaConfig::default());
        let _ = g.charge_ingest(1100, T); // trip the window
        assert!(g.counters().0 >= 1000);
        // New window (delta exactly WINDOW_NS+1) -> reset -> 500 is fine again.
        assert!(g.charge_ingest(500, T + WINDOW_NS + 1).is_ok());
        println!("after rollover, counters = {:?}", g.counters());
        assert_eq!(g.counters().0, 500, "counters reset on new window");
    }

    #[test]
    fn window_boundary_is_inclusive_no_off_by_one() {
        let g = guard(QuotaConfig::default());
        assert!(g.charge_ingest(1000, T).is_ok()); // window starts at T, now full
        // delta WINDOW_NS-1 -> still the SAME window -> one more must be capped.
        assert!(
            g.charge_ingest(1, T + WINDOW_NS - 1).is_err(),
            "within the window the cap holds"
        );
        // delta exactly WINDOW_NS -> NEW window (inclusive boundary) -> reset,
        // so a full 1000 fits again and the counter shows the fresh total.
        assert!(g.charge_ingest(1000, T + WINDOW_NS).is_ok());
        assert_eq!(
            g.counters().0,
            1000,
            "exactly-WINDOW_NS delta starts a fresh window"
        );
    }

    #[test]
    fn io_quota_exceeded_on_default() {
        let g = guard(QuotaConfig::default());
        let err = g.charge_io(256 * 1024 * 1024 + 1, T).unwrap_err();
        println!("charge_io(256MiB+1) = Err({})", err.code);
        assert_eq!(err.code, "CALYX_QUOTA_EXCEEDED");
        // Exactly the limit is allowed.
        let g2 = guard(QuotaConfig::default());
        assert!(g2.charge_io(256 * 1024 * 1024, T).is_ok());
    }

    #[test]
    fn zero_charge_always_ok_even_when_tripped() {
        let g = guard(QuotaConfig::default());
        let _ = g.charge_ingest(2000, T); // trip
        assert!(
            g.charge_ingest(0, T).is_ok(),
            "zero charge never exceeds quota"
        );
        assert!(g.charge_query(0, T).is_ok());
        assert!(g.charge_io(0, T).is_ok());
    }

    #[test]
    fn fail_closed_stays_tripped_for_rest_of_window() {
        let g = guard(QuotaConfig::default());
        assert!(g.charge_ingest(1001, T).is_err()); // trip
        // Every later non-zero same-window charge also fails.
        assert!(g.charge_ingest(1, T).is_err());
        assert!(g.charge_ingest(1, T + WINDOW_NS - 1).is_err());
    }

    #[test]
    fn config_update_applies_next_window_not_current() {
        let g = guard(QuotaConfig::default()); // 1000 CX/s
        assert!(g.charge_ingest(800, T).is_ok());
        // Lower the limit mid-window; current window keeps the old 1000 budget.
        g.update_config(QuotaConfig {
            max_ingest_cx_per_sec: 100,
            ..QuotaConfig::default()
        });
        assert_eq!(
            g.active_config().max_ingest_cx_per_sec,
            1000,
            "current window unchanged"
        );
        assert!(
            g.charge_ingest(150, T).is_ok(),
            "still under old 1000 budget this window"
        );
        // Next window applies the new 100 limit.
        assert!(g.charge_ingest(150, T + WINDOW_NS + 1).is_err());
        assert_eq!(
            g.active_config().max_ingest_cx_per_sec,
            100,
            "new window applies pending config"
        );
    }

    #[test]
    fn concurrent_charges_never_over_admit() {
        // The correctness proof the lock-free design would fail: 8 threads each
        // try 50 unit charges in ONE window; with a 100-CX limit EXACTLY 100
        // must be admitted, never more (no silent over-admission).
        let g = Arc::new(guard(QuotaConfig {
            max_ingest_cx_per_sec: 100,
            ..QuotaConfig::default()
        }));
        // Start the window deterministically at T so no thread triggers a reset.
        g.charge_ingest(0, T).ok();
        let mut handles = Vec::new();
        for _ in 0..8 {
            let g = Arc::clone(&g);
            handles.push(std::thread::spawn(move || {
                let mut ok = 0_u32;
                for _ in 0..50 {
                    if g.charge_ingest(1, T).is_ok() {
                        ok += 1;
                    }
                }
                ok
            }));
        }
        let total_admitted: u32 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        println!(
            "admitted under concurrency = {total_admitted} (limit 100); counter = {:?}",
            g.counters()
        );
        assert_eq!(
            total_admitted, 100,
            "exactly the limit admitted — no over/under-admission"
        );
    }

    proptest::proptest! {
        #[test]
        fn sum_of_admitted_never_exceeds_limit_per_window(
            charges in proptest::collection::vec(1u32..400, 0..200),
        ) {
            // All charges in a single fixed window (now=T): the sum of ADMITTED
            // charges must never exceed the limit (quota never exceeded silently).
            let limit = 1000u64;
            let g = guard(QuotaConfig { max_ingest_cx_per_sec: limit as u32, ..QuotaConfig::default() });
            let mut admitted_sum = 0u64;
            for c in charges {
                if g.charge_ingest(c, T).is_ok() {
                    admitted_sum += u64::from(c);
                }
            }
            proptest::prop_assert!(admitted_sum <= limit, "admitted {admitted_sum} exceeded limit {limit}");
        }
    }
}
