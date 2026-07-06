use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

#[test]
fn graph_cf_label_anchor_round_trips_chunks() {
    let root = temp_root("i8bin-label-anchor-db");
    let mut labels = Vec::new();
    for idx in 0..17 {
        labels.push(idx % 2 == 0);
    }
    let mut counts = BTreeMap::new();
    counts.insert("0".to_string(), 8);
    counts.insert("1".to_string(), 9);

    let written = write(
        &root,
        "unit_labels",
        "unit",
        1,
        &"11".repeat(32),
        &counts,
        &labels,
        5,
    )
    .unwrap();
    let loaded = read(&root, "unit_labels").unwrap();

    assert!(written.readback_matches);
    assert_eq!(loaded.labels, labels);
    assert_eq!(loaded.db_readback.chunk_count, 4);
    assert_eq!(
        loaded.db_readback.manifest_value_sha256,
        written.manifest_value_sha256
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn graph_cf_label_anchor_refuses_duplicate_key() {
    let root = temp_root("i8bin-label-anchor-db-duplicate");
    let labels = vec![true, false, true, false];
    let counts = BTreeMap::new();

    write(
        &root,
        "unit_labels",
        "unit",
        1,
        &"22".repeat(32),
        &counts,
        &labels,
        4,
    )
    .unwrap();
    let err = write(
        &root,
        "unit_labels",
        "unit",
        1,
        &"22".repeat(32),
        &counts,
        &labels,
        4,
    )
    .unwrap_err();

    assert_eq!(err.code, "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_EXISTS");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gdelt_root_anchor_imports_from_text() {
    let root = temp_root("i8bin-label-anchor-gdelt-root");
    let rows = root.join("rows.jsonl");
    fs::write(
        &rows,
        [
            "{\"label\":0,\"text\":\"EventCode 040 root 04 quad 1 Goldstein 3.5 tone -8 Actor1 USA Actor2 CAN\"}\n",
            "{\"label\":1,\"text\":\"EventCode 190 root 19 quad 4 Goldstein -2 tone 5 Actor1 FRA Actor2 USA\"}\n",
            "{\"label\":0,\"text\":\"EventCode 040 root 04 quad 1 Goldstein 2.5 tone -3 Actor1 USA Actor2 CAN\"}\n",
            "{\"label\":1,\"text\":\"EventCode 010 root 01 quad 1 Goldstein 0 tone 0 Actor1 MEX Actor2 BRA\"}\n",
        ]
        .concat(),
    )
    .unwrap();

    for anchor in [
        AnchorSpec::GdeltRoot("04".to_string()),
        AnchorSpec::GdeltEventCode("040".to_string()),
        AnchorSpec::GdeltEventRoot("04".to_string()),
        AnchorSpec::GdeltActorPair("USA".to_string(), "CAN".to_string()),
        AnchorSpec::GdeltGoldsteinSign("pos".to_string()),
        AnchorSpec::GdeltToneSign("neg".to_string()),
        AnchorSpec::GdeltGoldsteinBucket(6),
        AnchorSpec::GdeltToneBucket(9),
    ] {
        let imported = load_rows_jsonl(&rows, 1, &anchor, None).unwrap();
        assert_eq!(imported.labels, vec![true, false, true, false]);
        assert_eq!(imported.label_counts["1"], 2);
        assert_eq!(imported.label_counts["0"], 2);
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gdelt_field_anchors_import_from_structured_rows() {
    let root = temp_root("i8bin-label-anchor-gdelt-fields");
    let rows = root.join("rows.jsonl");
    fs::write(
        &rows,
        [
            "{\"label\":0,\"gdelt_event_code\":\"010\",\"gdelt_event_root\":\"01\",\"gdelt_sql_date\":\"20240102\",\"gdelt_goldstein\":\"3.5\",\"gdelt_avg_tone\":\"-8.0\",\"gdelt_actor1_name\":\"USA\",\"gdelt_actor2_name\":\"CAN\",\"gdelt_actor1_country\":\"USA\",\"gdelt_actor2_country\":\"CAN\",\"gdelt_action_geo_fullname\":\"New York, United States\",\"source_url\":\"https://news.example.org/path\"}\n",
            "{\"label\":1,\"gdelt_event_code\":\"190\",\"gdelt_event_root\":\"19\",\"gdelt_sql_date\":\"20240203\",\"gdelt_goldstein\":\"-2.0\",\"gdelt_avg_tone\":\"5.0\",\"gdelt_actor1_name\":\"FRA\",\"gdelt_actor2_name\":\"USA\",\"gdelt_actor1_country\":\"FRA\",\"gdelt_actor2_country\":\"USA\",\"gdelt_action_geo_fullname\":\"Gaza, Israel (general), Israel\",\"source_url\":\"https://agency.example.net/path\"}\n",
            "{\"label\":0,\"gdelt_event_code\":\"010\",\"gdelt_event_root\":\"01\",\"gdelt_sql_date\":\"20240104\",\"gdelt_goldstein\":\"2.5\",\"gdelt_avg_tone\":\"-3.0\",\"gdelt_actor1_name\":\"USA\",\"gdelt_actor2_name\":\"CAN\",\"gdelt_actor1_country\":\"USA\",\"gdelt_actor2_country\":\"CAN\",\"gdelt_action_geo_fullname\":\"New York, United States\",\"source_url\":\"https://news.example.org/other\"}\n",
            "{\"label\":1,\"gdelt_event_code\":\"043\",\"gdelt_event_root\":\"04\",\"gdelt_sql_date\":\"20240305\",\"gdelt_goldstein\":\"0.0\",\"gdelt_avg_tone\":\"0.0\",\"gdelt_actor1_name\":\"MEX\",\"gdelt_actor2_name\":\"BRA\",\"gdelt_actor1_country\":\"MEX\",\"gdelt_actor2_country\":\"BRA\",\"gdelt_action_geo_fullname\":\"Jerusalem, Israel (general), Israel\",\"source_url\":\"https://wire.example.com/item\"}\n",
        ]
        .concat(),
    )
    .unwrap();

    for anchor in [
        AnchorSpec::GdeltEventCode("010".to_string()),
        AnchorSpec::GdeltActor1Country("USA".to_string()),
        AnchorSpec::GeoFullContains("United States".to_string()),
        AnchorSpec::GdeltActorPair("USA".to_string(), "CAN".to_string()),
        AnchorSpec::GdeltActorCountryPair("USA".to_string(), "CAN".to_string()),
        AnchorSpec::GdeltSqlDatePrefix("202401".to_string()),
        AnchorSpec::GdeltSourceHost("news.example.org".to_string()),
        AnchorSpec::GdeltSourceTld("org".to_string()),
        AnchorSpec::GdeltGoldsteinSign("pos".to_string()),
        AnchorSpec::GdeltToneSign("neg".to_string()),
        AnchorSpec::GdeltGoldsteinBucket(6),
        AnchorSpec::GdeltToneBucket(9),
    ] {
        let imported = load_rows_jsonl(&rows, 1, &anchor, None).unwrap();
        assert_eq!(imported.labels, vec![true, false, true, false]);
        assert_eq!(imported.label_counts["1"], 2);
        assert_eq!(imported.label_counts["0"], 2);
    }
    let _ = fs::remove_dir_all(root);
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
