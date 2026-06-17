use std::fs::File;
use std::io::{BufWriter, Write as _};

use calyx_core::Result;

use super::{FORMAT_VERSION, SPANN_CENTROID_MAGIC, SpannCentroidIndex, corrupt, io};

pub(super) fn write_header(out: &mut BufWriter<File>, index: &SpannCentroidIndex) -> Result<()> {
    out.write_all(&SPANN_CENTROID_MAGIC)
        .map_err(|e| io("write centroid magic", e))?;
    out.write_all(&FORMAT_VERSION.to_le_bytes())
        .map_err(|e| io("write centroid version", e))?;
    out.write_all(&index.dim.to_le_bytes())
        .map_err(|e| io("write centroid dim", e))?;
    write_u64(out, index.centroids.len() as u64, "write centroid count")?;
    write_u64(
        out,
        index.posting_list_offsets.len() as u64,
        "write offset count",
    )?;
    write_u64(
        out,
        index.assignments.len() as u64,
        "write assignment count",
    )
}

pub(super) fn decode_centroids(bytes: &[u8]) -> Result<SpannCentroidIndex> {
    if bytes.len() < 40 {
        return Err(corrupt(format!("centroid file is {} B", bytes.len())));
    }
    if bytes[0..8] != SPANN_CENTROID_MAGIC {
        return Err(corrupt(format!("bad magic {:02x?}", &bytes[0..8])));
    }
    let mut cursor = 8;
    let version = read_u32(bytes, &mut cursor)?;
    if version != FORMAT_VERSION {
        return Err(corrupt(format!("format_version {version}")));
    }
    let dim = read_u32(bytes, &mut cursor)?;
    let centroid_count = read_u64(bytes, &mut cursor)? as usize;
    let offset_count = read_u64(bytes, &mut cursor)? as usize;
    let assignment_count = read_u64(bytes, &mut cursor)? as usize;
    let mut centroids = Vec::with_capacity(centroid_count);
    for _ in 0..centroid_count {
        let mut centroid = Vec::with_capacity(dim as usize);
        for _ in 0..dim {
            centroid.push(f32::from_le_bytes(read_exact::<4>(bytes, &mut cursor)?));
        }
        centroids.push(centroid);
    }
    let mut offsets = Vec::with_capacity(offset_count);
    for _ in 0..offset_count {
        offsets.push(read_u64(bytes, &mut cursor)?);
    }
    let mut assignments = Vec::with_capacity(assignment_count);
    for _ in 0..assignment_count {
        assignments.push((read_u32(bytes, &mut cursor)?, read_u32(bytes, &mut cursor)?));
    }
    if cursor != bytes.len() {
        return Err(corrupt(format!(
            "{} trailing centroid bytes",
            bytes.len() - cursor
        )));
    }
    SpannCentroidIndex::from_parts(dim, centroids, offsets, assignments)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    Ok(u32::from_le_bytes(read_exact::<4>(bytes, cursor)?))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64> {
    Ok(u64::from_le_bytes(read_exact::<8>(bytes, cursor)?))
}

fn read_exact<const N: usize>(bytes: &[u8], cursor: &mut usize) -> Result<[u8; N]> {
    let end = cursor.saturating_add(N);
    let slice = bytes
        .get(*cursor..end)
        .ok_or_else(|| corrupt("truncated centroid file"))?;
    *cursor = end;
    Ok(slice.try_into().expect("exact len"))
}

fn write_u64(out: &mut BufWriter<File>, value: u64, stage: &str) -> Result<()> {
    out.write_all(&value.to_le_bytes())
        .map_err(|e| io(stage, e))
}
