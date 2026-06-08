//! Per-slot quantization policy for Sextant indexes.

use calyx_core::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantKind {
    None,
    Scalar8,
    Binary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuantConfig {
    pub kind: QuantKind,
    pub scale: f32,
    pub zero_point: i8,
    locked: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuantizedVector {
    pub kind: QuantKind,
    pub bytes: Vec<u8>,
    pub approx: Vec<f32>,
}

impl QuantConfig {
    pub const fn none() -> Self {
        Self {
            kind: QuantKind::None,
            scale: 1.0,
            zero_point: 0,
            locked: false,
        }
    }

    pub const fn scalar8(scale: f32) -> Self {
        Self {
            kind: QuantKind::Scalar8,
            scale,
            zero_point: 0,
            locked: false,
        }
    }

    pub fn lock_after_first_insert(&mut self) {
        self.locked = true;
    }

    pub const fn is_locked(&self) -> bool {
        self.locked
    }

    pub fn quantize(&self, values: &[f32]) -> QuantizedVector {
        match self.kind {
            QuantKind::None => QuantizedVector {
                kind: self.kind,
                bytes: Vec::new(),
                approx: values.to_vec(),
            },
            QuantKind::Scalar8 => {
                let scale = self.scale.max(1e-6);
                let mut bytes = Vec::with_capacity(values.len());
                let mut approx = Vec::with_capacity(values.len());
                for value in values {
                    let q = (value / scale).round().clamp(-127.0, 127.0) as i8;
                    bytes.push(q as u8);
                    approx.push(q as f32 * scale);
                }
                QuantizedVector {
                    kind: self.kind,
                    bytes,
                    approx,
                }
            }
            QuantKind::Binary => {
                let bytes = values.iter().map(|v| u8::from(*v >= 0.0)).collect();
                let approx = values
                    .iter()
                    .map(|v| if *v >= 0.0 { 1.0 } else { -1.0 })
                    .collect();
                QuantizedVector {
                    kind: self.kind,
                    bytes,
                    approx,
                }
            }
        }
    }

    pub fn cpu_gpu_delta(&self, _values: &[f32]) -> Result<f32> {
        Err(crate::error::sextant_error(
            crate::error::CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE,
            "QuantConfig has no wired Forge GPU quantization path; CPU/GPU delta is unavailable",
        ))
    }
}
