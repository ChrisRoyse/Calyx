//! PH57 · T01 — Full State Verification for the VRAM budgeter.
//!
//! Source of Truth (SoT): the budgeter's atomic usage counter,
//! `VramBudgeter::allocated_bytes()`, and the `VramStats` snapshot. After every
//! mutating action we perform an *independent* read of that counter (not the
//! return value of the call) and print the before/after state, proving the
//! transition physically occurred.
//!
//! The GPU free-VRAM reading is supplied by a deterministic probe so the
//! accounting decisions run against hand-computed byte counts (the 2+2=4
//! discipline). The live `cudaMemGetInfo` path is exercised separately on
//! manual (see the closing FSV comment on the issue). Here the system under
//! test — the accounting + admission logic — runs on real in-memory bytes.
//!
//! Run with `cargo test -p calyx-forge --test ph57_vram_fsv -- --nocapture`
//! to emit the evidence log.

use calyx_forge::{
    DEFAULT_SOFT_CAP_BYTES, ForgeError, RESERVED_HEADROOM_BYTES, VramBudgeter, VramProbe,
};

const GIB: usize = 1024 * 1024 * 1024;
const MIB: usize = 1024 * 1024;
const CODE: &str = "CALYX_FORGE_VRAM_BUDGET";

/// Deterministic stand-in for `cudaMemGetInfo`, returning a fixed free figure.
struct StaticProbe {
    free: usize,
}
impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> Result<usize, ForgeError> {
        Ok(self.free)
    }
}

/// Probe that always fails — models a `cudaMemGetInfo` driver error.
struct FailingProbe;
impl VramProbe for FailingProbe {
    fn free_device_vram(&self) -> Result<usize, ForgeError> {
        Err(ForgeError::DeviceUnavailable {
            device: "fsv-gpu".into(),
            detail: "simulated cudaMemGetInfo failure".into(),
            remediation: "n/a".into(),
        })
    }
}

/// Independent read of the SoT, printed as evidence.
fn show(tag: &str, b: &VramBudgeter<StaticProbe>) {
    println!(
        "    [SoT] {tag:<28} allocated_bytes = {:>13}",
        b.allocated_bytes()
    );
}

#[test]
fn fsv_happy_path_reserve_release() {
    println!("\n=== FSV 1: happy path — reserve/release accounting (2+2=4) ===");
    // soft_cap = 1 GiB; device shows 32 GiB free (abundant).
    let b = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 32 * GIB });

    // Trigger X: reserve 512 MiB. Outcome Y: allocated == 536_870_912.
    let expected_after_first: usize = 512 * MIB;
    println!("  input: reserve(512 MiB = {} bytes)", 512 * MIB);
    println!("  hand-computed expected allocated = {expected_after_first}");
    show("BEFORE reserve #1", &b);
    let g1 = b.reserve(512 * MIB).expect("reserve 512 MiB");
    show("AFTER reserve #1", &b);
    assert_eq!(b.allocated_bytes(), expected_after_first);
    assert_eq!(b.allocated_bytes(), 536_870_912, "512 MiB in bytes");

    // Second reservation brings us exactly to the cap.
    println!("  input: reserve(512 MiB) again → expected allocated = {GIB} (== soft_cap)");
    show("BEFORE reserve #2", &b);
    let g2 = b.reserve(512 * MIB).expect("reserve 512 MiB #2");
    show("AFTER reserve #2", &b);
    assert_eq!(b.allocated_bytes(), GIB);

    // Release both; SoT must return to 0.
    show("BEFORE drop both", &b);
    drop(g1);
    drop(g2);
    show("AFTER drop both", &b);
    assert_eq!(b.allocated_bytes(), 0);
    println!("  RESULT: SoT transitions match hand-computed values ✓");
}

#[test]
fn fsv_edge_cases_before_after() {
    println!("\n=== FSV 2: boundary & edge audit (≥3 cases, before/after SoT) ===");

    // --- Edge A: at-cap boundary — 1 byte over fails closed, SoT unchanged ---
    println!("\n  -- Edge A: soft-cap boundary (allocated at cap, +1 byte) --");
    let b = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 32 * GIB });
    let _full = b.reserve(GIB).expect("fill to cap");
    show("BEFORE over-cap attempt", &b);
    match b.reserve(1) {
        Ok(_) => panic!("over-cap reservation must fail"),
        Err(e) => {
            println!(
                "    trigger: reserve(1) at cap → error {} (expected {CODE})",
                e.code()
            );
            assert_eq!(e.code(), CODE);
        }
    }
    show("AFTER over-cap attempt", &b);
    assert_eq!(
        b.allocated_bytes(),
        GIB,
        "rejected reservation must not perturb SoT"
    );

    // --- Edge B: empty input — zero-byte reservation, SoT unchanged ---
    println!("\n  -- Edge B: empty input (0-byte reservation) --");
    let b2 = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 32 * GIB });
    show("BEFORE 0-byte reserve", &b2);
    let z = b2.reserve(0).expect("zero-byte reservation is valid");
    show("AFTER 0-byte reserve", &b2);
    assert_eq!(b2.allocated_bytes(), 0);
    drop(z);

    // --- Edge C: device-headroom limit dominates a huge soft cap ---
    println!("\n  -- Edge C: device free-VRAM headroom limit (max-limit case) --");
    // soft_cap 32 GiB, but only 512 MiB + 1 KiB free → usable == 1 KiB.
    let b3 = VramBudgeter::with_soft_cap(
        32 * GIB,
        StaticProbe {
            free: RESERVED_HEADROOM_BYTES + 1024,
        },
    );
    println!(
        "    free={} headroom={} → usable=1024 bytes",
        RESERVED_HEADROOM_BYTES + 1024,
        RESERVED_HEADROOM_BYTES
    );
    show("BEFORE headroom-limited reserve", &b3);
    assert!(b3.can_allocate(1024).is_ok(), "exactly usable admits");
    match b3.reserve(1025) {
        Ok(_) => panic!("over-headroom reservation must fail"),
        Err(e) => {
            println!(
                "    trigger: reserve(1025) > usable(1024) → {} (expected {CODE})",
                e.code()
            );
            assert_eq!(e.code(), CODE);
        }
    }
    show("AFTER headroom-limited reserve", &b3);
    // 1024 still admissible; reserve it and confirm SoT.
    let g = b3.reserve(1024).expect("exactly-usable reservation");
    assert_eq!(b3.allocated_bytes(), 1024);
    drop(g);
    println!("\n  RESULT: all edge transitions physically verified in SoT ✓");
}

#[test]
fn fsv_fail_closed_on_probe_error() {
    println!("\n=== FSV 3: fail-closed — unknown device state ⇒ over-budget ===");
    let b = VramBudgeter::with_soft_cap(GIB, FailingProbe);
    println!("  trigger: can_allocate(1024) with failing cudaMemGetInfo probe");
    match b.can_allocate(1024) {
        Ok(_) => panic!("must fail closed when device state is unknown"),
        Err(e) => {
            println!("    outcome: {} (expected {CODE})", e.code());
            assert_eq!(e.code(), CODE);
        }
    }
    // SoT untouched by a rejected admission.
    assert_eq!(b.allocated_bytes(), 0);
    println!("  RESULT: probe failure surfaces {CODE}, SoT = 0 ✓");
}

#[test]
fn fsv_default_cap_is_12_gib() {
    println!("\n=== FSV 4: default soft cap (unset env) == 12 GiB ===");
    assert_eq!(DEFAULT_SOFT_CAP_BYTES, 12 * GIB);
    assert_eq!(DEFAULT_SOFT_CAP_BYTES, 12_884_901_888);
    println!("  DEFAULT_SOFT_CAP_BYTES = {DEFAULT_SOFT_CAP_BYTES} (== 12 GiB) ✓");
}
