use std::fs;
use std::path::Path;

use calyx_core::VaultId;
use calyx_poly::agent_secrets::{
    POLY_DEEPSEEK_API_KEY_NAME, POLY_DEEPSEEK_BASE_URL, POLY_DEEPSEEK_ENVIRONMENT,
    POLY_DEEPSEEK_MODEL_PRO, POLY_DEEPSEEK_PROJECT_ID, POLY_DEEPSEEK_SECRET_PATH,
};
use calyx_poly::model::MarketSnapshot;
use calyx_poly::{AgentForecastManifest, DeepSeekSecretMetadata};
use serde_json::Value;

pub(crate) fn read_manifest(path: &Path) -> AgentForecastManifest {
    serde_json::from_slice(&fs::read(path).expect("read manifest")).expect("decode manifest")
}

pub(crate) fn assert_agent_paths_exist(run_dir: &Path, manifest: &AgentForecastManifest) {
    for rel in [
        &manifest.prompt.rendered_prompt_path,
        &manifest.response.raw_response_path,
        &manifest.parsed_forecast_path,
        &manifest.parsed_forecast.rationale_path,
        &manifest.markdown_prediction_path,
    ] {
        assert!(run_dir.join(rel).exists(), "missing agent artifact {rel}");
    }
}

pub(crate) fn provider_metadata() -> DeepSeekSecretMetadata {
    DeepSeekSecretMetadata {
        project_id: POLY_DEEPSEEK_PROJECT_ID.to_string(),
        environment: POLY_DEEPSEEK_ENVIRONMENT.to_string(),
        secret_path: POLY_DEEPSEEK_SECRET_PATH.to_string(),
        api_key_name: POLY_DEEPSEEK_API_KEY_NAME.to_string(),
        key_present: true,
        key_length: 35,
        key_has_sk_prefix: true,
        key_sha256_prefix: "8e7788955344".to_string(),
        base_url: POLY_DEEPSEEK_BASE_URL.to_string(),
        model: POLY_DEEPSEEK_MODEL_PRO.to_string(),
        chat_completions_url: format!("{POLY_DEEPSEEK_BASE_URL}/chat/completions"),
    }
}

pub(crate) fn snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue096-token".to_string(),
        condition_id: "issue096-condition".to_string(),
        outcome_index: 0,
        slug: "issue096-local-forecast".to_string(),
        question: Some("Issue 096 local forecast market?".to_string()),
        event_id: Some("issue096-event".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue096".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_600_096,
        price: Some(0.71),
        mid: Some(0.70),
        best_bid: Some(0.69),
        best_ask: Some(0.71),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(150_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.02),
        one_day_change: Some(0.08),
        ofi: Some(0.36),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

pub(crate) fn hash_file(path: &Path) -> String {
    blake3::hash(&fs::read(path).expect("read artifact for hash"))
        .to_hex()
        .to_string()
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact")
        .to_string()
}

pub(crate) fn prefix(value: &str) -> String {
    value.chars().take(12).collect()
}

pub(crate) fn assert_no_trade_keys(value: &Value) {
    for key in ["authorized", "stake", "bankroll", "kelly", "order", "pnl"] {
        assert!(value.get(key).is_none(), "trade key survived: {key}");
    }
}

pub(crate) fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
