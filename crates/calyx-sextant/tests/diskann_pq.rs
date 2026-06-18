use calyx_sextant::index::{DiskAnnPqBuildParams, DiskAnnPqIndex};

fn rows() -> Vec<(u32, Vec<f32>)> {
    (0..32)
        .map(|idx| {
            let x = idx as f32 / 32.0;
            (idx, vec![x, x + 0.1, 1.0 - x, 0.9 - x])
        })
        .collect()
}

#[test]
fn pq_build_encode_lut_and_readback() {
    let dir = std::env::temp_dir().join(format!("calyx-pq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("graph.pq");
    let index = DiskAnnPqIndex::build(
        &rows(),
        DiskAnnPqBuildParams {
            subvectors: 2,
            centroids: 8,
            iterations: 3,
        },
    )
    .expect("build pq");
    index.write_atomic(&path).expect("write pq");
    let read = DiskAnnPqIndex::read(&path).expect("read pq");
    assert_eq!(read.node_count(), 32);
    assert_eq!(read.subvectors(), 2);
    assert_eq!(read.centroids(), 8);
    assert!(read.ram_bytes() > 32 * 2);
    let query = read.query(&rows()[7].1).expect("query");
    let self_distance = query.distance_l2(7).expect("self distance");
    let far_distance = query.distance_l2(31).expect("far distance");
    assert!(self_distance <= far_distance);
}

#[test]
fn pq_rejects_non_divisible_subvectors() {
    let err = DiskAnnPqIndex::build(
        &rows(),
        DiskAnnPqBuildParams {
            subvectors: 3,
            centroids: 8,
            iterations: 1,
        },
    )
    .expect_err("bad subvectors");
    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
}
