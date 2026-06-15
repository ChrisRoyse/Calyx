use calyx_aster::vault::encode;
use calyx_core::{CalyxError, CxId, QuantPolicy, Result, Slot, SlotVector};
use calyx_forge::{
    BinaryCodec, MxFp4Codec, QuantLevel, QuantizedVec, Quantizer, TurboQuantCodec, new_seed,
};

use super::recall::prepare_dense;
use super::{
    CALYX_VECTOR_COMPRESSION_INVALID, COMPRESSED_SLOT_TAG, COMPRESSED_SLOT_VERSION,
    StoredSlotCodec, StoredSlotEnvelope, compression_error,
};
use crate::spec::LensSpec;

#[derive(Clone)]
pub(super) struct EncodedRow {
    pub(super) cx_id: CxId,
    pub(super) prepared: Vec<f32>,
    pub(super) decoded: Vec<f32>,
    pub(super) raw_bytes: Vec<u8>,
    pub(super) stored_bytes: Vec<u8>,
    pub(super) codec: StoredSlotCodec,
}

pub(super) fn encode_rows(
    slot: &Slot,
    lens: &LensSpec,
    rows: &[(CxId, Vec<f32>)],
    policy: QuantPolicy,
    fallback_reason: Option<String>,
) -> Result<Vec<EncodedRow>> {
    rows.iter()
        .map(|(cx_id, raw)| {
            let prepared = prepare_dense(raw, lens.truncate_dim)?;
            let raw_bytes = raw_bytes(raw)?;
            let encoded = encode_prepared(slot, lens, *cx_id, &prepared, policy)?;
            let stored_bytes = if encoded.codec == StoredSlotCodec::RawF32 {
                raw_bytes.clone()
            } else {
                encode_envelope(
                    encoded.codec,
                    &encoded.qv,
                    raw.len() as u32,
                    lens.truncate_dim,
                    fallback_reason.is_some(),
                )
            };
            Ok(EncodedRow {
                cx_id: *cx_id,
                prepared,
                decoded: encoded.decoded,
                raw_bytes,
                stored_bytes,
                codec: encoded.codec,
            })
        })
        .collect()
}

pub(super) fn fallback_policy(policy: QuantPolicy) -> QuantPolicy {
    match policy {
        QuantPolicy::TurboQuant {
            bits_per_channel_x2: 5,
        }
        | QuantPolicy::Binary
        | QuantPolicy::Pq { .. } => QuantPolicy::turboquant_default(),
        QuantPolicy::MxFp4 => QuantPolicy::Float8,
        QuantPolicy::TurboQuant { .. } | QuantPolicy::Float8 | QuantPolicy::None => {
            QuantPolicy::None
        }
    }
}

pub fn decode_stored_slot_envelope(bytes: &[u8]) -> Result<StoredSlotEnvelope> {
    if bytes.first().copied() != Some(COMPRESSED_SLOT_TAG) {
        return Ok(StoredSlotEnvelope {
            codec: StoredSlotCodec::RawF32,
            level: "f32".to_string(),
            raw_dim: 0,
            stored_dim: 0,
            fallback: false,
            truncated: false,
            payload_bytes: bytes.len(),
        });
    }
    if bytes.len() < 53 {
        return Err(compression_error(
            CALYX_VECTOR_COMPRESSION_INVALID,
            "compressed slot envelope too short",
        ));
    }
    let version = bytes[1];
    if version != COMPRESSED_SLOT_VERSION {
        return Err(compression_error(
            CALYX_VECTOR_COMPRESSION_INVALID,
            format!("unsupported compressed slot version {version}"),
        ));
    }
    let codec = decode_codec(bytes[2])?;
    let level = decode_level(bytes[3])?;
    let raw_dim = u32::from_be_bytes(bytes[4..8].try_into().unwrap());
    let stored_dim = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    let flags = bytes[12];
    let payload_len = u32::from_be_bytes(bytes[49..53].try_into().unwrap()) as usize;
    if bytes.len() != 53 + payload_len {
        return Err(compression_error(
            CALYX_VECTOR_COMPRESSION_INVALID,
            "compressed slot payload length mismatch",
        ));
    }
    Ok(StoredSlotEnvelope {
        codec,
        level: format!("{level:?}"),
        raw_dim,
        stored_dim,
        fallback: flags & 1 == 1,
        truncated: flags & 2 == 2,
        payload_bytes: payload_len,
    })
}

struct EncodedPrepared {
    qv: QuantizedVec,
    decoded: Vec<f32>,
    codec: StoredSlotCodec,
}

fn encode_prepared(
    slot: &Slot,
    lens: &LensSpec,
    cx_id: CxId,
    prepared: &[f32],
    policy: QuantPolicy,
) -> Result<EncodedPrepared> {
    match policy {
        QuantPolicy::None => Ok(EncodedPrepared {
            qv: raw_qv(prepared),
            decoded: prepared.to_vec(),
            codec: StoredSlotCodec::RawF32,
        }),
        QuantPolicy::TurboQuant {
            bits_per_channel_x2,
        } => encode_turbo(lens, cx_id, prepared, bits_per_channel_x2),
        QuantPolicy::MxFp4 => encode_mxfp(slot, prepared, true),
        QuantPolicy::Float8 => encode_mxfp(slot, prepared, false),
        QuantPolicy::Binary => encode_binary(lens, cx_id, prepared),
        QuantPolicy::Pq { .. } => encode_turbo(lens, cx_id, prepared, 7),
    }
}

fn encode_turbo(
    lens: &LensSpec,
    cx_id: CxId,
    prepared: &[f32],
    bits_per_channel_x2: u8,
) -> Result<EncodedPrepared> {
    let level = match bits_per_channel_x2 {
        7 => QuantLevel::Bits3p5,
        5 => QuantLevel::Bits2p5,
        other => {
            return Err(compression_error(
                CALYX_VECTOR_COMPRESSION_INVALID,
                format!("unsupported TurboQuant bits_per_channel_x2 {other}"),
            ));
        }
    };
    let seed = new_seed(prepared.len(), &seed_entropy(lens, cx_id));
    let codec = TurboQuantCodec::new(seed, level).map_err(forge_error)?;
    let qv = codec.encode(prepared).map_err(forge_error)?;
    let decoded = codec.decode(&qv).map_err(forge_error)?;
    Ok(EncodedPrepared {
        qv,
        decoded,
        codec: if level == QuantLevel::Bits2p5 {
            StoredSlotCodec::TurboQuantBits2p5
        } else {
            StoredSlotCodec::TurboQuantBits3p5
        },
    })
}

fn encode_mxfp(slot: &Slot, prepared: &[f32], assay_safe: bool) -> Result<EncodedPrepared> {
    let codec = MxFp4Codec::new(prepared.len());
    let qv = codec
        .encode_assay_checked(slot.slot_key.key(), prepared, assay_safe)
        .map_err(forge_error)?;
    let decoded = codec.decode(&qv).map_err(forge_error)?;
    Ok(EncodedPrepared {
        codec: if qv.level == QuantLevel::Bits4Fp {
            StoredSlotCodec::MxFp4
        } else {
            StoredSlotCodec::MxFp8
        },
        qv,
        decoded,
    })
}

fn encode_binary(lens: &LensSpec, cx_id: CxId, prepared: &[f32]) -> Result<EncodedPrepared> {
    let seed = new_seed(prepared.len(), &seed_entropy(lens, cx_id));
    let codec = BinaryCodec::new(seed).map_err(forge_error)?;
    let qv = codec.encode(prepared).map_err(forge_error)?;
    let decoded = codec.decode(&qv).map_err(forge_error)?;
    Ok(EncodedPrepared {
        qv,
        decoded,
        codec: StoredSlotCodec::Binary,
    })
}

fn raw_bytes(raw: &[f32]) -> Result<Vec<u8>> {
    encode::encode_slot_vector(&SlotVector::Dense {
        dim: raw.len() as u32,
        data: raw.to_vec(),
    })
}

fn raw_qv(prepared: &[f32]) -> QuantizedVec {
    QuantizedVec {
        level: QuantLevel::F32,
        dim: prepared.len(),
        bytes: Vec::new(),
        scale: 1.0,
        seed_id: [0; 32],
    }
}

fn encode_envelope(
    codec: StoredSlotCodec,
    qv: &QuantizedVec,
    raw_dim: u32,
    truncate_dim: Option<u32>,
    fallback: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(49 + qv.bytes.len());
    out.push(COMPRESSED_SLOT_TAG);
    out.push(COMPRESSED_SLOT_VERSION);
    out.push(codec_code(codec));
    out.push(level_code(qv.level));
    out.extend_from_slice(&raw_dim.to_be_bytes());
    out.extend_from_slice(&(qv.dim as u32).to_be_bytes());
    out.push(u8::from(fallback) | (u8::from(truncate_dim.is_some()) << 1));
    out.extend_from_slice(&qv.scale.to_bits().to_be_bytes());
    out.extend_from_slice(&qv.seed_id);
    out.extend_from_slice(&(qv.bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(&qv.bytes);
    out
}

fn seed_entropy(lens: &LensSpec, cx_id: CxId) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(lens.lens_id().as_bytes());
    hasher.update(cx_id.as_bytes());
    *hasher.finalize().as_bytes()
}

fn codec_code(codec: StoredSlotCodec) -> u8 {
    match codec {
        StoredSlotCodec::RawF32 => 0,
        StoredSlotCodec::TurboQuantBits3p5 => 1,
        StoredSlotCodec::TurboQuantBits2p5 => 2,
        StoredSlotCodec::MxFp4 => 3,
        StoredSlotCodec::MxFp8 => 4,
        StoredSlotCodec::Binary => 5,
    }
}

fn decode_codec(code: u8) -> Result<StoredSlotCodec> {
    match code {
        0 => Ok(StoredSlotCodec::RawF32),
        1 => Ok(StoredSlotCodec::TurboQuantBits3p5),
        2 => Ok(StoredSlotCodec::TurboQuantBits2p5),
        3 => Ok(StoredSlotCodec::MxFp4),
        4 => Ok(StoredSlotCodec::MxFp8),
        5 => Ok(StoredSlotCodec::Binary),
        _ => Err(compression_error(
            CALYX_VECTOR_COMPRESSION_INVALID,
            format!("unknown stored slot codec code {code}"),
        )),
    }
}

fn level_code(level: QuantLevel) -> u8 {
    match level {
        QuantLevel::F32 => 0,
        QuantLevel::Bits8 => 1,
        QuantLevel::Bits8Fp => 2,
        QuantLevel::Bits4Fp => 3,
        QuantLevel::Bits3p5 => 4,
        QuantLevel::Bits2p5 => 5,
        QuantLevel::Bits1 => 6,
    }
}

fn decode_level(code: u8) -> Result<QuantLevel> {
    match code {
        0 => Ok(QuantLevel::F32),
        1 => Ok(QuantLevel::Bits8),
        2 => Ok(QuantLevel::Bits8Fp),
        3 => Ok(QuantLevel::Bits4Fp),
        4 => Ok(QuantLevel::Bits3p5),
        5 => Ok(QuantLevel::Bits2p5),
        6 => Ok(QuantLevel::Bits1),
        _ => Err(compression_error(
            CALYX_VECTOR_COMPRESSION_INVALID,
            format!("unknown quant level code {code}"),
        )),
    }
}

fn forge_error(error: calyx_forge::ForgeError) -> CalyxError {
    CalyxError {
        code: error.code(),
        message: error.to_string(),
        remediation: super::COMPRESSION_REMEDIATION,
    }
}
