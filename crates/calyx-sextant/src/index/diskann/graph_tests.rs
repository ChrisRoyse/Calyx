use std::fs::{self, File};
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

#[test]
fn writer_emits_compact_v3_i8_blocks_and_reader_follows_offsets() {
    let root = temp_root("diskann-graph-v3");
    let path = root.join("graph.cda");
    let header = DiskAnnHeader {
        format_version: DISKANN_FORMAT_VERSION,
        dim: 3,
        m_max: 2,
        max_degree: 1,
        entry_point_id: 0,
        node_count: 2,
    };
    let mut writer = DiskAnnGraphWriter::create(&path, header).unwrap();
    writer.write_node(0, &[1.0, 0.0, 0.0], &[1]).unwrap();
    writer.write_node(1, &[0.0, 1.0, 0.0], &[0]).unwrap();
    writer.finish().unwrap();

    let reader = DiskAnnGraphReader::open(&path).unwrap();

    assert_eq!(reader.header().format_version, DISKANN_FORMAT_VERSION);
    assert_eq!(reader.node_block_size(), 64);
    assert_eq!(fs::metadata(&path).unwrap().len(), 4096 + 2 * 64);
    assert_eq!(reader.node_block_offset(1).unwrap(), 4096 + 64);
    let node = reader.read_node(0).unwrap();
    let DiskAnnVectorRef::I8(vector) = node.vector else {
        panic!("v3 graph must expose i8 vectors");
    };
    assert_eq!(vector, &[127, 0, 0]);
    assert_eq!(node.neighbors, &[1]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reader_keeps_compact_v2_f32_graphs_readable() {
    let root = temp_root("diskann-graph-v2");
    let path = root.join("graph.cda");
    let header = DiskAnnHeader {
        format_version: DISKANN_F32_FORMAT_VERSION,
        dim: 3,
        m_max: 2,
        max_degree: 1,
        entry_point_id: 0,
        node_count: 2,
    };
    let mut file = File::create(&path).unwrap();
    file.write_all(&header.encode()).unwrap();
    file.write_all(&node_block(64, &[1.0, 0.0, 0.0], &[1]))
        .unwrap();
    file.write_all(&node_block(64, &[0.0, 1.0, 0.0], &[0]))
        .unwrap();
    file.sync_all().unwrap();

    let reader = DiskAnnGraphReader::open(&path).unwrap();

    assert_eq!(reader.header().format_version, DISKANN_F32_FORMAT_VERSION);
    assert_eq!(reader.node_block_size(), 64);
    assert_eq!(fs::metadata(&path).unwrap().len(), 4096 + 2 * 64);
    let node = reader.read_node(0).unwrap();
    let DiskAnnVectorRef::F32(vector) = node.vector else {
        panic!("v2 graph must expose f32 vectors");
    };
    assert_eq!(vector, &[1.0, 0.0, 0.0]);
    assert_eq!(node.neighbors, &[1]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reader_keeps_legacy_v1_page_aligned_graphs_readable() {
    let root = temp_root("diskann-graph-v1");
    let path = root.join("graph.cda");
    let header = DiskAnnHeader {
        format_version: DISKANN_LEGACY_FORMAT_VERSION,
        dim: 2,
        m_max: 2,
        max_degree: 1,
        entry_point_id: 0,
        node_count: 2,
    };
    let mut file = File::create(&path).unwrap();
    file.write_all(&header.encode()).unwrap();
    file.write_all(&node_block(4096, &[1.0, 0.0], &[1]))
        .unwrap();
    file.write_all(&node_block(4096, &[0.0, 1.0], &[0]))
        .unwrap();
    file.sync_all().unwrap();

    let reader = DiskAnnGraphReader::open(&path).unwrap();

    assert_eq!(
        reader.header().format_version,
        DISKANN_LEGACY_FORMAT_VERSION
    );
    assert_eq!(reader.node_block_size(), 4096);
    assert_eq!(fs::metadata(&path).unwrap().len(), 4096 * 3);
    let node = reader.read_node(1).unwrap();
    let DiskAnnVectorRef::F32(vector) = node.vector else {
        panic!("v1 graph must expose f32 vectors");
    };
    assert_eq!(vector, &[0.0, 1.0]);
    assert_eq!(node.neighbors, &[0]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reader_rejects_unknown_format_version() {
    let root = temp_root("diskann-graph-unknown-version");
    let path = root.join("graph.cda");
    let header = DiskAnnHeader {
        format_version: 99,
        dim: 2,
        m_max: 2,
        max_degree: 1,
        entry_point_id: 0,
        node_count: 1,
    };
    fs::write(&path, header.encode()).unwrap();

    let error = DiskAnnGraphReader::open(&path).unwrap_err();

    assert_eq!(error.code, CALYX_INDEX_CORRUPT);
    assert!(error.message.contains("format_version 99"));
    let _ = fs::remove_dir_all(root);
}

fn node_block(block_size: usize, vector: &[f32], neighbors: &[u32]) -> Vec<u8> {
    let mut block = Vec::with_capacity(block_size);
    for value in vector {
        block.extend_from_slice(&value.to_le_bytes());
    }
    block.extend_from_slice(&(neighbors.len() as u32).to_le_bytes());
    for neighbor in neighbors {
        block.extend_from_slice(&neighbor.to_le_bytes());
    }
    block.resize(block_size, 0);
    block
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
