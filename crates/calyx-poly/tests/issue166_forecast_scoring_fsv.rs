mod fsv_support;

use std::fs;
use std::path::Path;

use calyx_core::FixedClock;
use calyx_ledger::{
    DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore, LedgerEntry, decode,
};
use calyx_poly::{
    ForecastScoreRequest, ForecastSource, PolyError, ResolvedOutcome,
    write_forecast_score_artifacts,
};
use serde_json::{Value, json};

use fsv_support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const TEST_TS: u64 = 1_786_400_166;
const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn issue166_forecast_scoring_against_outcome_fsv() {
    let (root, _env_supplied) =
        named_fsv_root("POLY_ISSUE166_FSV_ROOT", "issue166-forecast-scoring-fsv");
    reset_dir(&root);

    let score_root = root.join("scores");
    let ledger_dir = root.join("ledger");
    let mut ledger = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).expect("open score ledger"),
        FixedClock::new(TEST_TS),
    )
    .expect("open score ledger appender");

    let happy = happy_request();
    let happy_dir = score_root.join(&happy.score_id);
    let happy_before = state_snapshot(&ledger_dir, &happy_dir);
    let manifest = write_forecast_score_artifacts(&score_root, &mut ledger, &happy)
        .expect("happy score should persist");
    let happy_after = state_snapshot(&ledger_dir, &happy_dir);
    assert!(happy_after["artifact_exists"].as_bool().unwrap());
    assert_eq!(happy_after["ledger_rows"].as_u64(), Some(1));
    assert_eq!(manifest.ledger_ref.seq, 0);
    assert_close(manifest.metrics.brier, 0.04);
    assert_close(manifest.metrics.log_loss.unwrap(), -0.8_f64.ln());
    assert!(manifest.metrics.direction_accuracy);
    assert_eq!(manifest.metrics.calibration_bin.index, 8);
    assert_close(manifest.metrics.probability_drift.unwrap(), 0.15);

    let manifest_readback: Value =
        read_json(&happy_dir.join("manifest.json")).expect("read manifest");
    let score_readback: Value = read_json(&happy_dir.join("score.json")).expect("read score");
    let forecast_readback: Value =
        read_json(&happy_dir.join("forecast.json")).expect("read forecast");
    let outcome_readback: Value = read_json(&happy_dir.join("outcome.json")).expect("read outcome");
    assert_eq!(score_readback["brier"], json!(0.03999999999999998));
    assert_eq!(forecast_readback["probability"], json!(0.8));
    assert_eq!(outcome_readback["actual_win"], json!(true));

    let happy_entry = read_ledger_entries(&ledger_dir)
        .pop()
        .expect("happy ledger row");
    let happy_payload = ledger_payload(&happy_entry);
    assert_eq!(happy_entry.kind, EntryKind::Score);
    assert_eq!(happy_payload["score_id"], happy.score_id);
    assert_eq!(
        happy_payload["forecast_ref"]["ref_hash"],
        safe_ref_hash(&happy.forecast_id)
    );
    assert_eq!(
        happy_payload["outcome_ref"]["ref_hash"],
        safe_ref_hash(&happy.outcome_id)
    );
    assert_eq!(happy_payload["direction_accuracy"], true);

    let long_ids = long_public_ids_request();
    let long_dir = score_root.join(&long_ids.score_id);
    let long_manifest = write_forecast_score_artifacts(&score_root, &mut ledger, &long_ids)
        .expect("long public ids should persist");
    let long_forecast_readback: Value =
        read_json(&long_dir.join("forecast.json")).expect("read long forecast");
    let long_outcome_readback: Value =
        read_json(&long_dir.join("outcome.json")).expect("read long outcome");
    assert_eq!(long_manifest.forecast_id, long_ids.forecast_id);
    assert_eq!(long_forecast_readback["market_id"], long_ids.market_id);
    assert_eq!(long_outcome_readback["outcome_id"], long_ids.outcome_id);

    let edges = vec![
        edge_unresolved(&score_root, &ledger_dir, &mut ledger, &happy),
        edge_malformed(&score_root, &ledger_dir, &mut ledger, &happy),
        edge_confidence_ceiling(&score_root, &ledger_dir, &mut ledger, &happy),
        edge_duplicate(&score_root, &ledger_dir, &mut ledger, &happy),
        edge_stale_version(&score_root, &ledger_dir, &mut ledger, &happy),
    ];

    let final_entries = read_ledger_entries(&ledger_dir);
    assert_eq!(final_entries.len(), 2);
    assert_eq!(
        score_artifact_dirs(&score_root),
        vec![happy.score_id.clone(), long_ids.score_id.clone()]
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "source_of_truth": {
            "score_root": score_root.display().to_string(),
            "ledger_dir": ledger_dir.display().to_string()
        },
        "happy_path": {
            "before": happy_before,
            "after": happy_after,
            "manifest": manifest_readback,
            "score": score_readback,
            "forecast": forecast_readback,
            "outcome": outcome_readback,
            "ledger_payload": happy_payload
        },
        "long_public_ids": {
            "manifest": long_manifest,
            "forecast": long_forecast_readback,
            "outcome": long_outcome_readback
        },
        "edge_cases": edges,
        "final": {
            "ledger_rows": final_entries.len(),
            "last_ledger_hash": hex(&final_entries.last().expect("last ledger row").entry_hash),
            "score_artifact_dirs": score_artifact_dirs(&score_root),
            "files": files
        }
    });
    write_json(
        &root.join("issue166-forecast-scoring-fsv-summary.json"),
        &summary,
    );
    write_blake3sums(&root);
}

fn happy_request() -> ForecastScoreRequest {
    ForecastScoreRequest {
        score_id: "score166happy".to_string(),
        forecast_id: "forecast166".to_string(),
        forecast_version: 2,
        current_forecast_version: 2,
        market_id: "market166".to_string(),
        outcome_id: "outcome166".to_string(),
        source: ForecastSource::DeepSeekAgent,
        provider: Some("deepseek-v4-pro".to_string()),
        probability: 0.8,
        confidence: 0.7,
        forecast_ts: TEST_TS - 3_600,
        scored_ts: TEST_TS,
        horizon_secs: 3_600,
        sufficiency_state: "sufficient".to_string(),
        previous_probability: Some(0.65),
        forecast_artifact_hash: HASH_A.to_string(),
        outcome: ResolvedOutcome {
            outcome_id: "outcome166".to_string(),
            resolved: true,
            actual_win: true,
            resolved_ts: TEST_TS - 60,
            source: "uma".to_string(),
            version: 1,
        },
        calibration_bin_count: 10,
    }
}

fn long_public_ids_request() -> ForecastScoreRequest {
    let condition = "0x39ea14226927f879d93550628e70f22bc444915cea787c5734c53e89d5cf6d20";
    let token = "34747254630927017064599589941321309211596494400768268440049646273862919127907";
    ForecastScoreRequest {
        score_id: "score166longids".to_string(),
        forecast_id: "crypto-snapshot-b000af389508a59ae29f653e195c434e".to_string(),
        forecast_version: 1,
        current_forecast_version: 1,
        market_id: condition.to_string(),
        outcome_id: token.to_string(),
        source: ForecastSource::CalyxNative,
        provider: None,
        probability: 0.61,
        confidence: 0.55,
        forecast_ts: TEST_TS - 7_200,
        scored_ts: TEST_TS,
        horizon_secs: 7_200,
        sufficiency_state: "non_admissible_kernel".to_string(),
        previous_probability: None,
        forecast_artifact_hash: HASH_A.to_string(),
        outcome: ResolvedOutcome {
            outcome_id: token.to_string(),
            resolved: true,
            actual_win: false,
            resolved_ts: TEST_TS - 60,
            source: "gamma-closed-derived".to_string(),
            version: 1,
        },
        calibration_bin_count: 10,
    }
}

fn edge_unresolved(
    score_root: &Path,
    ledger_dir: &Path,
    ledger: &mut LedgerAppender<DirectoryLedgerStore, FixedClock>,
    base: &ForecastScoreRequest,
) -> Value {
    let mut request = base.clone();
    request.score_id = "score166unresolved".to_string();
    request.outcome.resolved = false;
    edge_case(
        "unresolved_outcome",
        "CALYX_POLY_SCORE_UNRESOLVED_OUTCOME",
        score_root,
        ledger_dir,
        ledger,
        &request,
    )
}

fn edge_malformed(
    score_root: &Path,
    ledger_dir: &Path,
    ledger: &mut LedgerAppender<DirectoryLedgerStore, FixedClock>,
    base: &ForecastScoreRequest,
) -> Value {
    let mut request = base.clone();
    request.score_id = "score166malformed".to_string();
    request.probability = 1.2;
    edge_case(
        "malformed_forecast",
        "CALYX_POLY_SCORE_MALFORMED_FORECAST",
        score_root,
        ledger_dir,
        ledger,
        &request,
    )
}

fn edge_confidence_ceiling(
    score_root: &Path,
    ledger_dir: &Path,
    ledger: &mut LedgerAppender<DirectoryLedgerStore, FixedClock>,
    base: &ForecastScoreRequest,
) -> Value {
    // #184: confidence == 1.0 must fail closed against the never-reaches-1 ceiling, and no artifact
    // directory may be published on disk.
    let mut request = base.clone();
    request.score_id = "score166confidenceceiling".to_string();
    request.confidence = 1.0;
    edge_case(
        "confidence_ceiling",
        "CALYX_POLY_SCORE_CONFIDENCE_CEILING",
        score_root,
        ledger_dir,
        ledger,
        &request,
    )
}

fn edge_duplicate(
    score_root: &Path,
    ledger_dir: &Path,
    ledger: &mut LedgerAppender<DirectoryLedgerStore, FixedClock>,
    base: &ForecastScoreRequest,
) -> Value {
    edge_case(
        "duplicate_score_attempt",
        "CALYX_POLY_SCORE_DUPLICATE",
        score_root,
        ledger_dir,
        ledger,
        base,
    )
}

fn edge_stale_version(
    score_root: &Path,
    ledger_dir: &Path,
    ledger: &mut LedgerAppender<DirectoryLedgerStore, FixedClock>,
    base: &ForecastScoreRequest,
) -> Value {
    let mut request = base.clone();
    request.score_id = "score166stale".to_string();
    request.forecast_version = 1;
    request.current_forecast_version = 2;
    edge_case(
        "stale_forecast_version",
        "CALYX_POLY_SCORE_STALE_FORECAST_VERSION",
        score_root,
        ledger_dir,
        ledger,
        &request,
    )
}

fn edge_case(
    name: &str,
    expected_code: &str,
    score_root: &Path,
    ledger_dir: &Path,
    ledger: &mut LedgerAppender<DirectoryLedgerStore, FixedClock>,
    request: &ForecastScoreRequest,
) -> Value {
    let dir = score_root.join(&request.score_id);
    let before = state_snapshot(ledger_dir, &dir);
    let err = write_forecast_score_artifacts(score_root, ledger, request)
        .expect_err("edge case must fail closed");
    let (code, message) = score_error(err);
    assert_eq!(code, expected_code);
    let after = state_snapshot(ledger_dir, &dir);
    if name != "duplicate_score_attempt" {
        assert!(!after["artifact_exists"].as_bool().unwrap());
    }
    assert_eq!(after["ledger_rows"], before["ledger_rows"]);
    json!({
        "name": name,
        "expected_code": expected_code,
        "error_code": code,
        "error_message": message,
        "before": before,
        "after": after
    })
}

fn state_snapshot(ledger_dir: &Path, artifact_dir: &Path) -> Value {
    let file_count = if artifact_dir.exists() {
        fs::read_dir(artifact_dir)
            .expect("read artifact dir")
            .count()
    } else {
        0
    };
    json!({
        "ledger_rows": read_ledger_entries(ledger_dir).len(),
        "artifact_dir": artifact_dir.display().to_string(),
        "artifact_exists": artifact_dir.exists(),
        "artifact_file_count": file_count
    })
}

fn read_ledger_entries(ledger_dir: &Path) -> Vec<LedgerEntry> {
    let store = DirectoryLedgerStore::open(ledger_dir).expect("open ledger for readback");
    store
        .scan()
        .expect("scan physical ledger rows")
        .into_iter()
        .map(|row| decode(&row.bytes).expect("decode physical ledger row"))
        .collect()
}

fn ledger_payload(entry: &LedgerEntry) -> Value {
    serde_json::from_slice(&entry.payload).expect("decode ledger payload")
}

fn read_json(path: &Path) -> Option<Value> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn score_error(err: PolyError) -> (String, String) {
    match err {
        PolyError::Score { code, message } => (code, message),
        other => panic!("expected score error, got {other:?}"),
    }
}

fn score_artifact_dirs(root: &Path) -> Vec<String> {
    if !root.exists() {
        return Vec::new();
    }
    let mut dirs: Vec<String> = fs::read_dir(root)
        .expect("read score root")
        .filter_map(|entry| {
            let entry = entry.expect("read score root entry");
            entry
                .file_type()
                .expect("read file type")
                .is_dir()
                .then(|| entry.file_name().to_string_lossy().to_string())
        })
        .filter(|name| !name.starts_with('.'))
        .collect();
    dirs.sort();
    dirs
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "actual {actual} expected {expected}"
    );
}

fn safe_ref_hash(value: &str) -> String {
    blake3::hash(value.as_bytes()).to_hex().to_string()
}
