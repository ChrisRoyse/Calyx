#![no_main]

use calyx_core::{SlotVector, SparseEntry};
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let data = bounded(data);
    if let Ok(vector) = serde_json::from_slice::<SlotVector>(data) {
        validate_all_shapes(&vector);
    }
    if let Some(vector) = raw_vector(data) {
        validate_all_shapes(&vector);
    }
});

fn validate_all_shapes(vector: &SlotVector) {
    let _ = vector.validate_schema();
}

fn raw_vector(data: &[u8]) -> Option<SlotVector> {
    let kind = *data.first()?;
    let values = f32_values(&data[1..]);
    match kind % 3 {
        0 => Some(SlotVector::Dense {
            dim: values.len().min(8) as u32,
            data: values.into_iter().take(8).collect(),
        }),
        1 => Some(SlotVector::Sparse {
            dim: 8,
            entries: values
                .into_iter()
                .take(8)
                .enumerate()
                .map(|(idx, val)| SparseEntry {
                    idx: idx as u32,
                    val,
                })
                .collect(),
        }),
        _ => Some(SlotVector::Multi {
            token_dim: 4,
            tokens: values
                .chunks(4)
                .take(4)
                .filter(|chunk| !chunk.is_empty())
                .map(<[f32]>::to_vec)
                .collect(),
        }),
    }
}

fn f32_values(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(4)
        .take(32)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("chunk width")))
        .collect()
}

fn bounded(data: &[u8]) -> &[u8] {
    &data[..data.len().min(MAX_INPUT)]
}
