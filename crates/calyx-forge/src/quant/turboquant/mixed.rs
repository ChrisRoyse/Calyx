use crate::quant::QuantLevel;

use super::lloyd;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MixedCodes {
    pub(crate) codes: Vec<u8>,
    pub(crate) widths: Vec<u8>,
}

pub(crate) fn quantize(rotated: &[f32], scale: f32, level: QuantLevel) -> Vec<u8> {
    let mut out = vec![0; packed_len(rotated.len(), level)];
    if scale == 0.0 || rotated.is_empty() {
        return out;
    }
    let unit = (rotated.len() as f32).sqrt() / scale;
    let mut bit_offset = 0usize;
    for (idx, value) in rotated.iter().enumerate() {
        let width = lane_width(level, idx);
        let code = lloyd::quantize_bits(*value * unit, width);
        write_bits(&mut out, bit_offset, usize::from(width), code);
        bit_offset += usize::from(width);
    }
    out
}

pub(crate) fn unpack(bytes: &[u8], dim: usize, level: QuantLevel) -> MixedCodes {
    let mut codes = Vec::with_capacity(dim);
    let mut widths = Vec::with_capacity(dim);
    let mut bit_offset = 0usize;
    for idx in 0..dim {
        let width = lane_width(level, idx);
        codes.push(read_bits(bytes, bit_offset, usize::from(width)) as u8);
        widths.push(width);
        bit_offset += usize::from(width);
    }
    MixedCodes { codes, widths }
}

pub(crate) fn packed_len(dim: usize, level: QuantLevel) -> usize {
    scalar_bits(dim, level).div_ceil(8)
}

fn scalar_bits(dim: usize, level: QuantLevel) -> usize {
    (0..dim)
        .map(|idx| usize::from(lane_width(level, idx)))
        .sum()
}

fn lane_width(level: QuantLevel, idx: usize) -> u8 {
    let high_lane = idx.is_multiple_of(2);
    match (level, high_lane) {
        (QuantLevel::Bits3p5, true) => 3,
        (QuantLevel::Bits3p5, false) => 2,
        (QuantLevel::Bits2p5, true) => 2,
        (QuantLevel::Bits2p5, false) => 1,
        _ => unreachable!("TurboQuant level validated before mixed-width packing"),
    }
}

fn write_bits(out: &mut [u8], offset: usize, width: usize, value: u16) {
    for bit in 0..width {
        if ((value >> bit) & 1) == 1 {
            let absolute = offset + bit;
            out[absolute / 8] |= 1 << (absolute % 8);
        }
    }
}

fn read_bits(bytes: &[u8], offset: usize, width: usize) -> u16 {
    let mut value = 0u16;
    for bit in 0..width {
        let absolute = offset + bit;
        if ((bytes[absolute / 8] >> (absolute % 8)) & 1) == 1 {
            value |= 1 << bit;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_width_lengths_match_scalar_budget() {
        assert_eq!(packed_len(128, QuantLevel::Bits3p5), 40);
        assert_eq!(packed_len(128, QuantLevel::Bits2p5), 24);
        assert_eq!(packed_len(129, QuantLevel::Bits3p5), 41);
        assert_eq!(packed_len(129, QuantLevel::Bits2p5), 25);
    }

    #[test]
    fn mixed_width_unpack_recovers_codes_and_widths() {
        let bytes = quantize(&[0.0; 8], 0.0, QuantLevel::Bits3p5);
        let mixed = unpack(&bytes, 8, QuantLevel::Bits3p5);
        assert_eq!(mixed.widths, vec![3, 2, 3, 2, 3, 2, 3, 2]);
        assert_eq!(mixed.codes, vec![0; 8]);
    }
}
