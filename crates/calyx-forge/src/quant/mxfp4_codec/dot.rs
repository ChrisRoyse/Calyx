use super::MXFP4_BLOCK_BYTES;
use crate::mxfp4::{MXFP4_BLOCK_SIZE, MXFP4_PACKED_BYTES, MxFp4Block, decode_mxfp4_block};
use crate::mxfp8::{MXFP8_BLOCK_BYTES, MXFP8_BLOCK_SIZE, MxFp8Block, decode_mxfp8_block};

pub(super) fn dot_mxfp4_payload(left: &[u8], right: &[u8], dim: usize) -> f32 {
    let mut dot = 0.0;
    let mut remaining = dim;
    for (left_chunk, right_chunk) in left
        .chunks_exact(MXFP4_BLOCK_BYTES)
        .zip(right.chunks_exact(MXFP4_BLOCK_BYTES))
    {
        let left_block = mxfp4_block_from_chunk(left_chunk);
        let right_block = mxfp4_block_from_chunk(right_chunk);
        let left_decoded = decode_mxfp4_block(&left_block);
        let right_decoded = decode_mxfp4_block(&right_block);
        let count = remaining.min(MXFP4_BLOCK_SIZE);
        dot += dot_decoded_block(&left_decoded, &right_decoded, count);
        remaining -= count;
    }
    dot
}

pub(super) fn dot_mxfp8_payload(left: &[u8], right: &[u8], dim: usize) -> f32 {
    let mut dot = 0.0;
    let mut remaining = dim;
    for (left_chunk, right_chunk) in left
        .chunks_exact(MXFP8_BLOCK_BYTES)
        .zip(right.chunks_exact(MXFP8_BLOCK_BYTES))
    {
        let left_block = mxfp8_block_from_chunk(left_chunk);
        let right_block = mxfp8_block_from_chunk(right_chunk);
        let left_decoded = decode_mxfp8_block(&left_block);
        let right_decoded = decode_mxfp8_block(&right_block);
        let count = remaining.min(MXFP8_BLOCK_SIZE);
        dot += dot_decoded_block(&left_decoded, &right_decoded, count);
        remaining -= count;
    }
    dot
}

pub(super) fn dot_mxfp4_mxfp8_payload(left: &[u8], right: &[u8], dim: usize) -> f32 {
    let mut dot = 0.0;
    let mut remaining = dim;
    for (left_chunk, right_chunk) in left
        .chunks_exact(MXFP4_BLOCK_BYTES)
        .zip(right.chunks_exact(MXFP8_BLOCK_BYTES))
    {
        let left_block = mxfp4_block_from_chunk(left_chunk);
        let right_block = mxfp8_block_from_chunk(right_chunk);
        let left_decoded = decode_mxfp4_block(&left_block);
        let right_decoded = decode_mxfp8_block(&right_block);
        let count = remaining.min(MXFP4_BLOCK_SIZE);
        dot += dot_decoded_block(&left_decoded, &right_decoded, count);
        remaining -= count;
    }
    dot
}

fn dot_decoded_block<const N: usize>(left: &[f32; N], right: &[f32; N], count: usize) -> f32 {
    left[..count]
        .iter()
        .zip(right[..count].iter())
        .map(|(lhs, rhs)| lhs * rhs)
        .sum()
}

fn mxfp4_block_from_chunk(chunk: &[u8]) -> MxFp4Block {
    let mut codes = [0; MXFP4_PACKED_BYTES];
    codes.copy_from_slice(&chunk[..MXFP4_PACKED_BYTES]);
    MxFp4Block {
        codes,
        scale_e8m0: chunk[MXFP4_PACKED_BYTES],
    }
}

fn mxfp8_block_from_chunk(chunk: &[u8]) -> MxFp8Block {
    let mut codes = [0; MXFP8_BLOCK_SIZE];
    codes.copy_from_slice(&chunk[..MXFP8_BLOCK_SIZE]);
    MxFp8Block {
        codes,
        scale_e8m0: chunk[MXFP8_BLOCK_SIZE],
    }
}
