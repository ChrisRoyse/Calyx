use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::index::diskann::graph::DiskAnnGraphReader;

#[test]
fn build_with_progress_emits_vamana_batches_and_writes_physical_graph() {
    let root = temp_root("diskann-build-progress");
    let path = root.join("graph.cda");
    let rows = (0..128_u32)
        .map(|id| {
            (
                id,
                vec![
                    id as f32 / 128.0,
                    (128 - id) as f32 / 128.0,
                    (id % 7) as f32 / 7.0,
                ],
            )
        })
        .collect::<Vec<_>>();
    let params = DiskAnnBuildParams {
        dim: 3,
        m_max: 8,
        ef_construction: 16,
        alpha: 1.2,
    };
    let mut phases = Vec::new();

    build_diskann_graph_with_backend_and_progress(
        &path,
        &rows,
        params,
        DiskAnnBuildBackend::CpuVamana,
        |event| {
            phases.push((event.phase.to_string(), event.rows));
            Ok(())
        },
    )
    .expect("build graph with progress");

    let reader = DiskAnnGraphReader::open(&path).expect("open physical graph");
    assert_eq!(reader.header().node_count, rows.len() as u64);
    assert!(fs::metadata(&path).expect("graph metadata").len() > 4096);
    assert!(
        phases
            .iter()
            .any(|(phase, rows)| phase == "diskann_init_page" && *rows == 128)
    );
    assert!(
        phases
            .iter()
            .any(|(phase, rows)| phase == "diskann_vamana_pass1_batch_ok" && *rows == 128)
    );
    assert!(
        phases
            .iter()
            .any(|(phase, rows)| phase == "diskann_vamana_pass2_batch_ok" && *rows == 128)
    );
    assert!(
        phases
            .iter()
            .any(|(phase, rows)| phase == "diskann_graph_write_page" && *rows == 128)
    );
    assert!(
        phases
            .iter()
            .any(|(phase, rows)| phase == "diskann_graph_write_ok" && *rows == 128)
    );
    let _ = fs::remove_dir_all(root);
}

fn temp_root(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    dir.push(format!("calyx-sextant-{name}-{nanos}"));
    fs::create_dir_all(&dir).expect("create temp root");
    dir
}
