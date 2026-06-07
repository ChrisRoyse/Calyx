use calyx_core::{Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};

use crate::frozen::FrozenLensContract;
use crate::lens::ensure_input_modality;

const BYTE_FEATURE_DIM: u32 = 16;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Deterministic, data-local feature encoders with no model weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlgorithmicEncoder {
    /// Byte and character-class features for text/code/structured inputs.
    ByteFeatures,
}

impl AlgorithmicEncoder {
    /// Returns the dense output dimension.
    pub const fn dim(self) -> u32 {
        match self {
            Self::ByteFeatures => BYTE_FEATURE_DIM,
        }
    }
}

/// A frozen algorithmic lens.
#[derive(Clone, Debug)]
pub struct AlgorithmicLens {
    id: LensId,
    modality: Modality,
    encoder: AlgorithmicEncoder,
}

impl AlgorithmicLens {
    /// Creates an algorithmic byte-feature lens.
    pub fn byte_features(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(name, modality, AlgorithmicEncoder::ByteFeatures)
    }

    /// Creates an algorithmic lens from an encoder.
    pub fn new(name: impl Into<String>, modality: Modality, encoder: AlgorithmicEncoder) -> Self {
        let name = name.into();
        let id = match encoder {
            AlgorithmicEncoder::ByteFeatures => {
                FrozenLensContract::algorithmic_byte_features(&name, modality).lens_id()
            }
        };
        Self {
            id,
            modality,
            encoder,
        }
    }
}

impl Lens for AlgorithmicLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.encoder.dim())
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        ensure_input_modality(self, input)?;
        Ok(SlotVector::Dense {
            dim: self.encoder.dim(),
            data: match self.encoder {
                AlgorithmicEncoder::ByteFeatures => byte_features(&input.bytes),
            },
        })
    }
}

fn byte_features(bytes: &[u8]) -> Vec<f32> {
    let mut out = vec![0.0_f32; BYTE_FEATURE_DIM as usize];
    if bytes.is_empty() {
        out[0] = 1.0;
        return out;
    }

    let mut ascii = 0_u32;
    let mut whitespace = 0_u32;
    let mut alphabetic = 0_u32;
    let mut digits = 0_u32;
    let mut punctuation = 0_u32;
    let mut uppercase = 0_u32;
    let mut lowercase = 0_u32;
    let mut control = 0_u32;
    let mut nul = 0_u32;
    let mut path = 0_u32;
    let mut brackets = 0_u32;
    let mut newline = 0_u32;
    let mut byte_sum = 0_u64;
    let mut hash = FNV_OFFSET;

    for &byte in bytes {
        byte_sum += u64::from(byte);
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
        ascii += byte.is_ascii() as u32;
        whitespace += byte.is_ascii_whitespace() as u32;
        alphabetic += byte.is_ascii_alphabetic() as u32;
        digits += byte.is_ascii_digit() as u32;
        punctuation += byte.is_ascii_punctuation() as u32;
        uppercase += byte.is_ascii_uppercase() as u32;
        lowercase += byte.is_ascii_lowercase() as u32;
        control += byte.is_ascii_control() as u32;
        nul += (byte == 0) as u32;
        path += matches!(byte, b'/' | b'\\') as u32;
        brackets += matches!(byte, b'{' | b'}' | b'(' | b')' | b'[' | b']') as u32;
        newline += matches!(byte, b'\n' | b'\r') as u32;
    }

    let len = bytes.len().min(u32::MAX as usize) as f32;
    let inv_len = 1.0 / len.max(1.0);
    out[0] = len.log2().max(0.0) / 32.0;
    out[1] = ascii as f32 * inv_len;
    out[2] = whitespace as f32 * inv_len;
    out[3] = alphabetic as f32 * inv_len;
    out[4] = digits as f32 * inv_len;
    out[5] = punctuation as f32 * inv_len;
    out[6] = uppercase as f32 * inv_len;
    out[7] = lowercase as f32 * inv_len;
    out[8] = control as f32 * inv_len;
    out[9] = nul as f32 * inv_len;
    out[10] = path as f32 * inv_len;
    out[11] = brackets as f32 * inv_len;
    out[12] = newline as f32 * inv_len;
    out[13] = byte_sum as f32 / (len * 255.0);
    out[14] = hash_part((hash & 0xffff_ffff) as u32);
    out[15] = hash_part((hash >> 32) as u32);
    out
}

fn hash_part(value: u32) -> f32 {
    (value as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_features_are_bit_deterministic() {
        let lens = AlgorithmicLens::byte_features("byte-fsv", Modality::Text);
        let input = Input::new(Modality::Text, b"Calyx PH17: 2+2=4\n".to_vec());

        let first = lens.measure(&input).unwrap();
        let second = lens.measure(&input).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn empty_input_emits_real_dense_vector() {
        let lens = AlgorithmicLens::byte_features("byte-empty", Modality::Text);
        let input = Input::new(Modality::Text, Vec::new());
        let vector = lens.measure(&input).unwrap();
        let bytes = serde_json::to_vec(&vector).unwrap();

        println!(
            "ALGORITHMIC_EMPTY_BYTES={}",
            String::from_utf8_lossy(&bytes)
        );
        assert_eq!(
            vector,
            SlotVector::Dense {
                dim: BYTE_FEATURE_DIM,
                data: {
                    let mut data = vec![0.0; BYTE_FEATURE_DIM as usize];
                    data[0] = 1.0;
                    data
                }
            }
        );
    }

    #[test]
    fn algorithmic_fsv_determinism_probe() {
        let lens = AlgorithmicLens::byte_features("byte-fsv", Modality::Text);
        let input = Input::new(Modality::Text, b"Calyx registry manual FSV".to_vec());
        let first = lens.measure(&input).unwrap();
        let second = lens.measure(&input).unwrap();
        let first_bytes = serde_json::to_vec(&first).unwrap();
        let second_bytes = serde_json::to_vec(&second).unwrap();

        println!("ALGORITHMIC_FSV_DIGEST={}", digest_hex(&first_bytes));
        println!(
            "ALGORITHMIC_FSV_BYTES={}",
            String::from_utf8_lossy(&first_bytes)
        );
        assert_eq!(first_bytes, second_bytes);
    }

    fn digest_hex(bytes: &[u8]) -> String {
        calyx_core::content_address([bytes])
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
