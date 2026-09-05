use std::fs;
use std::path::PathBuf;

use calyx_core::{FixedClock, Result};
use calyx_ledger::{
    DirectoryLedgerStore, LedgerAppender, LedgerCfStore, LedgerHeadAnchor, LedgerRow,
};
use calyx_poly::{
    ForecastScoreRequest, ForecastSource, ResolvedOutcome, write_forecast_score_artifacts,
};
use serde_json::Value;

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct PublishBlockingStore {
    inner: DirectoryLedgerStore,
    final_dir: PathBuf,
}

impl LedgerCfStore for PublishBlockingStore {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        self.inner.scan()
    }

    fn put_new(&mut self, seq: u64, bytes: &[u8]) -> Result<()> {
        self.inner.put_new(seq, bytes)?;
        fs::create_dir_all(&self.final_dir).expect("create post-commit publish collision");
        fs::write(self.final_dir.join("blocker"), b"non-empty")
            .expect("make publish collision non-empty");
        Ok(())
    }

    fn head_anchor(&self) -> Result<Option<LedgerHeadAnchor>> {
        self.inner.head_anchor()
    }

    fn put_head_anchor(&mut self, anchor: &LedgerHeadAnchor) -> Result<()> {
        self.inner.put_head_anchor(anchor)
    }
}

#[test]
fn ledger_commit_precedes_diagnostic_publish() {
    let root = std::env::temp_dir().join(format!(
        "calyx-poly-issue1390-publish-order-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("remove previous test root");
    }
    let score_root = root.join("scores");
    let final_dir = score_root.join("score1390publish");
    let store = PublishBlockingStore {
        inner: DirectoryLedgerStore::open(root.join("ledger")).expect("open ledger"),
        final_dir: final_dir.clone(),
    };
    let mut ledger =
        LedgerAppender::open(store, FixedClock::new(1_785_700_000)).expect("open ledger appender");
    let request = request();

    let err = write_forecast_score_artifacts(&score_root, &mut ledger, &request)
        .expect_err("diagnostic publication collision must surface");
    assert_eq!(err.code(), "CALYX_POLY_SCORE_ARTIFACT_PUBLISH_FAILED");
    let entries = ledger.scan_entries().expect("read committed score ledger");
    assert_eq!(entries.len(), 1);
    let payload: Value = serde_json::from_slice(&entries[0].payload).expect("decode score payload");
    assert_eq!(payload["score_id"], "score1390publish");
    assert!(!final_dir.join("manifest.json").exists());
    assert!(
        score_root
            .join(".score1390publish.tmp")
            .join("manifest.json")
            .exists()
    );

    let duplicate = write_forecast_score_artifacts(&score_root, &mut ledger, &request)
        .expect_err("committed score must deduplicate despite missing final diagnostics");
    assert_eq!(duplicate.code(), "CALYX_POLY_SCORE_DUPLICATE");
    assert_eq!(ledger.scan_entries().expect("rescan ledger").len(), 1);
    drop(ledger);
    fs::remove_dir_all(&root).expect("remove test root");
}

fn request() -> ForecastScoreRequest {
    ForecastScoreRequest {
        score_id: "score1390publish".to_string(),
        forecast_id: "forecast1390publish".to_string(),
        forecast_version: 1,
        current_forecast_version: 1,
        market_id: "market1390publish".to_string(),
        outcome_id: "outcome1390publish".to_string(),
        source: ForecastSource::CalyxNative,
        provider: None,
        probability: 0.8,
        confidence: 0.6,
        forecast_ts: 100,
        scored_ts: 220,
        horizon_secs: 100,
        sufficiency_state: "sufficient".to_string(),
        previous_probability: Some(0.7),
        forecast_artifact_hash: HASH.to_string(),
        outcome: ResolvedOutcome {
            outcome_id: "outcome1390publish".to_string(),
            resolved: true,
            actual_win: true,
            resolved_ts: 200,
            source: "uma-onchain".to_string(),
            version: 1,
        },
        calibration_bin_count: 10,
    }
}
