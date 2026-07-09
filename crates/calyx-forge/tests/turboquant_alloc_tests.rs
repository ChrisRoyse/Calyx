use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use calyx_forge::{QuantLevel, Quantizer, Result, TurboQuantCodec, new_seed};

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: forwards the exact layout to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: forwards the exact layout to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        // SAFETY: forwards the exact pointer, layout, and new size to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwards the exact pointer and layout to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn record_allocation() {
    if COUNTING.load(Ordering::Relaxed) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

struct CountingGuard;

impl Drop for CountingGuard {
    fn drop(&mut self) {
        COUNTING.store(false, Ordering::SeqCst);
    }
}

fn count_allocations<T>(run: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATIONS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    let guard = CountingGuard;
    let result = run();
    drop(guard);
    (result, ALLOCATIONS.load(Ordering::SeqCst))
}

fn unitish_vec(dim: usize, salt: f32) -> Vec<f32> {
    let mut out = (0..dim)
        .map(|idx| {
            let x = idx as f32 + 1.0;
            (x * 0.173 + salt).sin() + (x * 0.071).cos() * 0.25
        })
        .collect::<Vec<_>>();
    let norm = out.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut out {
        *value /= norm;
    }
    out
}

#[test]
fn turboquant_dot_prepared_allocates_zero_in_hot_loop() -> Result<()> {
    let codec = TurboQuantCodec::new(new_seed(1536, b"tq_alloc_hot_loop"), QuantLevel::Bits3p5)?;
    let left = codec.encode(&unitish_vec(1536, 0.25))?;
    let right = codec.encode(&unitish_vec(1536, 0.75))?;
    let prepared_left = codec.prepare(&left)?;
    let prepared_right = codec.prepare(&right)?;
    let warmup = codec.dot_prepared(&prepared_left, &prepared_right);
    assert!(warmup.is_finite());

    let (score, allocations) = count_allocations(|| {
        let mut score = 0.0_f32;
        for _ in 0..1024 {
            score += codec.dot_prepared(&prepared_left, &prepared_right);
        }
        black_box(score)
    });

    assert!(score.is_finite());
    assert_eq!(allocations, 0);
    println!(
        "turboquant_dot_prepared_allocates_zero PASSED allocations={allocations} iterations=1024 score={score:.6}"
    );
    Ok(())
}
