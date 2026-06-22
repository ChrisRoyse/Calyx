#![allow(dead_code)]

use calyx_core::{CxId, SlotVector, content_address};

pub fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

pub fn cx_u8_fill(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

pub fn cx_u128_be(value: u128) -> CxId {
    CxId::from_bytes(value.to_be_bytes())
}

pub fn cx_usize_be(value: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..16].copy_from_slice(&(value as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

pub fn cx_u32_be(value: u32) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..16].copy_from_slice(&u64::from(value).to_be_bytes());
    CxId::from_bytes(bytes)
}

pub fn digest_hex(bytes: &[u8]) -> String {
    hex(&content_address([bytes]))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
