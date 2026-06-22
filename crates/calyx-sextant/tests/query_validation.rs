use calyx_core::{AnchorKind, AnchorValue, CxId, SlotId, SlotVector};
use calyx_sextant::{
    AnchorPredicate, CALYX_SEXTANT_QUERY_SHAPE, HnswIndex, MetadataPredicate, Query, QueryFilters,
    ScalarOp, ScalarPredicate, SearchEngine, SlotIndexMap,
};

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::dense;

#[test]
fn malformed_query_vector_fails_before_index_state_changes() {
    let engine = vector_engine();
    let before = engine.indexes.stats();
    let query = Query::new("bad vector")
        .with_slots(vec![slot()])
        .with_vector(SlotVector::Dense {
            dim: 2,
            data: vec![f32::NAN, 0.0],
        });

    let error = engine.search(&query).expect_err("query NaN rejected");
    let after = engine.indexes.stats();

    assert_eq!(error.code, CALYX_SEXTANT_QUERY_SHAPE);
    assert_eq!(after, before);
}

#[test]
fn duplicate_slots_and_zero_limits_fail_as_query_shape() {
    let engine = vector_engine();
    let duplicate = Query::new("duplicate slots")
        .with_slots(vec![slot(), slot()])
        .with_vector(dense(vec![1.0, 0.0]));
    let mut zero_k = Query::new("zero k")
        .with_slots(vec![slot()])
        .with_vector(dense(vec![1.0, 0.0]));
    zero_k.k = 0;

    assert_eq!(
        engine.search(&duplicate).unwrap_err().code,
        CALYX_SEXTANT_QUERY_SHAPE
    );
    assert_eq!(
        engine.search(&zero_k).unwrap_err().code,
        CALYX_SEXTANT_QUERY_SHAPE
    );
}

#[test]
fn malformed_filters_fail_as_query_shape() {
    let engine = vector_engine();
    let scalar_nan = filters_query(QueryFilters {
        scalars: vec![ScalarPredicate {
            name: "quality".to_string(),
            op: ScalarOp::Gte,
            value: f64::NAN,
        }],
        ..QueryFilters::default()
    });
    let anchor_confidence = filters_query(QueryFilters {
        anchors: vec![AnchorPredicate {
            kind: AnchorKind::Label("topic".to_string()),
            value: Some(AnchorValue::Number(f64::INFINITY)),
            min_confidence: Some(1.1),
            source: None,
        }],
        ..QueryFilters::default()
    });
    let reversed_time = filters_query(QueryFilters {
        metadata: vec![MetadataPredicate::CreatedAt {
            min: Some(20),
            max: Some(10),
        }],
        ..QueryFilters::default()
    });

    assert_eq!(
        engine.search(&scalar_nan).unwrap_err().code,
        CALYX_SEXTANT_QUERY_SHAPE
    );
    assert_eq!(
        engine.search(&anchor_confidence).unwrap_err().code,
        CALYX_SEXTANT_QUERY_SHAPE
    );
    assert_eq!(
        engine.search(&reversed_time).unwrap_err().code,
        CALYX_SEXTANT_QUERY_SHAPE
    );
}

fn filters_query(filters: QueryFilters) -> Query {
    Query::new("filter validation")
        .with_slots(vec![slot()])
        .with_vector(dense(vec![1.0, 0.0]))
        .with_filters(filters)
}

fn vector_engine() -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(slot(), 2, 42)).unwrap();
    let engine = SearchEngine::new(map);
    engine
        .indexes
        .insert(slot(), CxId::from_bytes([1; 16]), dense(vec![1.0, 0.0]), 1)
        .unwrap();
    engine
}

const fn slot() -> SlotId {
    SlotId::new(8)
}
