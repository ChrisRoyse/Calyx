use std::collections::BTreeMap;
use std::path::PathBuf;

use calyx_assay::MIN_ASSAY_SAMPLES;
use calyx_core::{CxId, SlotId, VaultId};
use calyx_loom::agreement_batch_gpu;
use serde_json::json;

pub(crate) fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-stage5-fsv"))
}

pub(crate) fn cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

pub(crate) fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

pub(crate) fn assay_vault() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

pub(crate) fn two_slot_map(a: Vec<f32>, b: Vec<f32>) -> BTreeMap<SlotId, Vec<f32>> {
    BTreeMap::from([(slot(1), a), (slot(2), b)])
}

pub(crate) fn slot_map_13() -> BTreeMap<SlotId, Vec<f32>> {
    (0..13)
        .map(|index| {
            let angle = index as f32 * 0.07;
            (slot(index), vec![angle.cos(), angle.sin()])
        })
        .collect()
}

pub(crate) fn correlated_samples(n: usize) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for i in 0..n {
        let t = (i as f32 - n as f32 / 2.0) / n as f32;
        let noise = ((i * 17 % 11) as f32 - 5.0) * 0.002;
        x.push(vec![t]);
        y.push(vec![0.8 * t + noise]);
    }
    (x, y)
}

pub(crate) fn gaussian_mi_bits(x: &[Vec<f32>], y: &[Vec<f32>]) -> f32 {
    let x_mean = x.iter().map(|row| row[0]).sum::<f32>() / x.len() as f32;
    let y_mean = y.iter().map(|row| row[0]).sum::<f32>() / y.len() as f32;
    let mut cov = 0.0;
    let mut xv = 0.0;
    let mut yv = 0.0;
    for (left, right) in x.iter().zip(y) {
        let dx = left[0] - x_mean;
        let dy = right[0] - y_mean;
        cov += dx * dy;
        xv += dx * dx;
        yv += dy * dy;
    }
    let r2 = (cov * cov / (xv * yv)).clamp(0.0, 0.999);
    -0.5 * (1.0 - r2).log2()
}

pub(crate) fn high_dim_matrix(rows: usize, dim: usize) -> Vec<Vec<f32>> {
    (0..rows)
        .map(|row| {
            (0..dim)
                .map(|col| ((row * 31 + col * 17) % 23) as f32 / 23.0)
                .collect()
        })
        .collect()
}

pub(crate) fn agreement_gpu_readback() -> serde_json::Value {
    let a = [1.0, 0.0];
    let b = [0.5, 3.0_f32.sqrt() * 0.5];
    match agreement_batch_gpu(&[(&a, &b)]) {
        Ok(scores) => json!({"backend": "cuda", "scores": scores}),
        Err(error) => json!({"error": error.code, "message": error.message}),
    }
}

pub(crate) fn binary_samples(separable: bool) -> (Vec<Vec<f32>>, Vec<bool>) {
    let mut samples = Vec::with_capacity(MIN_ASSAY_SAMPLES);
    let mut labels = Vec::with_capacity(MIN_ASSAY_SAMPLES);
    for i in 0..MIN_ASSAY_SAMPLES {
        let label = i % 2 == 0;
        labels.push(label);
        let value = if !separable {
            0.0
        } else if label {
            1.0 + (i % 3) as f32 * 0.01
        } else {
            -1.0 - (i % 3) as f32 * 0.01
        };
        samples.push(vec![value]);
    }
    (samples, labels)
}

pub(crate) fn complementary_pair_samples() -> (Vec<Vec<f32>>, Vec<Vec<f32>>, Vec<bool>) {
    let mut left = Vec::with_capacity(MIN_ASSAY_SAMPLES);
    let mut right = Vec::with_capacity(MIN_ASSAY_SAMPLES);
    let mut labels = Vec::with_capacity(MIN_ASSAY_SAMPLES);
    for i in 0..MIN_ASSAY_SAMPLES {
        let label = i % 2 == 0;
        labels.push(label);
        let left_value = if label {
            if i % 4 == 0 { -0.2 } else { 1.0 }
        } else if i % 4 == 1 {
            0.2
        } else {
            -1.0
        };
        let right_value = if label {
            if i % 4 == 2 { -0.2 } else { 1.0 }
        } else if i % 4 == 3 {
            0.2
        } else {
            -1.0
        };
        left.push(vec![left_value]);
        right.push(vec![right_value]);
    }
    (left, right, labels)
}

pub(crate) fn block_redundancy_matrix(size: usize, block: usize) -> Vec<Vec<f32>> {
    (0..size)
        .map(|row| {
            (0..size)
                .map(|col| if row / block == col / block { 1.0 } else { 0.0 })
                .collect()
        })
        .collect()
}
