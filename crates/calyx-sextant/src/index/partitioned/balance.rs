use rayon::prelude::*;

use crate::index::{SpannCentroidIndex, build_centroids};

use super::{IDX_MIX, gen_row, normalize};

type RegionSplit = (Vec<Vec<f32>>, Vec<Vec<u64>>);

const MAX_RECLUSTER_DEPTH: usize = 4;

/// Split any oversized region until every final bucket is <= `cap`.
pub(super) fn balance_regions(
    initial: &SpannCentroidIndex,
    buckets: Vec<Vec<u64>>,
    seed: u64,
    dim: usize,
    cap: usize,
) -> RegionSplit {
    let initial_centroids = initial.centroids();
    let split: Vec<RegionSplit> = buckets
        .par_iter()
        .enumerate()
        .map(|(region, members)| {
            if members.is_empty() {
                (Vec::new(), Vec::new())
            } else if members.len() <= cap {
                (
                    vec![initial_centroids[region].clone()],
                    vec![members.clone()],
                )
            } else {
                split_oversized(members, seed, dim, cap, region as u64, 0)
            }
        })
        .collect();
    flatten(split)
}

fn split_oversized(
    members: &[u64],
    seed: u64,
    dim: usize,
    cap: usize,
    salt: u64,
    depth: usize,
) -> RegionSplit {
    if members.len() <= cap {
        return (
            vec![centroid_for_members(members, seed, dim)],
            vec![members.to_vec()],
        );
    }
    if depth >= MAX_RECLUSTER_DEPTH {
        return chunk_by_cap(members, seed, dim, cap);
    }
    let k_sub = members.len().div_ceil(cap).max(2);
    let rows: Vec<(u32, Vec<f32>)> = members
        .iter()
        .enumerate()
        .map(|(i, &idx)| (i as u32, gen_row(seed, idx, dim)))
        .collect();
    let sub = build_centroids(&rows, k_sub, seed ^ salt.wrapping_mul(IDX_MIX));
    let mut sub_buckets: Vec<Vec<u64>> = vec![Vec::new(); sub.centroid_count()];
    for (i, &idx) in members.iter().enumerate() {
        sub_buckets[sub.assign(&rows[i].1) as usize].push(idx);
    }
    let largest = sub_buckets.iter().map(Vec::len).max().unwrap_or(0);
    if largest >= members.len() {
        return chunk_by_cap(members, seed, dim, cap);
    }
    let mut out = Vec::new();
    for (sub_idx, bucket) in sub_buckets.into_iter().enumerate() {
        if bucket.is_empty() {
            continue;
        }
        if bucket.len() <= cap {
            out.push((vec![sub.centroids()[sub_idx].clone()], vec![bucket]));
        } else {
            out.push(split_oversized(
                &bucket,
                seed,
                dim,
                cap,
                salt ^ (sub_idx as u64).wrapping_mul(IDX_MIX),
                depth + 1,
            ));
        }
    }
    flatten(out)
}

fn chunk_by_cap(members: &[u64], seed: u64, dim: usize, cap: usize) -> RegionSplit {
    let mut centroids = Vec::new();
    let mut buckets = Vec::new();
    for chunk in members.chunks(cap.max(1)) {
        centroids.push(centroid_for_members(chunk, seed, dim));
        buckets.push(chunk.to_vec());
    }
    (centroids, buckets)
}

fn centroid_for_members(members: &[u64], seed: u64, dim: usize) -> Vec<f32> {
    let mut center = vec![0.0; dim];
    for &idx in members {
        let row = gen_row(seed, idx, dim);
        for (c, v) in center.iter_mut().zip(row) {
            *c += v;
        }
    }
    normalize(&mut center);
    center
}

fn flatten(parts: Vec<RegionSplit>) -> RegionSplit {
    let mut centroids = Vec::new();
    let mut buckets = Vec::new();
    for (cents, buks) in parts {
        centroids.extend(cents);
        buckets.extend(buks);
    }
    (centroids, buckets)
}
