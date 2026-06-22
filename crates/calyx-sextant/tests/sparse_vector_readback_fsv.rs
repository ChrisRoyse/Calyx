use std::fs;

use calyx_core::{CxId, SlotId, SlotVector, SparseEntry};
use calyx_sextant::{InvertedIndex, SextantIndex};
use serde_json::json;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::cx_u8_fill as cx;

#[test]
fn sparse_vector_readback_preserves_non_contiguous_ids_and_weights() {
    let mut index = InvertedIndex::new(SlotId::new(1));
    let id = cx(0x42);
    let original = sparse_vector(&[(42, 0.25), (4_096, 2.5), (65_535, 0.75)]);

    assert!(index.vector(id).is_none());
    index.insert(id, original.clone(), 7).unwrap();
    assert_eq!(index.vector(id), Some(original.clone()));
    assert_eq!(
        index
            .search(&sparse_vector(&[(4_096, 1.0)]), 1, None)
            .unwrap()[0]
            .cx_id,
        id
    );

    index.rebuild().unwrap();
    assert_eq!(index.vector(id), Some(original.clone()));

    index.insert_text(id, "plain text", 8).unwrap();
    assert_ne!(index.vector(id), Some(original));
}

#[test]
fn inverted_index_replacement_drops_stale_postings() {
    let mut index = InvertedIndex::new(SlotId::new(1));
    let id = cx(0x24);

    index.insert_text(id, "alpha shared", 1).unwrap();
    assert_eq!(index.lookup("alpha"), vec![id]);
    assert_eq!(index.search_text("alpha", 1)[0].cx_id, id);

    index.insert_text(id, "beta only", 2).unwrap();
    assert_eq!(index.total_docs(), 1);
    assert_eq!(index.lookup("alpha"), Vec::<CxId>::new());
    assert_eq!(index.lookup("beta"), vec![id]);
    assert!(index.search_text("alpha", 1).is_empty());
    assert_eq!(index.search_text("beta", 1)[0].cx_id, id);

    index
        .insert(id, sparse_vector(&[(10, 1.0), (20, 1.0)]), 3)
        .unwrap();
    assert_eq!(index.lookup("beta"), Vec::<CxId>::new());
    assert_eq!(index.lookup("t10"), vec![id]);

    index.insert(id, sparse_vector(&[(30, 1.0)]), 4).unwrap();
    assert_eq!(index.lookup("t10"), Vec::<CxId>::new());
    assert_eq!(index.lookup("t30"), vec![id]);
}

#[test]
#[ignore = "manual FSV writes PH25 sparse vector readback source-of-truth artifacts"]
fn sparse_vector_readback_manual_fsv() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-sparse-vector-readback-fsv"));
    fs::create_dir_all(&root).unwrap();

    let mut index = InvertedIndex::new(SlotId::new(1));
    let id = cx(0x42);
    let original = sparse_vector(&[(42, 0.25), (4_096, 2.5), (65_535, 0.75)]);
    let before = index.vector(id);

    index.insert(id, original.clone(), 7).unwrap();
    let after_insert = index.vector(id).unwrap();
    let search_top = index
        .search(&sparse_vector(&[(65_535, 1.0)]), 1, None)
        .unwrap()[0]
        .cx_id;

    index.rebuild().unwrap();
    let after_rebuild = index.vector(id).unwrap();

    index.insert_text(id, "plain text", 8).unwrap();
    let after_text_overwrite = index.vector(id).unwrap();

    let original_entries = sparse_entries(&original);
    let insert_entries = sparse_entries(&after_insert);
    let rebuild_entries = sparse_entries(&after_rebuild);
    let text_entries = sparse_entries(&after_text_overwrite);
    let readback = json!({
        "before_present": before.is_some(),
        "original_entries": entries_json(&original_entries),
        "after_insert_entries": entries_json(&insert_entries),
        "after_rebuild_entries": entries_json(&rebuild_entries),
        "after_text_overwrite_entries": entries_json(&text_entries),
        "insert_preserves_sparse_ids": insert_entries == original_entries,
        "rebuild_preserves_sparse_ids": rebuild_entries == original_entries,
        "text_overwrite_clears_stale_sparse_ids": text_entries != original_entries,
        "search_top": search_top.to_string(),
        "expected_top": id.to_string(),
    });

    let path = root.join("sparse-vector-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("sparse_vector_readback={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["before_present"], false);
    assert_eq!(readback["insert_preserves_sparse_ids"], true);
    assert_eq!(readback["rebuild_preserves_sparse_ids"], true);
    assert_eq!(readback["text_overwrite_clears_stale_sparse_ids"], true);
    assert_eq!(readback["search_top"], readback["expected_top"]);
}

fn sparse_vector(entries: &[(u32, f32)]) -> SlotVector {
    SlotVector::Sparse {
        dim: 1_000_000,
        entries: entries
            .iter()
            .map(|(idx, val)| SparseEntry {
                idx: *idx,
                val: *val,
            })
            .collect(),
    }
}

fn sparse_entries(vector: &SlotVector) -> Vec<(u32, f32)> {
    match vector {
        SlotVector::Sparse { entries, .. } => {
            entries.iter().map(|entry| (entry.idx, entry.val)).collect()
        }
        _ => Vec::new(),
    }
}

fn entries_json(entries: &[(u32, f32)]) -> Vec<serde_json::Value> {
    entries
        .iter()
        .map(|(idx, val)| json!({ "idx": idx, "val": val }))
        .collect()
}
