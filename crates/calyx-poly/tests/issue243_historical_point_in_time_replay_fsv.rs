//! Issue #243 - point-in-time historical replay FSV.
//!
//! Source of truth: persisted cutoff snapshots, CalyxNative forecast artifacts, score artifacts,
//! and score ledger rows read back from disk.

mod fsv_support;

use std::fs;
use std::path::Path;

use calyx_assay::TrustTag;
use calyx_core::FixedClock;
use calyx_ledger::{DirectoryLedgerStore, LedgerAppender, LedgerCfStore};
use calyx_poly::calyx_native::CalyxNativeRequest;
use calyx_poly::forecast::{ComponentKind, ForecastComponent};
use calyx_poly::historical_point_in_time_replay::{
    ERR_HISTORICAL_REPLAY_DUPLICATE, ERR_HISTORICAL_REPLAY_LOOKAHEAD_OUTCOME,
    ERR_HISTORICAL_REPLAY_MISSING_OUTCOME, ERR_HISTORICAL_REPLAY_NO_CLEAN_WINNER,
    ERR_HISTORICAL_REPLAY_POST_CUTOFF_INPUT, ERR_HISTORICAL_REPLAY_TERMINAL_INPUT,
    ERR_HISTORICAL_REPLAY_UNSUPPORTED_SHAPE, HistoricalReplayOutcome, HistoricalReplayRequest,
    HistoricalReplaySnapshot, HistoricalReplaySourceRow, run_historical_point_in_time_replay,
};
use calyx_poly::superiority::SuperiorityTiers;
use calyx_poly::{MarketSnapshot, Resolution};
use serde_json::{Value, json};

use fsv_support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const SCORED_TS: u64 = 1_783_001_000;

struct SnapshotSpec<'a> {
    id: &'a str,
    market: u32,
    outcome_index: u32,
    cutoff: u64,
    closed: bool,
    binary: bool,
    terminal_row: bool,
    includes_final: bool,
    source_ts: u64,
    cutoff_block_number: Option<u64>,
    source_block_number: Option<u64>,
}

impl<'a> SnapshotSpec<'a> {
    fn clean(id: &'a str, market: u32, outcome_index: u32, cutoff: u64) -> Self {
        Self {
            id,
            market,
            outcome_index,
            cutoff,
            closed: false,
            binary: true,
            terminal_row: false,
            includes_final: false,
            source_ts: cutoff,
            cutoff_block_number: Some(10_000 + u64::from(market)),
            source_block_number: Some(10_000 + u64::from(market)),
        }
    }
}

#[test]
fn issue243_historical_point_in_time_replay_fsv() {
    let (root, _) = named_fsv_root(
        "POLY_ISSUE243_FSV_ROOT",
        "issue243-historical-point-in-time-replay",
    );
    #[cfg(windows)]
    assert!(
        root.to_string_lossy().starts_with("C:"),
        "issue243 FSV root must stay on C:"
    );
    reset_dir(&root);

    let score_root = root.join("scores");
    let ledger_dir = root.join("score-ledger");
    let mut ledger = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).expect("open score ledger"),
        FixedClock::new(SCORED_TS),
    )
    .expect("open score ledger appender");

    let request = replay_request();
    let before = state_snapshot(&root, &score_root, &ledger_dir);
    let report = run_historical_point_in_time_replay(
        &root.join("replay-artifacts"),
        &score_root,
        &mut ledger,
        &request,
    )
    .expect("run point-in-time replay");
    let after = state_snapshot(&root, &score_root, &ledger_dir);

    assert_eq!(report.accepted_count, 6);
    assert_eq!(report.rejected_count, 8);
    assert_eq!(after["score_artifact_count"], json!(6));
    assert_eq!(after["ledger_rows"], json!(6));
    assert_eq!(before["ledger_rows"], json!(0));
    assert!(report
        .accepted
        .iter()
        .all(|item| item.forecast_readback_equal && item.outcome_joined_after_forecast_readback));

    let rejected_codes = report
        .rejected
        .iter()
        .map(|item| item.code.as_str())
        .collect::<Vec<_>>();
    for expected in [
        ERR_HISTORICAL_REPLAY_TERMINAL_INPUT,
        ERR_HISTORICAL_REPLAY_POST_CUTOFF_INPUT,
        ERR_HISTORICAL_REPLAY_MISSING_OUTCOME,
        ERR_HISTORICAL_REPLAY_NO_CLEAN_WINNER,
        ERR_HISTORICAL_REPLAY_DUPLICATE,
        ERR_HISTORICAL_REPLAY_UNSUPPORTED_SHAPE,
        ERR_HISTORICAL_REPLAY_LOOKAHEAD_OUTCOME,
    ] {
        assert!(rejected_codes.contains(&expected), "missing {expected}");
    }

    let artifact_readback = report
        .accepted
        .iter()
        .map(|accepted| {
            let forecast: Value = read_json(Path::new(&accepted.forecast_path));
            let score_dir = score_root.join(&accepted.score_manifest.score_id);
            let score_forecast: Value = read_json(&score_dir.join("forecast.json"));
            let outcome: Value = read_json(&score_dir.join("outcome.json"));
            assert_eq!(forecast["computed_at"], json!(accepted.cutoff_ts));
            assert_eq!(score_forecast["forecast_ts"], json!(accepted.cutoff_ts));
            assert!(outcome["resolved_ts"].as_u64().unwrap() > accepted.cutoff_ts);
            json!({
                "snapshot_id": accepted.snapshot_id,
                "forecast_computed_at": forecast["computed_at"],
                "score_forecast_ts": score_forecast["forecast_ts"],
                "outcome_resolved_ts": outcome["resolved_ts"],
                "brier": read_json::<Value>(&score_dir.join("score.json"))["brier"]
            })
        })
        .collect::<Vec<_>>();

    let edge_readback = report
        .rejected
        .iter()
        .map(|rejected| {
            json!({
                "snapshot_id": rejected.snapshot_id,
                "code": rejected.code,
                "artifact_written": rejected.artifact_written,
                "before_ledger_rows": before["ledger_rows"],
                "after_ledger_rows": after["ledger_rows"]
            })
        })
        .collect::<Vec<_>>();

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 243,
        "proof_claim": "Historical source artifacts can become scored forecasts only when every input row is at/before a cutoff, CalyxNative forecast artifacts are persisted/read back before outcome labels are joined, and leak guards reject terminal/post-cutoff/ambiguous rows.",
        "minimum_sufficient_proof_corpus": {
            "accepted": "3 resolved binary markets x 2 pre-resolution cutoff snapshots = 6 clean scored pairs",
            "edges": "terminal/final-outcome row, post-cutoff timestamp row, post-cutoff block row, missing outcome, lookahead outcome, no-clean-winner/void row, duplicate condition/token/cutoff, unsupported non-binary row",
            "why_this_is_sufficient": "This proves cutoff fencing, forecast artifact readback before label join, score artifact persistence, ledger rows, and every #243 leak guard without broad historical replay.",
            "why_larger_is_wasteful": "A larger history sweep would repeat the same structural contract; coverage/calibration scale is a later claim after this proof passes."
        },
        "before": before,
        "after": after,
        "report": report,
        "accepted_artifact_readback": artifact_readback,
        "edge_readback": edge_readback,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue243_historical_point_in_time_replay_fsv_report.json");
    write_json(&report_path, &summary);
    let readback: Value = read_json(&report_path);
    assert_eq!(readback, summary);
    write_blake3sums(&root);
}

fn replay_request() -> HistoricalReplayRequest {
    let mut snapshots = Vec::new();
    let mut outcomes = Vec::new();
    for market in 0..3 {
        let condition = condition(market);
        outcomes.push(outcome(
            &condition,
            market % 2,
            true,
            false,
            u64::from(5_000 + market),
        ));
        for cutoff in [1_000, 2_000] {
            let outcome_index = if market == 1 { 1 } else { 0 };
            snapshots.push(snapshot(SnapshotSpec::clean(
                &format!("pt-market{market}-{cutoff}"),
                market,
                outcome_index,
                cutoff,
            )));
        }
    }
    let mut terminal = SnapshotSpec::clean("edge-terminal", 20, 0, 1_000);
    terminal.closed = true;
    terminal.terminal_row = true;
    terminal.includes_final = true;
    snapshots.push(snapshot(terminal));
    outcomes.push(outcome(&condition(20), 0, true, false, 5_100));
    let mut post_cutoff = SnapshotSpec::clean("edge-post-cutoff", 21, 0, 1_000);
    post_cutoff.source_ts = 1_001;
    snapshots.push(snapshot(post_cutoff));
    outcomes.push(outcome(&condition(21), 0, true, false, 5_200));
    let mut post_cutoff_block = SnapshotSpec::clean("edge-post-cutoff-block", 25, 0, 1_000);
    post_cutoff_block.source_block_number =
        post_cutoff_block.cutoff_block_number.map(|block| block + 1);
    snapshots.push(snapshot(post_cutoff_block));
    outcomes.push(outcome(&condition(25), 0, true, false, 5_250));
    snapshots.push(snapshot(SnapshotSpec::clean(
        "edge-missing-outcome",
        22,
        0,
        1_000,
    )));
    snapshots.push(snapshot(SnapshotSpec::clean(
        "edge-no-clean-winner",
        23,
        0,
        1_000,
    )));
    outcomes.push(outcome(&condition(23), 0, false, true, 5_300));
    snapshots.push(snapshot(SnapshotSpec::clean("edge-duplicate", 0, 0, 1_000)));
    snapshots.push(snapshot(SnapshotSpec::clean(
        "edge-lookahead-outcome",
        26,
        0,
        1_000,
    )));
    outcomes.push(outcome(&condition(26), 0, true, false, SCORED_TS + 1));
    let mut non_binary = SnapshotSpec::clean("edge-non-binary", 24, 2, 1_000);
    non_binary.binary = false;
    snapshots.push(snapshot(non_binary));
    outcomes.push(outcome(&condition(24), 0, true, false, 5_400));
    HistoricalReplayRequest {
        domain: "crypto".to_string(),
        scored_ts: SCORED_TS,
        snapshots,
        outcomes,
    }
}

fn snapshot(spec: SnapshotSpec<'_>) -> HistoricalReplaySnapshot {
    let condition_id = condition(spec.market);
    let token_id = token(spec.market, spec.outcome_index);
    let market_snapshot = MarketSnapshot {
        token_id: token_id.clone(),
        condition_id: condition_id.clone(),
        outcome_index: spec.outcome_index,
        slug: format!("historical-market-{}", spec.market),
        question: Some(format!(
            "Will asset {} finish above threshold?",
            spec.market
        )),
        event_id: Some(format!("event-{}", spec.market)),
        category: Some("crypto".to_string()),
        region: None,
        tags: vec!["crypto".to_string(), "historical".to_string()],
        resolution_source: None,
        neg_risk: !spec.binary,
        snapshot_ts: spec.cutoff,
        price: Some(if spec.outcome_index == 1 { 0.38 } else { 0.62 }),
        mid: Some(if spec.outcome_index == 1 { 0.38 } else { 0.62 }),
        best_bid: Some(0.60),
        best_ask: Some(0.64),
        spread: Some(0.04),
        tick_size: Some(0.01),
        volume_24h: Some(10_000.0 + f64::from(spec.market)),
        liquidity: Some(50_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.02),
        ofi: Some(0.10),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(3_600.0),
        holders: Vec::new(),
        makers: Vec::new(),
        counterparty_volumes: Vec::new(),
        onchain_fills: Vec::new(),
        temporal_reference_ts: Some(spec.cutoff),
        sequence_position: Some(spec.cutoff / 1_000),
        sequence_total: Some(2),
        oracle_risk: Default::default(),
        book: Default::default(),
    };
    HistoricalReplaySnapshot {
        snapshot_id: spec.id.to_string(),
        market_id: condition_id.clone(),
        forecast_version: 1,
        cutoff_ts: spec.cutoff,
        cutoff_block_number: spec.cutoff_block_number,
        market_closed_at_cutoff: spec.closed,
        binary_market: spec.binary,
        snapshot: market_snapshot,
        source_rows: vec![HistoricalReplaySourceRow {
            source_id: format!("gamma-clob-onchain-{}", spec.id),
            observed_ts: spec.source_ts,
            block_number: spec.source_block_number,
            terminal: spec.terminal_row,
            includes_final_outcome: spec.includes_final,
        }],
        forecast_request: forecast_request(condition_id, token_id),
    }
}

fn forecast_request(condition_id: String, token_id: String) -> CalyxNativeRequest {
    CalyxNativeRequest {
        domain: "crypto".to_string(),
        condition_id,
        token_id,
        horizon_bucket: "pre_resolution".to_string(),
        components: vec![
            comp(ComponentKind::BaselineMarket, 0.62, 0.7),
            comp(ComponentKind::Structural, 0.66, 0.8),
        ],
        calibration: None,
        raw_confidence: 0.82,
        oracle_flakiness: 0.03,
        oracle_validity: 0.95,
        panel_bits: 1.0,
        anchor_entropy_bits: 1.0,
        superiority_tiers: SuperiorityTiers {
            oracle_self_consistency: 0.94,
            panel_sufficient: true,
            kernel_recall_ratio: 0.97,
            min_kernel_recall_ratio: 0.95,
            calibrated: true,
            goodhart_defended: true,
            mistake_closed: true,
        },
    }
}

fn comp(kind: ComponentKind, p: f64, reliability: f64) -> ForecastComponent {
    ForecastComponent::new(kind, p, reliability, 50, TrustTag::Trusted, kind.slug())
        .expect("known-truth component")
}

fn outcome(
    condition_id: &str,
    winner: u32,
    clean: bool,
    voided: bool,
    resolved_ts: u64,
) -> HistoricalReplayOutcome {
    HistoricalReplayOutcome {
        resolution: Resolution {
            condition_id: condition_id.to_string(),
            winning_outcome_index: winner,
            winning_label: if winner == 0 { "YES" } else { "NO" }.to_string(),
            resolved_ts,
            source: "uma-onchain".to_string(),
            disputed: false,
        },
        clean_winner: clean,
        voided,
    }
}

fn condition(market: u32) -> String {
    format!("0x243{market:061}")
}

fn token(market: u32, outcome: u32) -> String {
    format!("token243{market:02}{outcome:02}{}", "7".repeat(40))
}

fn state_snapshot(root: &Path, score_root: &Path, ledger_dir: &Path) -> Value {
    json!({
        "root": root.display().to_string(),
        "score_artifact_count": score_artifact_count(score_root),
        "ledger_rows": ledger_row_count(ledger_dir),
    })
}

fn score_artifact_count(score_root: &Path) -> usize {
    if !score_root.exists() {
        return 0;
    }
    fs::read_dir(score_root)
        .expect("read score root")
        .filter(|entry| {
            let entry = entry.as_ref().expect("entry");
            entry.file_type().expect("file type").is_dir()
                && !entry.file_name().to_string_lossy().starts_with('.')
        })
        .count()
}

fn ledger_row_count(ledger_dir: &Path) -> usize {
    if !ledger_dir.exists() {
        return 0;
    }
    DirectoryLedgerStore::open(ledger_dir)
        .expect("open ledger readback")
        .scan()
        .expect("scan ledger readback")
        .len()
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    serde_json::from_slice(&fs::read(path).expect("read JSON")).expect("decode JSON")
}
