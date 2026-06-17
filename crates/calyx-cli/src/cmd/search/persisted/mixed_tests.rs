use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, SparseEntry,
    VaultId,
};
use ulid::Ulid;

use super::*;

#[test]
fn rebuild_writes_sparse_and_multi_sidecars_and_searches() {
    let root = scratch("mixed");
    let docs = mixed_docs();

    let summary = rebuild_from_docs(&root, &docs, 21).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let sparse_hits = indexes
        .search(SlotId::new(1), &sparse(8, [1]), 2)
        .expect("sparse search");
    let multi_hits = indexes
        .search(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 2)
        .expect("multi search");
    let sparse_entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 1)
        .expect("sparse entry");
    let multi_entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .expect("multi entry");
    let sparse_json: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join(sparse_entry.index_rel.as_ref().unwrap())).unwrap(),
    )
    .unwrap();
    let multi_json: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join(multi_entry.index_rel.as_ref().unwrap())).unwrap(),
    )
    .unwrap();

    assert_eq!(summary.slots, 3);
    assert_eq!(summary.total_rows, 9);
    assert_eq!(sparse_entry.kind, "sparse_inverted");
    assert_eq!(sparse_entry.dim, Some(8));
    assert_eq!(multi_entry.kind, "multi_maxsim");
    assert_eq!(multi_entry.token_dim, Some(2));
    assert_eq!(multi_entry.token_count, Some(5));
    assert_eq!(sparse_json["format"], "calyx-search-sparse-index-v1");
    assert_eq!(multi_json["format"], "calyx-search-multi-maxsim-index-v1");
    assert_eq!(sparse_hits[0].cx_id, cx(3));
    assert_eq!(sparse_hits[1].cx_id, cx(1));
    assert_eq!(multi_hits[0].cx_id, cx(2));
    fs::remove_dir_all(root).ok();
}

#[test]
fn filtered_sparse_and_multi_search_use_candidate_sidecars() {
    let root = scratch("mixed-filtered");
    let docs = mixed_docs();
    rebuild_from_docs(&root, &docs, 22).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let candidates = BTreeSet::from([cx(1), cx(2)]);

    let sparse_hits = indexes
        .search_filtered(SlotId::new(1), &sparse(8, [1]), 3, &candidates)
        .expect("filtered sparse");
    let multi_hits = indexes
        .search_filtered(SlotId::new(2), &multi(2, [[0.6, 0.4]]), 3, &candidates)
        .expect("filtered multi");

    assert_eq!(
        sparse_hits.iter().map(|hit| hit.cx_id).collect::<Vec<_>>(),
        vec![cx(1)]
    );
    assert_eq!(
        multi_hits.iter().map(|hit| hit.cx_id).collect::<Vec<_>>(),
        vec![cx(2), cx(1)]
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn missing_sparse_sidecar_fails_closed() {
    let root = scratch("missing-sparse");
    rebuild_from_docs(&root, &mixed_docs(), 23).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 1)
        .unwrap();
    fs::remove_file(root.join(entry.index_rel.as_ref().unwrap())).unwrap();

    let err = indexes
        .search(SlotId::new(1), &sparse(8, [1]), 1)
        .unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("sparse sidecar missing"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn sparse_dim_mismatch_and_corrupt_sidecar_fail_closed() {
    let root = scratch("bad-sparse");
    rebuild_from_docs(&root, &mixed_docs(), 24).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let dim_err = indexes
        .search(SlotId::new(1), &sparse(9, [1]), 1)
        .unwrap_err();

    assert_eq!(dim_err.code(), "CALYX_STALE_DERIVED");
    assert!(dim_err.message().contains("index dim 8 != query dim 9"));

    let mut corrupted = PersistedSearchIndexes::open(&root).expect("open");
    let pos = corrupted
        .manifest
        .slots
        .iter()
        .position(|entry| entry.slot == 1)
        .unwrap();
    let path = root.join(corrupted.manifest.slots[pos].index_rel.as_ref().unwrap());
    fs::write(&path, b"{not json").unwrap();
    corrupted.manifest.slots[pos].sha256 = Some(sha256_hex(&fs::read(&path).unwrap()));

    let corrupt_err = corrupted
        .search(SlotId::new(1), &sparse(8, [1]), 1)
        .unwrap_err();

    assert_eq!(corrupt_err.code(), "CALYX_STALE_DERIVED");
    assert!(corrupt_err.message().contains("not valid JSON"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn missing_multi_sidecar_token_dim_mismatch_and_corrupt_sidecar_fail_closed() {
    let root = scratch("bad-multi");
    rebuild_from_docs(&root, &mixed_docs(), 25).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let dim_err = indexes
        .search(SlotId::new(2), &multi(3, [[1.0, 0.0, 0.0]]), 1)
        .unwrap_err();

    assert_eq!(dim_err.code(), "CALYX_STALE_DERIVED");
    assert!(
        dim_err
            .message()
            .contains("token_dim 2 != query token_dim 3")
    );

    let entry = indexes
        .manifest
        .slots
        .iter()
        .find(|entry| entry.slot == 2)
        .unwrap();
    let original = fs::read(root.join(entry.index_rel.as_ref().unwrap())).unwrap();
    fs::remove_file(root.join(entry.index_rel.as_ref().unwrap())).unwrap();
    let missing_err = indexes
        .search(SlotId::new(2), &multi(2, [[1.0, 0.0]]), 1)
        .unwrap_err();

    assert_eq!(missing_err.code(), "CALYX_STALE_DERIVED");
    assert!(missing_err.message().contains("multi sidecar missing"));
    fs::write(root.join(entry.index_rel.as_ref().unwrap()), original).unwrap();

    let mut corrupted = PersistedSearchIndexes::open(&root).expect("open");
    let pos = corrupted
        .manifest
        .slots
        .iter()
        .position(|entry| entry.slot == 2)
        .unwrap();
    let path = root.join(corrupted.manifest.slots[pos].index_rel.as_ref().unwrap());
    fs::write(&path, b"{not json").unwrap();
    corrupted.manifest.slots[pos].sha256 = Some(sha256_hex(&fs::read(&path).unwrap()));
    let corrupt_err = corrupted
        .search(SlotId::new(2), &multi(2, [[1.0, 0.0]]), 1)
        .unwrap_err();

    assert_eq!(corrupt_err.code(), "CALYX_STALE_DERIVED");
    assert!(corrupt_err.message().contains("not valid JSON"));
    fs::remove_dir_all(root).ok();
}

fn mixed_docs() -> BTreeMap<CxId, Constellation> {
    [
        constellation(
            cx(1),
            [
                (SlotId::new(0), dense(vec![1.0, 0.0])),
                (SlotId::new(1), sparse(8, [1, 2])),
                (SlotId::new(2), multi(2, [[1.0, 0.0], [0.0, 1.0]])),
            ],
        ),
        constellation(
            cx(2),
            [
                (SlotId::new(0), dense(vec![0.0, 1.0])),
                (SlotId::new(1), sparse(8, [3])),
                (SlotId::new(2), multi(2, [[0.0, 1.0], [0.5, 0.5]])),
            ],
        ),
        constellation(
            cx(3),
            [
                (SlotId::new(0), dense(vec![0.8, 0.2])),
                (SlotId::new(1), sparse(8, [1])),
                (SlotId::new(2), multi(2, [[1.0, 0.0]])),
            ],
        ),
    ]
    .into_iter()
    .map(|cx| (cx.cx_id, cx))
    .collect()
}

fn constellation<const N: usize>(
    cx_id: CxId,
    slot_rows: [(SlotId, SlotVector); N],
) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.extend(slot_rows);
    Constellation {
        cx_id,
        vault_id: VaultId::from_ulid(Ulid::from_bytes([9; 16])),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: [0; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [1; 32],
        },
        flags: CxFlags::default(),
    }
}

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

fn sparse<const N: usize>(dim: u32, terms: [u32; N]) -> SlotVector {
    SlotVector::Sparse {
        dim,
        entries: terms
            .into_iter()
            .map(|idx| SparseEntry { idx, val: 1.0 })
            .collect(),
    }
}

fn multi<const N: usize, const D: usize>(token_dim: u32, tokens: [[f32; D]; N]) -> SlotVector {
    SlotVector::Multi {
        token_dim,
        tokens: tokens.into_iter().map(Vec::from).collect(),
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "calyx-cli-persisted-search-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch");
    dir
}
