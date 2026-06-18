use std::sync::OnceLock;

type DistanceFn = fn(&[f32], &[f32]) -> f32;

#[derive(Clone, Copy)]
struct Kernels {
    backend: &'static str,
    cosine_distance: DistanceFn,
    dot: DistanceFn,
    l2_sq: DistanceFn,
}

static KERNELS: OnceLock<Kernels> = OnceLock::new();

/// Runtime-selected distance backend. Used by FSV/bench readback to prove whether
/// the hot path is using SIMD on the current machine.
pub fn kernel_backend() -> &'static str {
    kernels().backend
}

pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    (kernels().cosine_distance)(a, b)
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    (kernels().dot)(a, b)
}

pub fn l2_sq(a: &[f32], b: &[f32]) -> f32 {
    (kernels().l2_sq)(a, b)
}

pub fn unit_l2_cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    0.5 * l2_sq(a, b)
}

pub fn l2_normalize(vector: &[f32]) -> Vec<f32> {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm == 0.0 {
        vector.to_vec()
    } else {
        vector.iter().map(|v| v / norm).collect()
    }
}

fn kernels() -> &'static Kernels {
    KERNELS.get_or_init(detect_kernels)
}

fn detect_kernels() -> Kernels {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            return Kernels {
                backend: "avx2",
                cosine_distance: cosine_distance_avx2_dispatch,
                dot: dot_avx2_dispatch,
                l2_sq: l2_sq_avx2_dispatch,
            };
        }
    }
    Kernels {
        backend: "scalar",
        cosine_distance: cosine_distance_scalar,
        dot: dot_scalar,
        l2_sq: l2_sq_scalar,
    }
}

fn cosine_distance_scalar(a: &[f32], b: &[f32]) -> f32 {
    let (dot, an, bn) = cosine_parts_scalar(a, b);
    if an == 0.0 || bn == 0.0 {
        1.0
    } else {
        (1.0 - dot / (an.sqrt() * bn.sqrt())).max(0.0)
    }
}

fn cosine_parts_scalar(a: &[f32], b: &[f32]) -> (f32, f32, f32) {
    let len = a.len().min(b.len());
    let mut dot = 0.0;
    let mut an = 0.0;
    let mut bn = 0.0;
    for i in 0..len {
        let x = a[i];
        let y = b[i];
        dot += x * y;
        an += x * x;
        bn += y * y;
    }
    (dot, an, bn)
}

pub fn dot_scalar(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    let mut sum = 0.0;
    for i in 0..len {
        sum += a[i] * b[i];
    }
    sum
}

pub fn l2_sq_scalar(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    let mut sum = 0.0;
    for i in 0..len {
        let d = a[i] - b[i];
        sum += d * d;
    }
    sum
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn cosine_distance_avx2_dispatch(a: &[f32], b: &[f32]) -> f32 {
    unsafe { x86::cosine_distance_avx2(a, b) }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn dot_avx2_dispatch(a: &[f32], b: &[f32]) -> f32 {
    unsafe { x86::dot_avx2(a, b) }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn l2_sq_avx2_dispatch(a: &[f32], b: &[f32]) -> f32 {
    unsafe { x86::l2_sq_avx2(a, b) }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86 {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn cosine_distance_avx2(a: &[f32], b: &[f32]) -> f32 {
        let (dot, an, bn) = unsafe { cosine_parts_avx2(a, b) };
        if an == 0.0 || bn == 0.0 {
            1.0
        } else {
            (1.0 - dot / (an.sqrt() * bn.sqrt())).max(0.0)
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn dot_avx2(a: &[f32], b: &[f32]) -> f32 {
        let len = a.len().min(b.len());
        let mut acc = _mm256_setzero_ps();
        let mut i = 0;
        while i + 8 <= len {
            let av = unsafe { _mm256_loadu_ps(a.as_ptr().add(i)) };
            let bv = unsafe { _mm256_loadu_ps(b.as_ptr().add(i)) };
            acc = _mm256_add_ps(acc, _mm256_mul_ps(av, bv));
            i += 8;
        }
        let mut out = hsum(acc);
        while i < len {
            out += a[i] * b[i];
            i += 1;
        }
        out
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn l2_sq_avx2(a: &[f32], b: &[f32]) -> f32 {
        let len = a.len().min(b.len());
        let mut acc = _mm256_setzero_ps();
        let mut i = 0;
        while i + 8 <= len {
            let av = unsafe { _mm256_loadu_ps(a.as_ptr().add(i)) };
            let bv = unsafe { _mm256_loadu_ps(b.as_ptr().add(i)) };
            let d = _mm256_sub_ps(av, bv);
            acc = _mm256_add_ps(acc, _mm256_mul_ps(d, d));
            i += 8;
        }
        let mut out = hsum(acc);
        while i < len {
            let d = a[i] - b[i];
            out += d * d;
            i += 1;
        }
        out
    }

    #[target_feature(enable = "avx2")]
    unsafe fn cosine_parts_avx2(a: &[f32], b: &[f32]) -> (f32, f32, f32) {
        let len = a.len().min(b.len());
        let mut dot = _mm256_setzero_ps();
        let mut an = _mm256_setzero_ps();
        let mut bn = _mm256_setzero_ps();
        let mut i = 0;
        while i + 8 <= len {
            let av = unsafe { _mm256_loadu_ps(a.as_ptr().add(i)) };
            let bv = unsafe { _mm256_loadu_ps(b.as_ptr().add(i)) };
            dot = _mm256_add_ps(dot, _mm256_mul_ps(av, bv));
            an = _mm256_add_ps(an, _mm256_mul_ps(av, av));
            bn = _mm256_add_ps(bn, _mm256_mul_ps(bv, bv));
            i += 8;
        }
        let (mut dot, mut an, mut bn) = (hsum(dot), hsum(an), hsum(bn));
        while i < len {
            let x = a[i];
            let y = b[i];
            dot += x * y;
            an += x * x;
            bn += y * y;
            i += 1;
        }
        (dot, an, bn)
    }

    #[target_feature(enable = "avx2")]
    fn hsum(v: __m256) -> f32 {
        let mut lanes = [0.0_f32; 8];
        unsafe { _mm256_storeu_ps(lanes.as_mut_ptr(), v) };
        lanes.into_iter().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec512(seed: u32) -> Vec<f32> {
        (0..512)
            .map(|i| {
                let n = seed
                    .wrapping_mul(1_664_525)
                    .wrapping_add((i as u32).wrapping_mul(1_013_904_223));
                ((n % 10_000) as f32 / 5_000.0) - 1.0
            })
            .collect()
    }

    #[test]
    fn selected_kernel_matches_scalar_reference() {
        let a = vec512(7);
        let b = vec512(29);

        println!("distance kernel backend: {}", kernel_backend());
        assert_close(l2_sq(&a, &b), l2_sq_scalar(&a, &b), 0.001);
        assert_close(dot(&a, &b), dot_scalar(&a, &b), 0.001);
        assert_close(
            cosine_distance(&a, &b),
            cosine_distance_scalar(&a, &b),
            0.00001,
        );
    }

    #[test]
    fn cosine_distance_preserves_zero_vector_convention() {
        let a = vec![0.0_f32; 512];
        let b = vec512(11);

        assert_eq!(cosine_distance(&a, &b), 1.0);
    }

    #[test]
    fn l2_sq_uses_shared_zip_semantics_for_mismatched_lengths() {
        let a = [1.0_f32, 3.0, 100.0];
        let b = [2.0_f32, -1.0];

        assert_eq!(l2_sq(&a, &b), 17.0);
    }

    #[test]
    fn unit_l2_cosine_distance_matches_normalized_cosine_distance() {
        let a = l2_normalize(&vec512(7));
        let b = l2_normalize(&vec512(29));

        assert_close(
            unit_l2_cosine_distance(&a, &b),
            cosine_distance(&a, &b),
            0.00001,
        );
    }

    fn assert_close(actual: f32, expected: f32, tolerance: f32) {
        let delta = (actual - expected).abs();
        assert!(
            delta <= tolerance,
            "actual {actual} expected {expected} delta {delta} tolerance {tolerance}"
        );
    }
}
