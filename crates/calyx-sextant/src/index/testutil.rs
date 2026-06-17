//! Deterministic Sextant index fixtures for PH68 tests and benches.

use std::path::{Path, PathBuf};

use calyx_core::{CxId, Result, SlotId};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::index::{
    DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams, PostingListWriter, SpannSearch,
    build_centroids,
};

#[derive(Debug)]
pub struct SyntheticVault {
    pub root: PathBuf,
    pub rows: Vec<(CxId, Vec<f32>)>,
    pub local_rows: Vec<(u32, Vec<f32>)>,
    pub diskann: DiskAnnSearch,
    pub spann: SpannSearch,
}

pub fn build_synthetic_vault(
    n_cx: usize,
    dim: usize,
    n_slots: usize,
    seed: u64,
    vault_path: &Path,
) -> Result<SyntheticVault> {
    validate_fixture_args(n_cx, dim, n_slots)?;
    std::fs::create_dir_all(vault_path)
        .map_err(|e| crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string()))?;
    let rows = synthetic_dense_rows(n_cx, dim, seed);
    let diskann = DiskAnnSearch::build(
        SlotId::new(0),
        vault_path.join("idx/slot_00.ann/graph.cda"),
        &rows,
        build_params(dim),
        None,
        search_params(64),
    )?;
    for slot in 1..n_slots {
        let path = vault_path.join(format!("idx/slot_{slot:02}.ann/graph.cda"));
        let _ = DiskAnnSearch::build(
            SlotId::new(slot as u16),
            path,
            &rows,
            build_params(dim),
            None,
            search_params(64),
        )?;
    }
    let local_rows = rows
        .iter()
        .enumerate()
        .map(|(idx, (_, vector))| (idx as u32, vector.clone()))
        .collect::<Vec<_>>();
    let centroid_count = (n_cx as f64).sqrt().ceil().max(1.0) as usize;
    let centroids = build_centroids(&local_rows, centroid_count, seed);
    let sparse_dir = vault_path.join("idx/slot_00.sparse");
    centroids.save(&sparse_dir)?;
    write_postings(&sparse_dir, &centroids, &local_rows)?;
    let spann = SpannSearch::new(SlotId::new(0), centroids, sparse_dir)
        .with_cx_map(rows.iter().map(|(cx, _)| *cx).collect())
        .with_default_n_probe(8);
    Ok(SyntheticVault {
        root: vault_path.to_path_buf(),
        rows,
        local_rows,
        diskann,
        spann,
    })
}

pub fn cx(idx: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..16].copy_from_slice(&(idx as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

pub fn synthetic_dense_rows(n: usize, dim: usize, seed: u64) -> Vec<(CxId, Vec<f32>)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..n)
        .map(|idx| {
            let mut vector = (0..dim)
                .map(|j| rng.gen_range(-1.0_f32..1.0) + ((idx + j) % dim) as f32 * 0.001)
                .collect::<Vec<_>>();
            vector[idx % dim] += 4.0;
            normalize(&mut vector);
            (cx(idx), vector)
        })
        .collect()
}

fn build_params(dim: usize) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim,
        m_max: 16,
        ef_construction: 64,
        alpha: 1.2,
    }
}

fn search_params(n: usize) -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: n,
        ef_search: n.max(64),
        rescore_k: n.max(64),
        rescore_from_raw: false,
    }
}

fn write_postings(
    sparse_dir: &Path,
    centroids: &crate::index::SpannCentroidIndex,
    rows: &[(u32, Vec<f32>)],
) -> Result<()> {
    let writer = PostingListWriter::new(sparse_dir);
    for centroid_id in 0..centroids.centroid_count() as u32 {
        let mut entries = centroids
            .assignments()
            .iter()
            .filter(|(_, assigned)| *assigned == centroid_id)
            .map(|(local_id, _)| {
                crate::index::PostingMember::new(*local_id, dense_to_sparse(rows, *local_id))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|m| m.cx_id);
        writer.write_list(centroid_id, &entries)?;
    }
    Ok(())
}

/// Posting members store the full member vector (#701) so SPANN search ranks by
/// true query distance. Test fixtures are dense, so store every dim as an (idx,val).
fn dense_to_sparse(rows: &[(u32, Vec<f32>)], local_id: u32) -> Vec<(u32, f32)> {
    rows[local_id as usize]
        .1
        .iter()
        .enumerate()
        .map(|(idx, val)| (idx as u32, *val))
        .collect()
}

fn validate_fixture_args(n_cx: usize, dim: usize, n_slots: usize) -> Result<()> {
    if n_cx == 0 || dim == 0 || n_slots == 0 || n_cx > u32::MAX as usize {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "synthetic vault requires nonzero n_cx, dim, n_slots and u32 local ids",
        ));
    }
    Ok(())
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in vector {
        *value /= norm;
    }
}
