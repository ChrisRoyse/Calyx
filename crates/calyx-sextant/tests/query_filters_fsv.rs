use std::collections::BTreeMap;
use std::fs;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, InputRef, LedgerRef, Modality, SlotId, VaultId,
};
use calyx_sextant::{
    AnchorPredicate, CALYX_SEXTANT_QUERY_SHAPE, InvertedIndex, MetadataPredicate, Query,
    QueryFilters, ScalarOp, ScalarPredicate, SearchEngine, SlotIndexMap,
};
use serde_json::json;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::{cx_u8_fill as cx, hex};

#[test]
fn query_filters_apply_scalars_anchors_and_metadata() {
    let (engine, rows) = filter_engine();
    let filters = include_alpha_filters();
    let filtered = engine
        .search(
            &Query::new("cat")
                .with_slots(vec![SlotId::new(1)])
                .with_filters(filters),
        )
        .unwrap();

    assert_eq!(ids(&filtered), vec![rows[0].cx_id.to_string()]);
    assert_eq!(filtered[0].rank, 1);
    assert_eq!(filtered[0].provenance.hash, rows[0].provenance.hash);

    let missing_scalar = engine
        .search(
            &Query::new("cat")
                .with_slots(vec![SlotId::new(1)])
                .with_filters(QueryFilters {
                    scalars: vec![ScalarPredicate {
                        name: "missing".to_string(),
                        op: ScalarOp::Gte,
                        value: 0.0,
                    }],
                    ..QueryFilters::default()
                }),
        )
        .unwrap();
    assert!(missing_scalar.is_empty());

    let high_confidence = engine
        .search(
            &Query::new("cat")
                .with_slots(vec![SlotId::new(1)])
                .with_filters(QueryFilters {
                    anchors: vec![AnchorPredicate {
                        kind: AnchorKind::Label("topic".to_string()),
                        value: Some(AnchorValue::Enum("science".to_string())),
                        min_confidence: Some(0.99),
                        source: Some("filter-fsv".to_string()),
                    }],
                    ..QueryFilters::default()
                }),
        )
        .unwrap();
    assert!(high_confidence.is_empty());

    let metadata_mismatch = engine
        .search(
            &Query::new("cat")
                .with_slots(vec![SlotId::new(1)])
                .with_filters(QueryFilters {
                    metadata: vec![MetadataPredicate::InputPointerContains(
                        "missing-pointer-fragment".to_string(),
                    )],
                    ..QueryFilters::default()
                }),
        )
        .unwrap();
    assert!(metadata_mismatch.is_empty());

    let non_finite_scalar = engine
        .search(
            &Query::new("cat")
                .with_slots(vec![SlotId::new(1)])
                .with_filters(QueryFilters {
                    scalars: vec![ScalarPredicate {
                        name: "quality".to_string(),
                        op: ScalarOp::Gte,
                        value: f64::NAN,
                    }],
                    ..QueryFilters::default()
                }),
        )
        .unwrap_err();
    assert_eq!(non_finite_scalar.code, CALYX_SEXTANT_QUERY_SHAPE);
}

#[test]
fn query_filters_scan_full_candidate_set_when_needed() {
    let filtered = late_filter_hits();

    assert_eq!(ids(&filtered), vec![cx(10).to_string()]);
    assert_eq!(filtered[0].rank, 1);
}

#[test]
#[ignore = "manual FSV writes filter rows and result/provenance source-of-truth artifacts"]
fn query_filters_manual_fsv() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-query-filters-fsv"));
    fs::create_dir_all(&root).unwrap();

    let (engine, rows) = filter_engine();
    let unfiltered = engine
        .search(&Query::new("cat").with_slots(vec![SlotId::new(1)]))
        .unwrap();
    let filters = include_alpha_filters();
    let filtered = engine
        .search(
            &Query::new("cat")
                .with_slots(vec![SlotId::new(1)])
                .with_filters(filters.clone()),
        )
        .unwrap();
    let late_filtered = late_filter_hits();
    let metadata_mismatch = engine
        .search(
            &Query::new("cat")
                .with_slots(vec![SlotId::new(1)])
                .with_filters(QueryFilters {
                    metadata: vec![MetadataPredicate::InputPointerContains(
                        "missing-pointer-fragment".to_string(),
                    )],
                    ..QueryFilters::default()
                }),
        )
        .unwrap();
    let anchor_mismatch = engine
        .search(
            &Query::new("cat")
                .with_slots(vec![SlotId::new(1)])
                .with_filters(QueryFilters {
                    anchors: vec![AnchorPredicate {
                        kind: AnchorKind::Label("topic".to_string()),
                        value: Some(AnchorValue::Enum("missing-topic".to_string())),
                        min_confidence: Some(0.8),
                        source: Some("filter-fsv".to_string()),
                    }],
                    ..QueryFilters::default()
                }),
        )
        .unwrap();
    let non_finite_scalar = engine
        .search(
            &Query::new("cat")
                .with_slots(vec![SlotId::new(1)])
                .with_filters(QueryFilters {
                    scalars: vec![ScalarPredicate {
                        name: "quality".to_string(),
                        op: ScalarOp::Gte,
                        value: f64::NAN,
                    }],
                    ..QueryFilters::default()
                }),
        )
        .unwrap_err();

    fs::write(
        root.join("filterable-rows.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("query-filters.json"),
        serde_json::to_vec_pretty(&filters).unwrap(),
    )
    .unwrap();
    let readback = json!({
        "unfiltered_ids": ids(&unfiltered),
        "filtered_ids": ids(&filtered),
        "late_filter_ids": ids(&late_filtered),
        "late_filter_rank": late_filtered.first().map(|hit| hit.rank),
        "filtered_provenance_hashes": filtered
            .iter()
            .map(|hit| hex(&hit.provenance.hash))
            .collect::<Vec<_>>(),
        "metadata_mismatch_count": metadata_mismatch.len(),
        "anchor_mismatch_count": anchor_mismatch.len(),
        "non_finite_scalar_error": non_finite_scalar.code,
        "excluded_ids_absent": !ids(&filtered).contains(&rows[1].cx_id.to_string())
            && !ids(&filtered).contains(&rows[2].cx_id.to_string()),
        "rank_renumbered": filtered.first().map(|hit| hit.rank) == Some(1),
    });
    fs::write(
        root.join("query-filter-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();

    assert_eq!(ids(&filtered), vec![rows[0].cx_id.to_string()]);
    assert_eq!(ids(&late_filtered), vec![cx(10).to_string()]);
    assert_eq!(readback["metadata_mismatch_count"], 0);
    assert_eq!(readback["anchor_mismatch_count"], 0);
    assert_eq!(
        readback["non_finite_scalar_error"],
        CALYX_SEXTANT_QUERY_SHAPE
    );
    assert_eq!(readback["excluded_ids_absent"], true);
    assert_eq!(readback["rank_renumbered"], true);
}

fn late_filter_hits() -> Vec<calyx_sextant::Hit> {
    let map = SlotIndexMap::new();
    map.register(InvertedIndex::new(SlotId::new(1))).unwrap();
    let mut engine = SearchEngine::new(map);
    for value in 1..=10 {
        let row = row(RowSpec {
            value,
            text: "cat same",
            quality: if value == 10 { 1.0 } else { 0.0 },
            topic: "science",
            confidence: 0.90,
            pointer_fragment: "late-filter-match",
            modality: Modality::Text,
            panel_version: 7,
            created_at: value as u64,
        });
        let text = row.input_ref.pointer.as_ref().unwrap();
        engine
            .indexes
            .insert_text(SlotId::new(1), row.cx_id, text, row.created_at)
            .unwrap();
        engine.put_constellation(row);
    }

    let mut query = Query::new("cat")
        .with_slots(vec![SlotId::new(1)])
        .with_filters(QueryFilters {
            scalars: vec![ScalarPredicate {
                name: "quality".to_string(),
                op: ScalarOp::Eq,
                value: 1.0,
            }],
            ..QueryFilters::default()
        });
    query.k = 1;
    engine.search(&query).unwrap()
}

fn filter_engine() -> (SearchEngine, Vec<calyx_core::Constellation>) {
    let map = SlotIndexMap::new();
    map.register(InvertedIndex::new(SlotId::new(1))).unwrap();
    let mut engine = SearchEngine::new(map);
    let rows = vec![
        row(RowSpec {
            value: 1,
            text: "cat alpha include",
            quality: 0.95,
            topic: "science",
            confidence: 0.90,
            pointer_fragment: "include-alpha",
            modality: Modality::Text,
            panel_version: 7,
            created_at: 100,
        }),
        row(RowSpec {
            value: 2,
            text: "cat beta low",
            quality: 0.20,
            topic: "science",
            confidence: 0.95,
            pointer_fragment: "include-beta",
            modality: Modality::Text,
            panel_version: 7,
            created_at: 101,
        }),
        row(RowSpec {
            value: 3,
            text: "cat gamma finance",
            quality: 0.97,
            topic: "finance",
            confidence: 0.90,
            pointer_fragment: "exclude-gamma",
            modality: Modality::Code,
            panel_version: 8,
            created_at: 102,
        }),
    ];
    for row in &rows {
        let text = row.input_ref.pointer.as_ref().unwrap();
        engine
            .indexes
            .insert_text(SlotId::new(1), row.cx_id, text, row.created_at)
            .unwrap();
        engine.put_constellation(row.clone());
    }
    (engine, rows)
}

struct RowSpec {
    value: u8,
    text: &'static str,
    quality: f64,
    topic: &'static str,
    confidence: f32,
    pointer_fragment: &'static str,
    modality: Modality,
    panel_version: u32,
    created_at: u64,
}

fn row(spec: RowSpec) -> calyx_core::Constellation {
    let mut scalars = BTreeMap::new();
    scalars.insert("quality".to_string(), spec.quality);
    calyx_core::Constellation {
        cx_id: cx(spec.value),
        vault_id: vault(),
        panel_version: spec.panel_version,
        created_at: spec.created_at,
        input_ref: InputRef {
            hash: [spec.value; 32],
            pointer: Some(format!(
                "zfs://calyx/filter/{}/{}",
                spec.pointer_fragment, spec.text
            )),
            redacted: false,
        },
        modality: spec.modality,
        slots: BTreeMap::new(),
        scalars,
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("topic".to_string()),
            value: AnchorValue::Enum(spec.topic.to_string()),
            source: "filter-fsv".to_string(),
            observed_at: spec.created_at,
            confidence: spec.confidence,
        }],
        provenance: LedgerRef {
            seq: spec.created_at,
            hash: [spec.value; 32],
        },
        flags: CxFlags::default(),
    }
}

fn include_alpha_filters() -> QueryFilters {
    QueryFilters {
        scalars: vec![ScalarPredicate {
            name: "quality".to_string(),
            op: ScalarOp::Gte,
            value: 0.90,
        }],
        anchors: vec![AnchorPredicate {
            kind: AnchorKind::Label("topic".to_string()),
            value: Some(AnchorValue::Enum("science".to_string())),
            min_confidence: Some(0.8),
            source: Some("filter-fsv".to_string()),
        }],
        metadata: vec![
            MetadataPredicate::Vault(vault()),
            MetadataPredicate::Modality(Modality::Text),
            MetadataPredicate::PanelVersion(7),
            MetadataPredicate::CreatedAt {
                min: Some(90),
                max: Some(100),
            },
            MetadataPredicate::InputRedacted(false),
            MetadataPredicate::InputPointerContains("include-alpha".to_string()),
        ],
    }
}

fn ids(hits: &[calyx_sextant::Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.cx_id.to_string()).collect()
}

fn vault() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
