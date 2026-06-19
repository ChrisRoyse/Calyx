use crate::index::partitioned::{PartitionBuildParams, PartitionedSearch, build_partitioned_vault};

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
