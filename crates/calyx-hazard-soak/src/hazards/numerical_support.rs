use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CxId, SlotId, SlotVector};
use calyx_forge::{BinaryCodec, Quantizer, RotationSeed, apply_inverse_rotation, new_seed};
use calyx_sextant::index::{DiskAnnSearch, DiskAnnSearchParams};
use calyx_sextant::{HnswIndex, SextantIndex};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::resource_support::err;

#[derive(serde::Serialize)]
pub struct DriftSummary {
    pub max_abs_error: f32,
    pub max_relative_error: f32,
    pub mean_abs_error: f32,
    pub min_ip_before: f32,
    pub max_ip_before: f32,
}

#[derive(serde::Serialize)]
pub struct MinBitDrift {
    pub code: String,
    pub true_ip: f32,
    pub binary_ip: f32,
    pub relative_error: f32,
}

pub fn drift_summary(rows: &[Vec<f32>], decoded: &[Vec<f32>]) -> Result<DriftSummary, String> {
    let mut max_abs = 0.0_f32;
    let mut max_rel = 0.0_f32;
    let mut sum_abs = 0.0_f32;
    let mut min_before = f32::INFINITY;
    let mut max_before = f32::NEG_INFINITY;
    for i in 0..rows.len() {
        let j = i ^ 1;
        let before = dot(&rows[i], &rows[j]);
        let after = dot(&decoded[i], &decoded[j]);
        let abs = (before - after).abs();
        max_abs = max_abs.max(abs);
        max_rel = max_rel.max(abs / before.abs().max(1e-6));
        sum_abs += abs;
        min_before = min_before.min(before);
        max_before = max_before.max(before);
    }
    Ok(DriftSummary {
        max_abs_error: max_abs,
        max_relative_error: max_rel,
        mean_abs_error: sum_abs / rows.len() as f32,
        min_ip_before: min_before,
        max_ip_before: max_before,
    })
}

pub fn recall_against_exact(
    exact_rows: &[Vec<f32>],
    ids: &[CxId],
    index_rows: &[Vec<f32>],
    dim: usize,
    k: usize,
) -> Result<f32, String> {
    let mut index = HnswIndex::new(SlotId::new(10), dim as u32, 59);
    for (seq, (id, row)) in ids.iter().zip(index_rows).enumerate() {
        index.insert(*id, dense(row), seq as u64 + 1).map_err(err)?;
    }
    let mut total = 0.0_f32;
    for query in exact_rows.iter().take(100) {
        let expected = brute_top_k(exact_rows, ids, query, k);
        let got = index
            .search(&dense(query), k, Some(128))
            .map_err(err)?
            .into_iter()
            .map(|hit| hit.cx_id)
            .collect::<Vec<_>>();
        total += got.iter().filter(|id| expected.contains(id)).count() as f32 / k as f32;
    }
    Ok(total / 100.0)
}

pub fn min_bit_contract_code(dim: usize, bound: f32, code: &str) -> Result<MinBitDrift, String> {
    let seed = new_seed(dim, b"ph59-h10-min-bits");
    let (left, right) = min_bit_edge_pair(&seed);
    let codec = BinaryCodec::new(seed).map_err(err)?;
    let lq = codec.encode(&left).map_err(err)?;
    let rq = codec.encode(&right).map_err(err)?;
    let before = dot(&left, &right);
    let after = codec.dot_estimate(&lq, &rq).map_err(err)?;
    let relative_error = (before - after).abs() / before.abs().max(1e-6);
    let code = if relative_error > bound {
        code.to_string()
    } else {
        "NO_ERROR".to_string()
    };
    Ok(MinBitDrift {
        code,
        true_ip: before,
        binary_ip: after,
        relative_error,
    })
}

pub fn fallback_hnsw_search(
    ids: &[CxId],
    rows: &[Vec<f32>],
    query: &[f32],
    k: usize,
) -> Result<Vec<calyx_sextant::IndexSearchHit>, String> {
    let mut index = HnswIndex::new(SlotId::new(12), query.len() as u32, 12);
    for (seq, (id, row)) in ids.iter().zip(rows).enumerate() {
        index.insert(*id, dense(row), seq as u64 + 1).map_err(err)?;
    }
    index.search(&dense(query), k, Some(64)).map_err(err)
}

pub fn paired_vectors(rows: usize, dim: usize) -> Vec<Vec<f32>> {
    (0..rows)
        .map(|i| {
            let pair = i / 2;
            let mut base = (0..dim)
                .map(|d| dense_signal(pair, d, 0.013, 0.021, 0.007))
                .collect::<Vec<_>>();
            normalize(&mut base);
            if i % 2 == 1 {
                let mut perturb = (0..dim)
                    .map(|d| dense_signal(pair + 97, d + 31, 0.019, 0.011, 0.005))
                    .collect::<Vec<_>>();
                normalize(&mut perturb);
                for (value, delta) in base.iter_mut().zip(perturb) {
                    *value = *value * 0.96 + delta * 0.28;
                }
            }
            normalize(&mut base);
            base
        })
        .collect()
}

pub fn ids(count: usize) -> Vec<CxId> {
    (0..count)
        .map(|idx| {
            let mut bytes = [0_u8; 16];
            bytes[8..].copy_from_slice(&(idx as u64).to_be_bytes());
            CxId::from_bytes(bytes)
        })
        .collect()
}

pub fn id_rows(ids: &[CxId], rows: &[Vec<f32>]) -> Vec<(CxId, Vec<f32>)> {
    ids.iter().copied().zip(rows.iter().cloned()).collect()
}

pub fn top_hit(search: &DiskAnnSearch, query: &[f32]) -> Result<CxId, String> {
    search
        .search(&dense(query), 1, Some(64))
        .map_err(err)?
        .first()
        .map(|hit| hit.cx_id)
        .ok_or_else(|| "DiskANN search returned no hits".to_string())
}

pub fn search_params() -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: 16,
        ef_search: 64,
        rescore_k: 32,
        rescore_from_raw: false,
    }
}

pub fn single_nan_vector(dim: usize) -> Vec<f32> {
    let mut values = paired_vectors(2, dim).remove(0);
    values[0] = f32::NAN;
    values
}

pub fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

pub fn vec_bytes(row: &[f32]) -> Vec<u8> {
    row.iter().flat_map(|value| value.to_le_bytes()).collect()
}

pub fn read_base_rows(router: &CfRouter, ids: &[CxId]) -> Result<Vec<Vec<f32>>, String> {
    ids.iter()
        .map(|id| {
            router
                .get(ColumnFamily::Base, id.as_bytes())
                .map_err(err)?
                .ok_or_else(|| format!("missing base row {id}"))
                .and_then(|bytes| decode_vec(&bytes))
        })
        .collect()
}

pub fn flip_8_bytes(path: &Path, offset: u64) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(err)?;
    file.seek(SeekFrom::Start(offset)).map_err(err)?;
    let mut bytes = [0_u8; 8];
    file.read_exact(&mut bytes).map_err(err)?;
    for byte in &mut bytes {
        *byte ^= 0xFF;
    }
    file.seek(SeekFrom::Start(offset)).map_err(err)?;
    file.write_all(&bytes).map_err(err)?;
    file.sync_all().map_err(err)
}

pub fn count_files(path: &Path) -> usize {
    fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .count()
        })
        .unwrap_or(0)
}

pub fn tree_bytes(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| tree_bytes(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

pub fn tree_has_nan_f32(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.is_file() {
        return fs::read(path).is_ok_and(|bytes| {
            bytes
                .chunks_exact(4)
                .any(|chunk| f32::from_le_bytes(chunk.try_into().expect("4B")).is_nan())
        });
    }
    fs::read_dir(path).is_ok_and(|entries| {
        entries
            .filter_map(Result::ok)
            .any(|entry| tree_has_nan_f32(&entry.path()))
    })
}

fn dense(row: &[f32]) -> SlotVector {
    SlotVector::Dense {
        dim: row.len() as u32,
        data: row.to_vec(),
    }
}

fn brute_top_k(rows: &[Vec<f32>], ids: &[CxId], query: &[f32], k: usize) -> Vec<CxId> {
    let mut scored = rows
        .iter()
        .zip(ids)
        .map(|(row, id)| (*id, dot(row, query)))
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.as_bytes().cmp(right.0.as_bytes()))
    });
    scored.into_iter().take(k).map(|entry| entry.0).collect()
}

fn decode_vec(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err(format!("vector bytes {} not multiple of 4", bytes.len()));
    }
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let value = f32::from_le_bytes(chunk.try_into().expect("4B"));
            value
                .is_finite()
                .then_some(value)
                .ok_or_else(|| "base CF vector contained non-finite f32".to_string())
        })
        .collect()
}

fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in values {
        *value /= norm.max(1e-6);
    }
}

fn dense_signal(pair: usize, dim: usize, a: f32, b: f32, c: f32) -> f32 {
    let x = (pair as f32 + 1.0) * (dim as f32 + 3.0);
    let y = (pair as f32 + 17.0) * (dim as f32 + 11.0);
    (x * a).sin() * 0.63 + (y * b).cos() * 0.31 + ((x + y) * c).sin() * 0.06
}

fn min_bit_edge_pair(seed: &RotationSeed) -> (Vec<f32>, Vec<f32>) {
    let mut left = vec![0.0; seed.dim];
    let mut right = vec![0.0; seed.dim];
    left[0] = 1.0;
    right[1] = 1.0;
    apply_inverse_rotation(seed, &mut left);
    apply_inverse_rotation(seed, &mut right);
    (left, right)
}
