//! The VRAM budgeter: soft-cap config, live free-VRAM admission, and atomic
//! usage accounting with RAII release.
//!
//! See [`crate::vram`] for the design rationale (why the probe is injectable).

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::vram::{VramProbe, VramStats};
use crate::{ForgeError, Result};

/// Default soft cap when `CALYX_FORGE_VRAM_BUDGET` is unset: 12 GiB. Leaves
/// ~20 GiB of the 32 GiB device for the three resident TEI containers.
pub const DEFAULT_SOFT_CAP_BYTES: usize = 12 * 1024 * 1024 * 1024;

/// Headroom reserved below the live free-VRAM figure for driver/runtime
/// overhead and allocator fragmentation: 512 MiB. A dispatch must fit within
/// `free - RESERVED_HEADROOM_BYTES`, not the full free figure.
pub const RESERVED_HEADROOM_BYTES: usize = 512 * 1024 * 1024;

/// Environment variable that configures the soft cap (bytes, decimal).
pub const VRAM_BUDGET_ENV: &str = "CALYX_FORGE_VRAM_BUDGET";

/// Operator remediation attached to every `CALYX_FORGE_VRAM_BUDGET` error.
pub const VRAM_BUDGET_REMEDIATION: &str =
    "Forge VRAM budget exceeded; reduce batch size or wait for eviction; set CALYX_FORGE_VRAM_BUDGET env var (bytes)";

/// Enforces a soft cap on Forge's cumulative GPU allocation and consults live
/// device free-VRAM before admitting a dispatch.
///
/// Generic over the [`VramProbe`] hardware boundary so the accounting logic is
/// testable with deterministic byte counts on CPU; production uses
/// `CudaVramProbe`. The `allocated_bytes` counter is atomic and shared via a
/// single budgeter instance across all Forge subsystems, so every dispatch
/// sees the same budget.
pub struct VramBudgeter<P: VramProbe> {
    soft_cap_bytes: usize,
    allocated_bytes: AtomicUsize,
    probe: P,
}

impl<P: VramProbe> VramBudgeter<P> {
    /// Construct with an explicit soft cap (bytes) and a probe.
    pub fn with_soft_cap(soft_cap_bytes: usize, probe: P) -> Self {
        Self {
            soft_cap_bytes,
            allocated_bytes: AtomicUsize::new(0),
            probe,
        }
    }

    /// Construct from the environment.
    ///
    /// `CALYX_FORGE_VRAM_BUDGET` unset → [`DEFAULT_SOFT_CAP_BYTES`] (12 GiB).
    /// Set to a decimal byte count → that value. Set to a non-integer →
    /// `Err(CALYX_FORGE_VRAM_BUDGET)` (fail-loud on misconfiguration; no silent
    /// default that would mask an operator typo). Logs the resolved cap.
    pub fn from_env(probe: P) -> Result<Self> {
        let raw = std::env::var(VRAM_BUDGET_ENV).ok();
        let soft_cap_bytes = parse_soft_cap_strict(raw.as_deref())?;
        tracing::info!(
            target: "calyx_forge::vram",
            soft_cap_bytes,
            source = if raw.is_some() { "env" } else { "default" },
            "VRAM budgeter configured"
        );
        Ok(Self::with_soft_cap(soft_cap_bytes, probe))
    }

    /// The configured soft cap in bytes.
    pub fn soft_cap_bytes(&self) -> usize {
        self.soft_cap_bytes
    }

    /// Forge's currently reserved total in bytes (sum of live guards).
    pub fn allocated_bytes(&self) -> usize {
        self.allocated_bytes.load(Ordering::Acquire)
    }

    /// Check — without reserving — whether `bytes` could be allocated now.
    ///
    /// Two gates, both must pass:
    /// 1. soft cap: `allocated + bytes <= soft_cap`;
    /// 2. device headroom: `bytes <= free_device_vram - RESERVED_HEADROOM_BYTES`.
    ///
    /// A zero-byte request is always admissible and short-circuits before any
    /// device query. Any probe failure is treated as over-budget (fail-closed)
    /// and surfaces as `CALYX_FORGE_VRAM_BUDGET`.
    pub fn can_allocate(&self, bytes: usize) -> Result<()> {
        if bytes == 0 {
            return Ok(());
        }
        let current = self.allocated_bytes.load(Ordering::Acquire);
        self.check_soft_cap(current, bytes)?;
        self.check_device_headroom(bytes)?;
        Ok(())
    }

    /// Reserve `bytes`, returning an RAII [`VramGuard`] that releases the
    /// reservation on drop.
    ///
    /// The soft-cap reservation is performed with a compare-and-swap loop so
    /// concurrent reservers can never collectively exceed `soft_cap` (no
    /// time-of-check/time-of-use race on the counter). The device-headroom
    /// gate is checked first; it is necessarily best-effort against the live
    /// driver, but the atomic soft cap is the hard invariant.
    pub fn reserve(&self, bytes: usize) -> Result<VramGuard<'_, P>> {
        if bytes == 0 {
            return Ok(VramGuard {
                budgeter: self,
                bytes: 0,
            });
        }
        self.check_device_headroom(bytes)?;

        let mut current = self.allocated_bytes.load(Ordering::Acquire);
        loop {
            let projected = self.checked_projection(current, bytes)?;
            match self.allocated_bytes.compare_exchange_weak(
                current,
                projected,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(VramGuard {
                        budgeter: self,
                        bytes,
                    });
                }
                Err(actual) => current = actual,
            }
        }
    }

    /// Snapshot accounting + live device free VRAM. A probe failure logs at
    /// warn level and reports `device_free_bytes = 0` (a visible alarm, never
    /// a silent success — control decisions go through [`Self::reserve`], which
    /// fails closed).
    pub fn stats(&self) -> VramStats {
        let device_free_bytes = match self.probe.free_device_vram() {
            Ok(free) => free,
            Err(err) => {
                tracing::warn!(
                    target: "calyx_forge::vram",
                    error = %err,
                    "free-VRAM probe failed during stats(); reporting device_free_bytes=0"
                );
                0
            }
        };
        VramStats {
            soft_cap_bytes: self.soft_cap_bytes,
            allocated_bytes: self.allocated_bytes.load(Ordering::Acquire),
            device_free_bytes,
        }
    }

    fn check_soft_cap(&self, current: usize, bytes: usize) -> Result<()> {
        self.checked_projection(current, bytes).map(|_| ())
    }

    /// `current + bytes`, validated against overflow and the soft cap.
    fn checked_projection(&self, current: usize, bytes: usize) -> Result<usize> {
        let projected = current.checked_add(bytes).ok_or_else(|| {
            budget_err(format!(
                "reservation arithmetic overflow: allocated={current} + requested={bytes}"
            ))
        })?;
        if projected > self.soft_cap_bytes {
            return Err(budget_err(format!(
                "soft cap exceeded: allocated={current} + requested={bytes} = {projected} > soft_cap={}",
                self.soft_cap_bytes
            )));
        }
        Ok(projected)
    }

    fn check_device_headroom(&self, bytes: usize) -> Result<()> {
        let free = self.probe.free_device_vram().map_err(|err| {
            budget_err(format!(
                "device free-VRAM query failed; treating unknown device state as over-budget: {err}"
            ))
        })?;
        let usable = free.saturating_sub(RESERVED_HEADROOM_BYTES);
        if bytes > usable {
            return Err(budget_err(format!(
                "insufficient device VRAM: requested={bytes} > usable={usable} (free={free} - headroom={RESERVED_HEADROOM_BYTES})"
            )));
        }
        Ok(())
    }
}

/// RAII handle for a live VRAM reservation. Dropping it returns `bytes` to the
/// budgeter's available pool. Holding it keeps that VRAM accounted-for.
pub struct VramGuard<'b, P: VramProbe> {
    budgeter: &'b VramBudgeter<P>,
    bytes: usize,
}

impl<P: VramProbe> VramGuard<'_, P> {
    /// The number of bytes this guard holds reserved.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

impl<P: VramProbe> Drop for VramGuard<'_, P> {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.budgeter
                .allocated_bytes
                .fetch_sub(self.bytes, Ordering::AcqRel);
        }
    }
}

fn budget_err(detail: String) -> ForgeError {
    ForgeError::VramBudget {
        detail,
        remediation: VRAM_BUDGET_REMEDIATION.to_string(),
    }
}

/// Parse the soft cap from a raw env value. Fail-loud on a non-integer; default
/// only on absence. Pure (no env access) so it can be tested with known input.
fn parse_soft_cap_strict(raw: Option<&str>) -> Result<usize> {
    match raw {
        None => Ok(DEFAULT_SOFT_CAP_BYTES),
        Some(s) => s.trim().parse::<usize>().map_err(|_| {
            budget_err(format!(
                "{VRAM_BUDGET_ENV}={s:?} is not a valid byte count (expected a non-negative integer number of bytes)"
            ))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Barrier, Mutex};

    const GIB: usize = 1024 * 1024 * 1024;
    const MIB: usize = 1024 * 1024;
    const CODE: &str = "CALYX_FORGE_VRAM_BUDGET";

    /// Deterministic probe returning a fixed free-VRAM reading.
    struct StaticProbe {
        free: usize,
    }
    impl VramProbe for StaticProbe {
        fn free_device_vram(&self) -> Result<usize> {
            Ok(self.free)
        }
    }

    /// Probe that always fails — stands in for a `cudaMemGetInfo` driver error.
    struct FailingProbe;
    impl VramProbe for FailingProbe {
        fn free_device_vram(&self) -> Result<usize> {
            Err(ForgeError::DeviceUnavailable {
                device: "test-gpu".into(),
                detail: "simulated cudaMemGetInfo failure".into(),
                remediation: "n/a".into(),
            })
        }
    }

    // Serialize the one test that mutates process-global env.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn soft_cap_accounting_reserve_and_release() {
        // soft_cap = 1 GiB, abundant device free VRAM.
        let b = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 32 * GIB });

        let g1 = b.reserve(512 * MIB).expect("first 512 MiB reservation");
        assert_eq!(b.allocated_bytes(), 512 * MIB);

        let g2 = b.reserve(512 * MIB).expect("second 512 MiB reservation");
        assert_eq!(b.allocated_bytes(), GIB);

        // 1 byte over the cap fails closed with the exact code.
        match b.reserve(1) {
            Ok(_) => panic!("over-cap reservation must fail"),
            Err(err) => assert_eq!(err.code(), CODE),
        }
        // ... and does not perturb the counter.
        assert_eq!(b.allocated_bytes(), GIB);

        drop(g1);
        drop(g2);
        assert_eq!(b.allocated_bytes(), 0);
    }

    #[test]
    fn guard_releases_on_drop() {
        let b = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 32 * GIB });
        {
            let _g = b.reserve(256 * MIB).expect("reservation");
            assert_eq!(b.allocated_bytes(), 256 * MIB);
        }
        assert_eq!(b.allocated_bytes(), 0);
        // The freed budget is immediately reusable.
        let _g = b.reserve(256 * MIB).expect("re-reservation after release");
        assert_eq!(b.allocated_bytes(), 256 * MIB);
    }

    #[test]
    fn parse_soft_cap_known_inputs() {
        assert_eq!(parse_soft_cap_strict(Some("1073741824")).unwrap(), GIB);
        assert_eq!(parse_soft_cap_strict(None).unwrap(), DEFAULT_SOFT_CAP_BYTES);
        // Default is exactly 12 GiB.
        assert_eq!(DEFAULT_SOFT_CAP_BYTES, 12_884_901_888);
        // Whitespace tolerated.
        assert_eq!(parse_soft_cap_strict(Some(" 1073741824 ")).unwrap(), GIB);
        // Garbage fails loud.
        let err = parse_soft_cap_strict(Some("not-a-number")).expect_err("must reject garbage");
        assert_eq!(err.code(), CODE);
    }

    #[test]
    fn from_env_reads_configured_cap() {
        let _lock = ENV_LOCK.lock().unwrap();
        // SAFETY: env access is serialized by ENV_LOCK; we restore below.
        unsafe { std::env::set_var(VRAM_BUDGET_ENV, "1073741824") };
        let b = VramBudgeter::from_env(StaticProbe { free: 32 * GIB }).unwrap();
        assert_eq!(b.soft_cap_bytes(), GIB);

        unsafe { std::env::remove_var(VRAM_BUDGET_ENV) };
        let b2 = VramBudgeter::from_env(StaticProbe { free: 32 * GIB }).unwrap();
        assert_eq!(b2.soft_cap_bytes(), DEFAULT_SOFT_CAP_BYTES);
    }

    #[test]
    fn zero_soft_cap_rejects_all_nonzero() {
        let b = VramBudgeter::with_soft_cap(0, StaticProbe { free: 32 * GIB });
        assert_eq!(b.can_allocate(1).unwrap_err().code(), CODE);
        assert!(b.reserve(1).is_err());
        // A zero-byte request is still valid.
        assert!(b.can_allocate(0).is_ok());
    }

    #[test]
    fn zero_byte_reservation_skips_device_query() {
        // FailingProbe would error if consulted; a 0-byte request must not
        // consult it and must succeed with no accounting change.
        let b = VramBudgeter::with_soft_cap(GIB, FailingProbe);
        assert!(b.can_allocate(0).is_ok());
        let g = b.reserve(0).expect("zero-byte reservation");
        assert_eq!(g.bytes(), 0);
        assert_eq!(b.allocated_bytes(), 0);
        drop(g);
        assert_eq!(b.allocated_bytes(), 0);
    }

    #[test]
    fn probe_failure_is_fail_closed() {
        let b = VramBudgeter::with_soft_cap(GIB, FailingProbe);
        let err = b.can_allocate(1024).expect_err("probe failure => over-budget");
        assert_eq!(err.code(), CODE);
        assert!(b.reserve(1024).is_err());
        assert_eq!(b.allocated_bytes(), 0);
    }

    #[test]
    fn device_headroom_gate_independent_of_soft_cap() {
        // Huge soft cap, but only 1 KiB usable after the 512 MiB headroom.
        let b = VramBudgeter::with_soft_cap(32 * GIB, StaticProbe {
            free: RESERVED_HEADROOM_BYTES + 1024,
        });
        // Exactly usable succeeds.
        assert!(b.can_allocate(1024).is_ok());
        // One byte more than usable fails despite the soft cap being huge.
        let err = b.can_allocate(1025).expect_err("device headroom exceeded");
        assert_eq!(err.code(), CODE);
    }

    #[test]
    fn free_below_headroom_saturates_to_zero_usable() {
        // free < headroom => usable saturates to 0 => every nonzero alloc fails.
        let b = VramBudgeter::with_soft_cap(32 * GIB, StaticProbe {
            free: RESERVED_HEADROOM_BYTES - 1,
        });
        assert_eq!(b.can_allocate(1).unwrap_err().code(), CODE);
        assert!(b.can_allocate(0).is_ok());
    }

    proptest::proptest! {
        /// No matter how reservations interleave across threads, the live
        /// total never exceeds the soft cap, and all guards release cleanly.
        #[test]
        fn concurrent_reservations_never_exceed_soft_cap(
            soft_cap in 1usize..=4096,
            allocs in proptest::collection::vec(1usize..=512, 1..24),
        ) {
            let budgeter = Arc::new(VramBudgeter::with_soft_cap(
                soft_cap,
                StaticProbe { free: usize::MAX },
            ));
            let barrier = Arc::new(Barrier::new(allocs.len()));
            let peak = Arc::new(AtomicUsize::new(0));

            let handles: Vec<_> = allocs
                .into_iter()
                .map(|a| {
                    let b = Arc::clone(&budgeter);
                    let bar = Arc::clone(&barrier);
                    let pk = Arc::clone(&peak);
                    std::thread::spawn(move || {
                        // Hold the guard (if admitted) until every thread has
                        // attempted, so the peak reflects the concurrent sum.
                        let guard = b.reserve(a).ok();
                        pk.fetch_max(b.allocated_bytes(), Ordering::AcqRel);
                        bar.wait();
                        pk.fetch_max(b.allocated_bytes(), Ordering::AcqRel);
                        drop(guard);
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }

            proptest::prop_assert!(peak.load(Ordering::Acquire) <= soft_cap);
            proptest::prop_assert_eq!(budgeter.allocated_bytes(), 0);
        }
    }
}
