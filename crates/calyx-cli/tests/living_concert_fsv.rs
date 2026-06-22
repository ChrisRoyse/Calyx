use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

mod support;

use support::living_concert::run_living_concert;
use support::living_concert_data::CorpusSource;

#[test]
fn living_concert_synthetic_known_io_smoke() {
    let root = temp_root("issue641-synthetic");
    let readback = run_living_concert(
        &root,
        CorpusSource::Synthetic,
        &PathBuf::from(env!("CARGO_BIN_EXE_calyx")),
    );

    assert_living_concert(&readback, 4);
    assert_eq!(
        readback["corpus"]["hand_expected"],
        json!("2+2=4 doc is relevant; water-dry doc is non-relevant")
    );
    fs::remove_dir_all(root).expect("cleanup synthetic root");
}

#[test]
#[ignore = "manual FSV: requires verified BEIR SciFact dataset under CALYX_HOME"]
fn living_concert_manual_scifact_fsv() {
    let root = env::var_os("CALYX_ISSUE641_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| temp_root("issue641-scifact"));
    let dataset = env::var_os("CALYX_SCIFACT_DATASET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/calyx/data/datasets/beir-scifact/scifact"));

    let readback = run_living_concert(
        &root,
        CorpusSource::Scifact(dataset),
        &PathBuf::from(env!("CARGO_BIN_EXE_calyx")),
    );

    assert_living_concert(&readback, 4);
    assert!(readback["corpus"]["corpus"]["bytes"].as_u64().unwrap() > 1_000_000);
    println!("issue641_living_concert_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
}

fn assert_living_concert(readback: &Value, expected_docs: usize) {
    assert_eq!(readback["issue"], json!(641));
    assert_eq!(readback["expected"]["docs_ingested"], json!(expected_docs));
    assert_eq!(
        readback["loop"]["admission"]["decision"]["admitted"],
        json!(true)
    );
    assert_eq!(readback["loop"]["objective"]["non_decreasing"], json!(true));
    assert_eq!(readback["loop"]["oracle"]["sufficient"], json!(true));
    assert_eq!(
        readback["loop"]["oracle"]["t_hat"],
        readback["expected"]["oracle_t_hat"]
    );
    assert_eq!(
        readback["loop"]["ward"]["verdict"]["overall_pass"],
        json!(false)
    );
    assert_eq!(
        readback["edges"]["lens_endpoint_killed"]["error_code"],
        json!("CALYX_LENS_UNREACHABLE")
    );
    assert_eq!(
        readback["edges"]["conflicting_anchor_recurrence"]["merged"],
        json!(false)
    );
    assert_eq!(
        readback["edges"]["over_budget_background_work"]["error_code"],
        json!("CALYX_ANNEAL_BUDGET_EXHAUSTED")
    );
    assert_eq!(
        readback["edges"]["over_budget_background_work"]["serving_read_after"],
        json!(true)
    );
    assert!(
        readback["readbacks"]["verify_chain"]
            .as_str()
            .unwrap()
            .contains("CHAIN_INTACT")
    );
    for cf in [
        "base",
        "anchors",
        "slot_00",
        "slot_01",
        "slot_02",
        "slot_03",
        "assay",
        "online",
        "recurrence",
        "ledger",
    ] {
        assert!(
            readback["readbacks"][cf].as_str().unwrap().contains("CF\t"),
            "{cf} readback should contain physical CF rows"
        );
    }
}

fn temp_root(name: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    env::temp_dir().join(format!("{name}-{}-{millis}", std::process::id()))
}
