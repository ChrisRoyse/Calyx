//! Issue #237 - CalyxNative accumulation without duplicate pending conditions.
//!
//! Source of truth: prior capture-state JSON files for the exclusion set, plus the newly persisted
//! live capture-state JSON and CalyxNative forecast artifacts when a distinct market is available.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;
use calyx_poly::crypto_capture_harness::{
    CRYPTO_CAPTURE_STATE_FILE, CryptoCaptureHarnessConfig, CryptoCaptureHarnessRequest,
    CryptoCaptureHarnessState, LiveCryptoCaptureRunner, read_crypto_capture_state,
    run_crypto_capture_harness_once,
};
use calyx_poly::crypto_forecast_registration::CryptoForecastRegistrationMode;
use calyx_poly::crypto_ingestor::{
    CryptoIngestorConfig, ERR_CRYPTO_INGESTOR_NO_MARKET, select_crypto_capture_market,
};
use calyx_poly::gamma_client::{
    GammaClient, GammaClientConfig, GammaMarketRecord, GammaMarketsPage, GammaMarketsRequest,
};
use calyx_poly::gamma_public_search::{GammaPublicSearchPage, GammaPublicSearchRequest};
use calyx_poly::pending_forecast_register::PendingForecastRegister;
use calyx_poly::score::ForecastSource;
use serde_json::{Value, json};
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue237-calyx-native-accumulation";

#[test]
#[ignore = "requires live public Gamma/CLOB/Data API and prior C: capture roots"]
fn issue237_live_calyx_native_accumulation_distinct_market_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE237_ACCUMULATION_FSV_ROOT",
        "issue237-calyx-native-accumulation",
    );
    assert_c_drive(&root);
    reset_dir(&root);
    let prior = prior_capture_roots();
    let (excluded_condition_ids, prior_states) = read_excluded_conditions(&prior);
    assert!(
        !excluded_condition_ids.is_empty(),
        "at least one prior pending condition is required for #237 accumulation exclusion proof"
    );

    let now_ts = live_now();
    let ingestor_config = CryptoIngestorConfig {
        market_limit: 500,
        public_search_limit_per_type: 10,
        holder_limit: 25,
        trade_limit: 10,
        captured_ts: now_ts,
        max_secs_to_resolution: Some(14 * 24 * 60 * 60),
        panel_version: 237,
        capture_ws: false,
        forecast_mode: CryptoForecastRegistrationMode::CalyxNative,
        excluded_condition_ids: excluded_condition_ids.iter().cloned().collect(),
        ..CryptoIngestorConfig::default()
    };
    let gamma = GammaClient::new(GammaClientConfig::default()).expect("Gamma client");
    let gamma_page = gamma
        .fetch_markets(&GammaMarketsRequest::crypto_active(
            ingestor_config.market_limit,
        ))
        .expect("fetch tagged crypto markets");
    let search_pages = fetch_public_search_pages(&gamma, &ingestor_config);
    let raw_source_files = write_raw_source_files(&root, &gamma_page, &search_pages);
    let candidates = merged_candidate_markets(&gamma_page.markets, &search_pages);
    let selected = match select_crypto_capture_market(&candidates, now_ts, &ingestor_config) {
        Ok(market) => Some(market.clone()),
        Err(err) if err.code() == ERR_CRYPTO_INGESTOR_NO_MARKET => None,
        Err(err) => panic!("unexpected selector error: {err:?}"),
    };

    let vault_id: VaultId = VAULT_ID.parse().unwrap();
    let mut capture_state = Value::Null;
    let mut capture_report = Value::Null;
    let mut artifact_checks = Vec::new();
    let decision = if let Some(preview) = selected.as_ref() {
        let vault = AsterVault::new_durable(
            root.join("live-capture-vault"),
            vault_id,
            VAULT_SALT.to_vec(),
            VaultOptions::default(),
        )
        .unwrap();
        let mut register = PendingForecastRegister::default();
        let run = run_crypto_capture_harness_once(
            &vault,
            &mut register,
            CryptoCaptureHarnessRequest {
                vault_id,
                vault_salt: VAULT_SALT,
                output_root: &root,
                config: CryptoCaptureHarnessConfig {
                    interval_secs: 60,
                    ingestor_config: ingestor_config.clone(),
                },
                now_ts,
            },
            &mut LiveCryptoCaptureRunner,
        )
        .expect("live distinct CalyxNative capture");
        vault.flush().unwrap();
        assert_eq!(run.state.captures.len(), 1);
        assert!(!excluded_condition_ids.contains(&run.state.captures[0].condition_id));
        assert_eq!(run.state.captures[0].condition_id, preview.condition_id);
        artifact_checks = assert_calyx_native_artifacts(&run.state);
        capture_state = serde_json::to_value(&run.state).unwrap();
        capture_report = serde_json::to_value(&run.report).unwrap();
        "captured_distinct_market"
    } else {
        "no_distinct_market_available"
    };

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 237,
        "proof_claim": "A live #237 accumulation run can avoid duplicate pending CalyxNative conditions by reading prior capture-state files into the selector exclusion set, and can capture the nearest distinct eligible market when one exists.",
        "minimum_sufficient_proof_corpus": {
            "prior_capture_states": prior_states.len(),
            "excluded_condition_count": excluded_condition_ids.len(),
            "live_due_passes": if selected.is_some() { 1 } else { 0 },
            "selected_market_count": if selected.is_some() { 1 } else { 0 },
            "why_this_is_sufficient": "One prior-condition exclusion plus one bounded live selector/capture pass proves the accumulation invariant needed by #237 without sweeping markets or duplicating pending observations.",
            "why_smaller_is_insufficient": "Zero prior capture states would not prove duplicate exclusion; zero candidate discovery would not prove whether a distinct live market is available.",
            "why_larger_is_wasteful": "Capturing many markets would repeat the same CalyxNative artifact path and spend disk before those markets can resolve; #237 needs distinct real observations over time."
        },
        "source_of_truth": {
            "prior_capture_roots": prior.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "raw_source_files": raw_source_files,
            "new_capture_root": root.display().to_string()
        },
        "excluded_condition_ids": excluded_condition_ids.iter().cloned().collect::<Vec<_>>(),
        "candidate_counts": {
            "tagged_crypto": gamma_page.markets.len(),
            "public_search_pages": search_pages.len(),
            "merged_candidates": candidates.len()
        },
        "selected_preview": selected,
        "decision": decision,
        "capture_state": capture_state,
        "capture_report": capture_report,
        "artifact_checks": artifact_checks,
        "physical_files": files
    });
    let report_path = root.join("issue237_calyx_native_accumulation_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback, report);
    write_blake3sums(&root);
    println!(
        "ISSUE237_CALYX_NATIVE_ACCUMULATION_READBACK={}",
        report_path.display()
    );
}

fn write_raw_source_files(
    root: &Path,
    gamma_page: &GammaMarketsPage,
    search_pages: &[GammaPublicSearchPage],
) -> Vec<Value> {
    let mut files = Vec::new();
    let tagged = root.join("gamma-crypto-active-body.json");
    fs::write(&tagged, &gamma_page.raw_body).expect("write tagged Gamma body");
    files.push(json!({
        "path": tagged.display().to_string(),
        "url": gamma_page.url,
        "status_code": gamma_page.status_code,
        "body_bytes": gamma_page.body_bytes,
        "body_sha256": gamma_page.body_sha256,
        "market_count": gamma_page.markets.len()
    }));
    for (index, page) in search_pages.iter().enumerate() {
        let path = root.join(format!("gamma-public-search-{index}.json"));
        fs::write(&path, &page.raw_body).expect("write public-search Gamma body");
        files.push(json!({
            "path": path.display().to_string(),
            "url": page.url,
            "status_code": page.status_code,
            "body_bytes": page.body_bytes,
            "body_sha256": page.body_sha256,
            "market_count": page.markets.len()
        }));
    }
    files
}

fn prior_capture_roots() -> Vec<PathBuf> {
    if let Some(raw) = std::env::var_os("POLY_ISSUE237_EXCLUDED_CAPTURE_ROOTS") {
        return raw
            .to_string_lossy()
            .split(';')
            .filter(|part| !part.trim().is_empty())
            .map(PathBuf::from)
            .collect();
    }
    let fsv = repo_root().join("target").join("fsv");
    let mut roots = fs::read_dir(&fsv)
        .expect("read target/fsv")
        .filter_map(|entry| {
            let path = entry.expect("read FSV entry").path();
            (path.is_dir()
                && is_live_accumulation_root(&path)
                && path.join(CRYPTO_CAPTURE_STATE_FILE).exists())
            .then_some(path)
        })
        .collect::<Vec<_>>();
    roots.sort();
    roots
}

fn is_live_accumulation_root(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.contains("live") || name.contains("accumulation")
}

fn read_excluded_conditions(roots: &[PathBuf]) -> (BTreeSet<String>, Vec<Value>) {
    let mut excluded = BTreeSet::new();
    let mut states = Vec::new();
    for root in roots {
        assert_c_drive(root);
        let state_path = root.join(CRYPTO_CAPTURE_STATE_FILE);
        if !state_path.exists() {
            continue;
        }
        let state = read_crypto_capture_state(&state_path).expect("read prior capture state");
        let calyx_native_conditions = state
            .captures
            .iter()
            .filter(|capture| {
                capture
                    .snapshots
                    .iter()
                    .any(|snapshot| snapshot.pending_entry.source == ForecastSource::CalyxNative)
            })
            .map(|capture| capture.condition_id.clone())
            .collect::<Vec<_>>();
        for condition_id in &calyx_native_conditions {
            excluded.insert(condition_id.clone());
        }
        if !calyx_native_conditions.is_empty() {
            states.push(json!({
                "root": root.display().to_string(),
                "capture_count": state.captures.len(),
                "calyx_native_condition_ids": calyx_native_conditions
            }));
        }
    }
    (excluded, states)
}

fn fetch_public_search_pages(
    gamma: &GammaClient,
    config: &CryptoIngestorConfig,
) -> Vec<GammaPublicSearchPage> {
    config
        .public_search_queries
        .iter()
        .map(|query| {
            gamma
                .fetch_public_search_markets(&GammaPublicSearchRequest::new(
                    query,
                    config.public_search_limit_per_type,
                ))
                .expect("fetch public-search markets")
        })
        .collect()
}

fn merged_candidate_markets(
    tagged: &[GammaMarketRecord],
    search_pages: &[GammaPublicSearchPage],
) -> Vec<GammaMarketRecord> {
    let mut seen = BTreeSet::new();
    let mut markets = Vec::new();
    for market in tagged
        .iter()
        .chain(search_pages.iter().flat_map(|page| page.markets.iter()))
    {
        if seen.insert(market.market_id.clone()) {
            markets.push(market.clone());
        }
    }
    markets
}

fn assert_calyx_native_artifacts(state: &CryptoCaptureHarnessState) -> Vec<Value> {
    state.captures[0]
        .snapshots
        .iter()
        .map(|snapshot| {
            assert_eq!(snapshot.pending_entry.source, ForecastSource::CalyxNative);
            let path = snapshot
                .forecast_artifact_path
                .as_ref()
                .expect("artifact path");
            let expected = snapshot
                .forecast_artifact_blake3
                .as_ref()
                .expect("artifact hash");
            let actual = blake3::hash(&fs::read(path).expect("read artifact"))
                .to_hex()
                .to_string();
            assert_eq!(&actual, expected);
            json!({
                "forecast_id": snapshot.forecast_id,
                "condition_id": snapshot.pending_entry.condition_id,
                "token_id": snapshot.token_id,
                "artifact_path": path,
                "artifact_blake3": actual,
                "p_model": snapshot.pending_entry.p_model,
                "confidence": snapshot.pending_entry.confidence
            })
        })
        .collect()
}

fn live_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn assert_c_drive(path: &Path) {
    let text = PathBuf::from(path).display().to_string().replace('/', "\\");
    assert!(
        text.to_ascii_lowercase().starts_with("c:\\"),
        "FSV root must stay on C:, got {text}"
    );
}
