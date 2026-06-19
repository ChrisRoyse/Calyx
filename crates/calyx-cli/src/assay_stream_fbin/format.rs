use std::fs::File;
use std::io::{BufWriter, Write};

use serde::Serialize;

use crate::error::{CliError, CliResult};

use super::{io_error, local_error};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum VectorFormat {
    #[serde(rename = "fbin")]
    #[default]
    Fbin,
    #[serde(rename = "i8bin")]
    I8Bin,
}

impl VectorFormat {
    pub(crate) fn parse(value: &str) -> CliResult<Self> {
        match value {
            "fbin" => Ok(Self::Fbin),
            "i8bin" => Ok(Self::I8Bin),
            other => Err(CliError::usage(format!(
                "--vector-format must be fbin or i8bin, got {other}"
            ))),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fbin => "fbin",
            Self::I8Bin => "i8bin",
        }
    }

    pub(crate) fn dir_name(self) -> &'static str {
        match self {
            Self::Fbin => "fbin",
            Self::I8Bin => "i8bin",
        }
    }

    pub(crate) fn extension(self) -> &'static str {
        self.as_str()
    }

    pub(crate) fn storage_contract(self) -> &'static str {
        match self {
            Self::Fbin => "f32-row-major-calyx-fbin",
            Self::I8Bin => "per-row-directional-symmetric-int8-normalized-on-read",
        }
    }
}

pub(super) fn write_header(
    writer: &mut BufWriter<File>,
    format: VectorFormat,
    dim: usize,
    count: usize,
) -> CliResult {
    match format {
        VectorFormat::Fbin => write_fbin_header(writer, dim, count),
        VectorFormat::I8Bin => write_i8bin_header(writer, dim, count),
    }
}

pub(super) fn write_row(
    writer: &mut BufWriter<File>,
    format: VectorFormat,
    vector: &[f32],
) -> CliResult {
    match format {
        VectorFormat::Fbin => write_f32_row(writer, vector),
        VectorFormat::I8Bin => write_i8_row(writer, vector),
    }
}

fn write_fbin_header(writer: &mut BufWriter<File>, dim: usize, count: usize) -> CliResult {
    writer
        .write_all(&calyx_sextant::index::VEC_MAGIC)
        .map_err(io_error)?;
    writer
        .write_all(
            &u32::try_from(dim)
                .map_err(|_| CliError::usage("fbin dim exceeds u32"))?
                .to_le_bytes(),
        )
        .map_err(io_error)?;
    writer
        .write_all(
            &u64::try_from(count)
                .map_err(|_| CliError::usage("fbin count exceeds u64"))?
                .to_le_bytes(),
        )
        .map_err(io_error)
}

fn write_i8bin_header(writer: &mut BufWriter<File>, dim: usize, count: usize) -> CliResult {
    writer
        .write_all(
            &u32::try_from(count)
                .map_err(|_| CliError::usage("i8bin count exceeds u32"))?
                .to_le_bytes(),
        )
        .map_err(io_error)?;
    writer
        .write_all(
            &u32::try_from(dim)
                .map_err(|_| CliError::usage("i8bin dim exceeds u32"))?
                .to_le_bytes(),
        )
        .map_err(io_error)
}

fn write_f32_row(writer: &mut BufWriter<File>, vector: &[f32]) -> CliResult {
    for value in vector {
        writer.write_all(&value.to_le_bytes()).map_err(io_error)?;
    }
    Ok(())
}

fn write_i8_row(writer: &mut BufWriter<File>, vector: &[f32]) -> CliResult {
    let quantized = quantize_direction_i8(vector)?;
    let bytes = quantized
        .iter()
        .map(|value| *value as u8)
        .collect::<Vec<_>>();
    writer.write_all(&bytes).map_err(io_error)
}

fn quantize_direction_i8(vector: &[f32]) -> CliResult<Vec<i8>> {
    let max_abs = vector
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f32, f32::max);
    if !max_abs.is_finite() || max_abs == 0.0 {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_I8_ZERO_VECTOR",
            "cannot encode zero/non-finite vector as directional i8bin row",
            "inspect the lens output; i8bin rows require a finite non-zero direction",
        ));
    }
    let scale = 127.0 / max_abs;
    let row = vector
        .iter()
        .map(|value| (value * scale).round().clamp(-127.0, 127.0) as i8)
        .collect::<Vec<_>>();
    if row.iter().all(|value| *value == 0) {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_I8_ZERO_VECTOR",
            "i8bin quantization collapsed a vector to all zeros",
            "inspect vector magnitudes before trusting compressed scale output",
        ));
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i8_quantization_preserves_direction_bytes() {
        let row = quantize_direction_i8(&[0.5, -1.0, 0.0]).unwrap();

        assert_eq!(row, [64, -127, 0]);
    }

    #[test]
    fn i8_quantization_rejects_zero_vector() {
        let error = quantize_direction_i8(&[0.0, 0.0]).unwrap_err();

        assert_eq!(error.code(), "CALYX_FSV_ASSAY_STREAM_FBIN_I8_ZERO_VECTOR");
    }
}
