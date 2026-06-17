use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId,
};
use calyx_sextant::{AnchorPredicate, MetadataPredicate, QueryFilters, ScalarOp, ScalarPredicate};
use ulid::Ulid;

use super::*;

#[test]
fn rebuild_writes_manifest_graph_idmap_filter_sidecar_and_searches() {
    let root = scratch("happy");
    let docs = docs([
        (1, vec![1.0, 0.0]),
        (2, vec![0.0, 1.0]),
        (3, vec![0.8, 0.2]),
    ]);

    let summary = rebuild_from_docs(&root, &docs, 7).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let hits = indexes
        .search(SlotId::new(0), &dense(vec![1.0, 0.0]), 2)
        .expect("search");
    let filter_entry = indexes.manifest.filter.as_ref().expect("filter entry");
    let filter_path = root.join(&filter_entry.index_rel);
    let filter_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&filter_path).unwrap()).unwrap();

    assert_eq!(summary.slots, 1);
    assert_eq!(summary.total_rows, 3);
    assert!(summary.manifest_path.is_file());
    assert_eq!(hits[0].cx_id, cx(1));
    assert_eq!(filter_json["format"], "calyx-search-filter-index-v1");
    assert_eq!(filter_json["rows"].as_array().unwrap().len(), 3);
    assert!(root.join("idx/search/manifest.json").is_file());
    assert!(root.join("idx/search").read_dir().unwrap().count() >= 3);
    fs::remove_dir_all(root).ok();
}

#[test]
fn filtered_search_matches_exact_reference_for_scalar_anchor_and_metadata() {
    let root = scratch("filtered");
    let docs = rich_docs();
    rebuild_from_docs(&root, &docs, 11).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let filters = selective_filters();
    let candidates = indexes.filter_candidates(&filters).unwrap().unwrap();
    let query = dense(vec![1.0, 0.0]);
    let hits = indexes
        .search_filtered(SlotId::new(0), &query, candidates.len(), &candidates)
        .expect("filtered search");
    let expected = exact_reference(&docs, &filters, query.as_dense().unwrap());

    assert_eq!(candidates, BTreeSet::from([cx(1), cx(3)]));
    assert_eq!(
        hits.iter().map(|hit| hit.cx_id).collect::<Vec<_>>(),
        expected
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn empty_match_filter_returns_empty_candidate_set() {
    let root = scratch("empty-filter");
    rebuild_from_docs(&root, &rich_docs(), 12).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let filters = QueryFilters {
        scalars: vec![ScalarPredicate {
            name: "quality_score".to_string(),
            op: ScalarOp::Gt,
            value: 100.0,
        }],
        anchors: Vec::new(),
        metadata: Vec::new(),
    };

    let candidates = indexes.filter_candidates(&filters).unwrap().unwrap();

    assert!(candidates.is_empty());
    fs::remove_dir_all(root).ok();
}

#[test]
fn missing_filter_sidecar_fails_closed() {
    let root = scratch("missing-filter");
    rebuild_from_docs(&root, &rich_docs(), 13).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes.manifest.filter.as_ref().unwrap();
    fs::remove_file(root.join(&entry.index_rel)).unwrap();

    let err = indexes.filter_candidates(&selective_filters()).unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("filter sidecar missing"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn stale_filter_sidecar_hash_fails_closed() {
    let root = scratch("stale-filter");
    rebuild_from_docs(&root, &rich_docs(), 14).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");
    let entry = indexes.manifest.filter.as_ref().unwrap();
    fs::write(root.join(&entry.index_rel), b"{\"format\":\"tampered\"}").unwrap();

    let err = indexes.filter_candidates(&selective_filters()).unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("sha256"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn missing_manifest_fails_closed() {
    let err = PersistedSearchIndexes::open(&scratch("missing")).unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("manifest missing"));
}

#[test]
fn query_dim_mismatch_fails_closed() {
    let root = scratch("dim");
    rebuild_from_docs(&root, &docs([(1, vec![1.0, 0.0])]), 2).expect("rebuild");
    let indexes = PersistedSearchIndexes::open(&root).expect("open");

    let err = indexes
        .search(SlotId::new(0), &dense(vec![1.0, 0.0, 0.0]), 1)
        .unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    assert!(err.message().contains("dim 2 != query dim 3"));
    fs::remove_dir_all(root).ok();
}

fn selective_filters() -> QueryFilters {
    QueryFilters {
        scalars: vec![ScalarPredicate {
            name: "quality_score".to_string(),
            op: ScalarOp::Gte,
            value: 0.7,
        }],
        anchors: vec![AnchorPredicate {
            kind: AnchorKind::Label("issue735".to_string()),
            value: Some(AnchorValue::Enum("gold".to_string())),
            min_confidence: Some(0.8),
            source: Some("unit".to_string()),
        }],
        metadata: vec![
            MetadataPredicate::Modality(Modality::Text),
            MetadataPredicate::InputPointerContains("north".to_string()),
        ],
    }
}

fn exact_reference(
    docs: &BTreeMap<CxId, Constellation>,
    filters: &QueryFilters,
    query: &[f32],
) -> Vec<CxId> {
    let mut scored = docs
        .values()
        .filter(|cx| crate::cmd::search::filters::matches(cx, filters))
        .filter_map(|cx| {
            cx.slots
                .get(&SlotId::new(0))?
                .as_dense()
                .map(|values| (cx.cx_id, values))
        })
        .map(|(cx_id, values)| (cx_id, cosine(query, values)))
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    scored.into_iter().map(|(cx_id, _)| cx_id).collect()
}

fn rich_docs() -> BTreeMap<CxId, Constellation> {
    let mut first = constellation(cx(1), vec![1.0, 0.0]);
    first.scalars.insert("quality_score".to_string(), 0.91);
    first.input_ref.pointer = Some("north/alpha".to_string());
    first.anchors.push(anchor("gold", 0.95, "unit"));

    let mut second = constellation(cx(2), vec![0.0, 1.0]);
    second.scalars.insert("quality_score".to_string(), 0.97);
    second.input_ref.pointer = Some("south/beta".to_string());
    second.anchors.push(anchor("gold", 0.95, "unit"));

    let mut third = constellation(cx(3), vec![0.8, 0.2]);
    third.scalars.insert("quality_score".to_string(), 0.72);
    third.input_ref.pointer = Some("north/gamma".to_string());
    third.anchors.push(anchor("gold", 0.82, "unit"));

    let mut fourth = constellation(cx(4), vec![0.9, 0.1]);
    fourth.scalars.insert("quality_score".to_string(), 0.69);
    fourth.input_ref.pointer = Some("north/delta".to_string());
    fourth.anchors.push(anchor("gold", 0.95, "unit"));

    [first, second, third, fourth]
        .into_iter()
        .map(|cx| (cx.cx_id, cx))
        .collect()
}

fn docs<const N: usize>(rows: [(u8, Vec<f32>); N]) -> BTreeMap<CxId, Constellation> {
    rows.into_iter()
        .map(|(seed, vector)| {
            let id = cx(seed);
            (id, constellation(id, vector))
        })
        .collect()
}

fn anchor(label: &str, confidence: f32, source: &str) -> Anchor {
    Anchor {
        kind: AnchorKind::Label("issue735".to_string()),
        value: AnchorValue::Enum(label.to_string()),
        source: source.to_string(),
        observed_at: 1,
        confidence,
    }
}

fn constellation(cx_id: CxId, vector: Vec<f32>) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(SlotId::new(0), dense(vector));
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

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let (mut dot, mut left_l2, mut right_l2) = (0.0, 0.0, 0.0);
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_l2 += left * left;
        right_l2 += right * right;
    }
    if left_l2 == 0.0 || right_l2 == 0.0 {
        0.0
    } else {
        dot / (left_l2.sqrt() * right_l2.sqrt())
    }
}
