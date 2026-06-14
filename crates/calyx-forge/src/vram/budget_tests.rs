use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

const GIB: usize = 1024 * 1024 * 1024;
const MIB: usize = 1024 * 1024;
const CODE: &str = "CALYX_FORGE_VRAM_BUDGET";

struct StaticProbe {
    free: usize,
}

impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> Result<usize> {
        Ok(self.free)
    }
}

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

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn soft_cap_accounting_reserve_and_release() {
    let b = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 32 * GIB });

    let g1 = b.reserve(512 * MIB).expect("first 512 MiB reservation");
    assert_eq!(b.allocated_bytes(), 512 * MIB);
    assert_eq!(b.allocated_bytes_for(Category::Serving), 512 * MIB);
    assert_eq!(g1.category(), Category::Serving);

    let g2 = b.reserve(512 * MIB).expect("second 512 MiB reservation");
    assert_eq!(b.allocated_bytes(), GIB);
    assert_eq!(b.allocated_bytes_for(Category::Serving), GIB);

    match b.reserve(1) {
        Ok(_) => panic!("over-cap reservation must fail"),
        Err(err) => assert_eq!(err.code(), CODE),
    }
    assert_eq!(b.allocated_bytes(), GIB);

    drop(g1);
    drop(g2);
    assert_eq!(b.allocated_bytes(), 0);
    assert_eq!(b.allocated_bytes_for(Category::Serving), 0);
}

#[test]
fn category_accounting_splits_serving_and_anneal() {
    let b = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 32 * GIB });
    let serving = b.reserve(128 * MIB).expect("serving reservation");
    let anneal = b
        .reserve_category(256 * MIB, Category::Anneal)
        .expect("anneal reservation");

    let stats = b.stats();
    assert_eq!(stats.allocated_bytes, 384 * MIB);
    assert_eq!(stats.serving_allocated_bytes, 128 * MIB);
    assert_eq!(stats.anneal_allocated_bytes, 256 * MIB);
    assert_eq!(serving.category(), Category::Serving);
    assert_eq!(anneal.category(), Category::Anneal);

    drop(anneal);
    assert_eq!(b.allocated_bytes_for(Category::Anneal), 0);
    assert_eq!(b.allocated_bytes_for(Category::Serving), 128 * MIB);
    drop(serving);
    assert_eq!(b.allocated_bytes(), 0);
}

#[test]
fn category_cap_rejects_before_counter_change() {
    let b = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 32 * GIB });
    let _g = b
        .reserve_category_with_cap(256 * MIB, Category::Anneal, 256 * MIB)
        .expect("exact anneal cap fits");

    let err = match b.reserve_category_with_cap(1, Category::Anneal, 256 * MIB) {
        Ok(_) => panic!("one byte over category cap rejects"),
        Err(err) => err,
    };

    assert_eq!(err.code(), CODE);
    assert_eq!(b.allocated_bytes_for(Category::Anneal), 256 * MIB);
    assert_eq!(b.allocated_bytes(), 256 * MIB);
}

#[test]
fn guard_releases_on_drop() {
    let b = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 32 * GIB });
    {
        let _g = b.reserve(256 * MIB).expect("reservation");
        assert_eq!(b.allocated_bytes(), 256 * MIB);
    }
    assert_eq!(b.allocated_bytes(), 0);
    let _g = b.reserve(256 * MIB).expect("re-reservation after release");
    assert_eq!(b.allocated_bytes(), 256 * MIB);
}

#[test]
fn parse_soft_cap_known_inputs() {
    assert_eq!(parse_soft_cap_strict(Some("1073741824")).unwrap(), GIB);
    assert_eq!(parse_soft_cap_strict(None).unwrap(), DEFAULT_SOFT_CAP_BYTES);
    assert_eq!(DEFAULT_SOFT_CAP_BYTES, 12_884_901_888);
    assert_eq!(parse_soft_cap_strict(Some(" 1073741824 ")).unwrap(), GIB);
    let err = parse_soft_cap_strict(Some("not-a-number")).expect_err("must reject garbage");
    assert_eq!(err.code(), CODE);
}

#[test]
fn from_env_reads_configured_cap() {
    let _lock = ENV_LOCK.lock().unwrap();
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
    assert!(b.can_allocate(0).is_ok());
}

#[test]
fn zero_byte_reservation_skips_device_query() {
    let b = VramBudgeter::with_soft_cap(GIB, FailingProbe);
    assert!(b.can_allocate(0).is_ok());
    let g = b.reserve(0).expect("zero-byte reservation");
    assert_eq!(g.bytes(), 0);
    assert_eq!(g.category(), Category::Serving);
    assert_eq!(b.allocated_bytes(), 0);
    drop(g);
    assert_eq!(b.allocated_bytes(), 0);
}

#[test]
fn probe_failure_is_fail_closed() {
    let b = VramBudgeter::with_soft_cap(GIB, FailingProbe);
    let err = b
        .can_allocate(1024)
        .expect_err("probe failure => over-budget");
    assert_eq!(err.code(), CODE);
    assert!(b.reserve(1024).is_err());
    assert_eq!(b.allocated_bytes(), 0);
}

#[test]
fn device_headroom_gate_independent_of_soft_cap() {
    let b = VramBudgeter::with_soft_cap(
        32 * GIB,
        StaticProbe {
            free: RESERVED_HEADROOM_BYTES + 1024,
        },
    );
    assert!(b.can_allocate(1024).is_ok());
    let err = b.can_allocate(1025).expect_err("device headroom exceeded");
    assert_eq!(err.code(), CODE);
}

#[test]
fn free_below_headroom_saturates_to_zero_usable() {
    let b = VramBudgeter::with_soft_cap(
        32 * GIB,
        StaticProbe {
            free: RESERVED_HEADROOM_BYTES - 1,
        },
    );
    assert_eq!(b.can_allocate(1).unwrap_err().code(), CODE);
    assert!(b.can_allocate(0).is_ok());
}

proptest::proptest! {
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
        proptest::prop_assert_eq!(budgeter.allocated_bytes_for(Category::Serving), 0);
    }
}
