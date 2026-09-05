#![cfg_attr(not(sextant_cuvs), allow(dead_code))]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use calyx_core::Result;

use super::build::DiskAnnBuildMetric;
use crate::error::{CALYX_INDEX_CORRUPT, CALYX_INDEX_IO, sextant_error};

const MAGIC: &[u8; 8] = b"CALYXCSD";
const VERSION: u32 = 1;
const HEADER_LEN: usize = 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DatasetDtype {
    I8 = 1,
    F32 = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DatasetMetric {
    UnitL2 = 1,
    RawL2 = 2,
}

#[derive(Clone, Debug)]
pub(super) struct DatasetHeader {
    pub(super) dtype: DatasetDtype,
    pub(super) metric: DatasetMetric,
    pub(super) rows: usize,
    pub(super) dim: usize,
    pub(super) payload_digest: [u8; 32],
}

pub(super) enum DatasetPayload {
    I8(DatasetHeader, Vec<i8>),
    F32(DatasetHeader, Vec<f32>),
}

pub(super) fn prepare(
    final_path: &Path,
    rows: &[Vec<f32>],
    metric: DiskAnnBuildMetric,
) -> Result<PathBuf> {
    let dim = rows.first().map_or(0, Vec::len);
    if rows.is_empty() || dim == 0 || rows.iter().any(|row| row.len() != dim) {
        return Err(corrupt(
            "CAGRA serving dataset requires a non-empty rectangular matrix",
        ));
    }
    let use_i8 = metric == DiskAnnBuildMetric::RawL2
        && rows.iter().flatten().all(|value| lossless_i8(*value));
    let (dtype, payload) = if use_i8 {
        (
            DatasetDtype::I8,
            rows.iter()
                .flatten()
                .map(|value| (*value as i8) as u8)
                .collect::<Vec<_>>(),
        )
    } else {
        let mut payload = Vec::with_capacity(rows.len().saturating_mul(dim).saturating_mul(4));
        for value in rows.iter().flatten() {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        (DatasetDtype::F32, payload)
    };
    let metric = match metric {
        DiskAnnBuildMetric::UnitL2 => DatasetMetric::UnitL2,
        DiskAnnBuildMetric::RawL2 => DatasetMetric::RawL2,
    };
    let digest = *blake3::hash(&payload).as_bytes();
    let header = DatasetHeader {
        dtype,
        metric,
        rows: rows.len(),
        dim,
        payload_digest: digest,
    };
    let tmp = final_path.with_extension("cagra-data.tmp");
    if let Some(parent) = tmp.parent()
        && !parent.as_os_str().is_empty()
    {
        calyx_aster::private_fs::create_private_dir_all(parent)?;
    }
    remove_if_present(&tmp)?;
    // Owner-only (0600) at creation (#1989).
    let mut file = calyx_aster::private_fs::open_private_temp_file(&tmp)?;
    file.write_all(&encode_header(&header))
        .and_then(|()| file.write_all(&payload))
        .and_then(|()| file.sync_all())
        .map_err(|error| io("write dataset asset", error))?;
    Ok(tmp)
}

pub(super) fn read_header(path: &Path) -> Result<DatasetHeader> {
    let mut file = File::open(path).map_err(|error| io("open dataset asset", error))?;
    let mut bytes = [0_u8; HEADER_LEN];
    file.read_exact(&mut bytes)
        .map_err(|error| io("read dataset asset header", error))?;
    decode_header(path, &bytes)
}

pub(super) fn load(path: &Path) -> Result<DatasetPayload> {
    let header = read_header(path)?;
    let values = header
        .rows
        .checked_mul(header.dim)
        .ok_or_else(|| corrupt("CAGRA serving dataset shape overflow"))?;
    let element_bytes = match header.dtype {
        DatasetDtype::I8 => 1,
        DatasetDtype::F32 => 4,
    };
    let payload_len = values
        .checked_mul(element_bytes)
        .ok_or_else(|| corrupt("CAGRA serving dataset byte length overflow"))?;
    let expected_len = HEADER_LEN
        .checked_add(payload_len)
        .ok_or_else(|| corrupt("CAGRA serving dataset file length overflow"))?;
    let actual_len = usize::try_from(
        path.metadata()
            .map_err(|error| io("stat dataset asset", error))?
            .len(),
    )
    .map_err(|_| corrupt("CAGRA serving dataset file exceeds usize"))?;
    if actual_len != expected_len {
        return Err(corrupt(format!(
            "CAGRA serving dataset length {actual_len} != expected {expected_len}"
        )));
    }
    let mut file = File::open(path).map_err(|error| io("open dataset payload", error))?;
    let mut discard = [0_u8; HEADER_LEN];
    file.read_exact(&mut discard)
        .and_then(|()| {
            let mut payload = vec![0_u8; payload_len];
            file.read_exact(&mut payload).map(|()| payload)
        })
        .map_err(|error| io("read dataset payload", error))
        .and_then(|payload| decode_payload(header, payload))
}

fn decode_payload(header: DatasetHeader, payload: Vec<u8>) -> Result<DatasetPayload> {
    if blake3::hash(&payload).as_bytes() != &header.payload_digest {
        return Err(corrupt("CAGRA serving dataset payload digest mismatch"));
    }
    match header.dtype {
        DatasetDtype::I8 => Ok(DatasetPayload::I8(
            header,
            payload.into_iter().map(|value| value as i8).collect(),
        )),
        DatasetDtype::F32 => {
            let values = payload
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four-byte chunk")))
                .collect();
            Ok(DatasetPayload::F32(header, values))
        }
    }
}

fn encode_header(header: &DatasetHeader) -> [u8; HEADER_LEN] {
    let mut out = [0_u8; HEADER_LEN];
    out[..8].copy_from_slice(MAGIC);
    out[8..12].copy_from_slice(&VERSION.to_le_bytes());
    out[12] = header.dtype as u8;
    out[13] = header.metric as u8;
    out[16..24].copy_from_slice(&(header.rows as u64).to_le_bytes());
    out[24..28].copy_from_slice(&(header.dim as u32).to_le_bytes());
    out[28..60].copy_from_slice(&header.payload_digest);
    out
}

fn decode_header(path: &Path, bytes: &[u8; HEADER_LEN]) -> Result<DatasetHeader> {
    if &bytes[..8] != MAGIC || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != VERSION {
        return Err(corrupt(format!(
            "invalid CAGRA serving dataset header {}",
            path.display()
        )));
    }
    let dtype = match bytes[12] {
        1 => DatasetDtype::I8,
        2 => DatasetDtype::F32,
        value => return Err(corrupt(format!("unknown dataset dtype {value}"))),
    };
    let metric = match bytes[13] {
        1 => DatasetMetric::UnitL2,
        2 => DatasetMetric::RawL2,
        value => return Err(corrupt(format!("unknown dataset metric {value}"))),
    };
    let rows = usize::try_from(u64::from_le_bytes(bytes[16..24].try_into().unwrap()))
        .map_err(|_| corrupt("dataset rows exceed usize"))?;
    let dim = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    if rows == 0 || dim == 0 {
        return Err(corrupt("CAGRA serving dataset has an empty shape"));
    }
    let mut payload_digest = [0_u8; 32];
    payload_digest.copy_from_slice(&bytes[28..60]);
    Ok(DatasetHeader {
        dtype,
        metric,
        rows,
        dim,
        payload_digest,
    })
}

fn lossless_i8(value: f32) -> bool {
    value.is_finite() && (-128.0..=127.0).contains(&value) && value.fract() == 0.0
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io("remove stale temporary dataset asset", error)),
    }
}

fn corrupt(detail: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_CORRUPT, detail)
}

fn io(stage: &'static str, error: std::io::Error) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_IO, format!("{stage}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn raw_integral_dataset_roundtrips_as_i8() {
        let root = temp_root("i8");
        let path = root.join("graph.cagra-data");
        let tmp = prepare(
            &path,
            &[vec![-128.0, 0.0], vec![127.0, 42.0]],
            DiskAnnBuildMetric::RawL2,
        )
        .expect("prepare i8 dataset");
        fs::rename(tmp, &path).expect("publish dataset");
        match load(&path).expect("load i8 dataset") {
            DatasetPayload::I8(header, values) => {
                assert_eq!((header.rows, header.dim), (2, 2));
                assert_eq!(values, [-128, 0, 127, 42]);
            }
            DatasetPayload::F32(_, _) => panic!("integral raw-L2 dataset must be compact i8"),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unit_dataset_roundtrips_as_f32_and_detects_corruption() {
        let root = temp_root("f32");
        let path = root.join("graph.cagra-data");
        let tmp = prepare(
            &path,
            &[vec![0.25, 0.75], vec![0.5, -0.5]],
            DiskAnnBuildMetric::UnitL2,
        )
        .expect("prepare f32 dataset");
        fs::rename(tmp, &path).expect("publish dataset");
        assert!(matches!(
            load(&path).expect("load f32 dataset"),
            DatasetPayload::F32(_, _)
        ));
        let mut bytes = fs::read(&path).expect("read dataset");
        *bytes.last_mut().expect("payload byte") ^= 1;
        fs::write(&path, bytes).expect("corrupt dataset");
        assert!(load(&path).is_err());
        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("calyx-cagra-dataset-{tag}-{nanos}"));
        fs::create_dir_all(&root).expect("create temp root");
        root
    }
}
