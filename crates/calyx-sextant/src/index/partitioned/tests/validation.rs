use super::super::assignment::{
    AssignmentRouting, AssignmentSink, BoundedAssignmentConfig, read_ids,
    stream_assign_to_ids_bounded,
};
use crate::index::SpannCentroidIndex;
use crate::index::partitioned::{
    PartitionBuildParams, PartitionedSearch, VectorSource, build_partitioned_vault,
};

#[test]
fn partitioned_open_rejects_corrupt_root_graph() {
    let dir = std::env::temp_dir().join(format!("calyx-part-root-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = params(31);
    let manifest = build_partitioned_vault(&dir, p).expect("build");
    corrupt_format_version(&dir.join(&manifest.root_graph_rel));

    let error = match PartitionedSearch::open(&dir) {
        Ok(_) => panic!("corrupt root graph opened"),
        Err(error) => error,
    };

    assert_eq!(error.code, crate::error::CALYX_INDEX_CORRUPT);
    assert!(error.message.contains("root graph"));
    assert!(error.message.contains(&manifest.root_graph_rel));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn partitioned_open_rejects_corrupt_unprobed_region_graph() {
    let dir =
        std::env::temp_dir().join(format!("calyx-part-region-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = params(37);
    let manifest = build_partitioned_vault(&dir, p).expect("build");
    let meta = manifest.regions.last().expect("region");
    corrupt_format_version(&dir.join(&meta.graph_rel));

    let error = match PartitionedSearch::open(&dir) {
        Ok(_) => panic!("corrupt region graph opened"),
        Err(error) => error,
    };

    assert_eq!(error.code, crate::error::CALYX_INDEX_CORRUPT);
    assert!(error.message.contains("region"));
    assert!(error.message.contains(&meta.graph_rel));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bounded_assignment_cap_is_hard_stored_region_cap() {
    let dir = std::env::temp_dir().join(format!("calyx-part-hard-cap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let centroids = SpannCentroidIndex::from_parts(
        2,
        vec![vec![0.0, 0.0], vec![10.0, 0.0]],
        Vec::new(),
        Vec::new(),
    )
    .expect("centroids");
    let source = StaticSource {
        rows: vec![vec![5.0, 0.0]; 4],
    };

    let regions = stream_assign_to_ids_bounded(
        &dir,
        AssignmentSink::Final,
        &centroids,
        &source,
        2,
        BoundedAssignmentConfig {
            cap: 2,
            routing_probe: 2,
            routing: AssignmentRouting::Exact,
            boundary_epsilon: 3.0,
            max_replication: 2,
        },
    )
    .expect("bounded assignment");

    assert_eq!(regions.iter().map(|region| region.count).sum::<usize>(), 4);
    assert!(regions.iter().all(|region| region.count <= 2));
    for region in &regions {
        assert_eq!(
            read_ids(&dir.join(&region.ids_rel)).unwrap().len(),
            region.count
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn params(seed: u64) -> PartitionBuildParams {
    PartitionBuildParams {
        n_cx: 128,
        dim: 16,
        n_regions: 4,
        seed,
        sample: 128,
        chunk: 64,
        m_max: 8,
        ef_construction: 32,
        region_build_parallelism: 2,
        final_assignment_probe: crate::index::DEFAULT_FINAL_ASSIGNMENT_PROBE,
        final_assignment_cap: None,
    }
}

fn corrupt_format_version(path: &std::path::Path) {
    use std::io::{Seek, SeekFrom, Write};

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open graph for corruption");
    file.seek(SeekFrom::Start(8)).expect("seek format version");
    file.write_all(&99_u32.to_le_bytes())
        .expect("write bad format version");
}

struct StaticSource {
    rows: Vec<Vec<f32>>,
}

impl VectorSource for StaticSource {
    fn dim(&self) -> usize {
        self.rows[0].len()
    }

    fn len(&self) -> u64 {
        self.rows.len() as u64
    }

    fn row(&self, idx: u64) -> Vec<f32> {
        self.rows[idx as usize].clone()
    }
}
