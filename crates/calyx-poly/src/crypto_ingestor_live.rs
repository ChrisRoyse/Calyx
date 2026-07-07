use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{CxId, VaultId, VaultStore};

use crate::clob_client::{ClobClient, ClobClientConfig};
use crate::crypto_forecast_registration::{
    CryptoForecastRegistrationRequest, register_crypto_pending_for_mode,
};
use crate::crypto_ingestor::{
    CRYPTO_INGESTOR_SCHEMA_VERSION, CryptoIngestionRun, CryptoIngestorConfig, CryptoMarketInputs,
    CryptoSnapshotIngestRecord, ERR_CRYPTO_INGESTOR_INVALID_CONFIG, ERR_CRYPTO_INGESTOR_NO_MARKET,
    build_crypto_market_snapshots, ingestor_error, put_crypto_snapshot, reject_forbidden_drive,
    validate_pre_resolution,
};
use crate::data_api_client::{DataApiClient, DataApiClientConfig};
use crate::error::Result;
use crate::gamma_client::{
    GammaClient, GammaClientConfig, GammaMarketRecord, GammaMarketsPage, GammaMarketsRequest,
    GammaOutcomeShape,
};
use crate::gamma_public_search::{GammaPublicSearchPage, GammaPublicSearchRequest};
use crate::lenses::default_panel;
use crate::pending_forecast_register::{PendingForecastLedgerStore, PendingForecastRegister};
use crate::ws_market_client::{MarketWsClient, require_market_ws_session_data};
use crate::ws_market_report::{
    MarketWsCaptureReport, MarketWsProofContext, write_market_ws_capture_report,
};
use crate::ws_market_types::{MARKET_WS_DOCS_URL, MarketWsClientConfig, MarketWsSubscription};

pub struct CryptoLiveCaptureRun {
    pub run: CryptoIngestionRun,
    pub gamma_page: GammaMarketsPage,
    pub gamma_public_search_pages: Vec<GammaPublicSearchPage>,
    pub books: Vec<crate::clob_client::ClobBookPage>,
    pub holders: crate::data_api_client::DataApiHoldersPage,
    pub trades: crate::data_api_client::DataApiTradesPage,
}

pub fn run_live_crypto_ingestion_cycle<S>(
    store: &S,
    register: &mut PendingForecastRegister,
    vault_id: VaultId,
    vault_salt: &[u8],
    output_root: &Path,
    config: CryptoIngestorConfig,
) -> Result<CryptoLiveCaptureRun>
where
    S: VaultStore + PendingForecastLedgerStore,
{
    reject_forbidden_drive(output_root)?;
    validate_config(&config)?;
    let captured_ts = capture_ts(config.captured_ts)?;
    let gamma = GammaClient::new(GammaClientConfig::default())?;
    let gamma_page =
        gamma.fetch_markets(&GammaMarketsRequest::crypto_active(config.market_limit))?;
    let gamma_public_search_pages = fetch_public_search_pages(&gamma, &config)?;
    let markets = merged_candidate_markets(&gamma_page, &gamma_public_search_pages);
    let market = select_crypto_capture_market(&markets, captured_ts, &config)?.clone();
    let tokens = market
        .clob_token_ids
        .iter()
        .take(config.outcome_limit_per_market)
        .cloned()
        .collect::<Vec<_>>();

    let book_pages = fetch_books(&tokens)?;
    let data = DataApiClient::new(DataApiClientConfig::default())?;
    let holders = data.fetch_holders(&market.condition_id, config.holder_limit)?;
    let trades = data.fetch_trades_by_market(&market.condition_id, config.trade_limit, 0)?;
    let ws_report = if config.capture_ws {
        Some(capture_ws_report(output_root, &tokens, &config.ws_config)?)
    } else {
        None
    };

    let inputs = CryptoMarketInputs {
        market,
        books: book_pages.iter().map(|page| page.book.clone()).collect(),
        holders: holders.holder_shares(),
        trades: trades.trades.clone(),
        captured_ts,
    };
    let panel = default_panel(config.panel_version, config.region_vocab.clone());
    let mut records = Vec::new();
    for snapshot in build_crypto_market_snapshots(&inputs)? {
        let put = put_crypto_snapshot(store, &panel, &snapshot, vault_id, vault_salt)?;
        let cx_id = CxId::from_input(
            &snapshot.canonical_input_bytes()?,
            config.panel_version,
            vault_salt,
        );
        let pending = register_crypto_pending_for_mode(
            store,
            register,
            CryptoForecastRegistrationRequest {
                snapshot: &snapshot,
                cx_id,
                domain: &config.domain,
                horizon_bucket: &config.horizon_bucket,
                output_root,
                mode: config.forecast_mode,
            },
        )?;
        records.push(CryptoSnapshotIngestRecord { put, pending });
    }

    Ok(CryptoLiveCaptureRun {
        run: CryptoIngestionRun {
            schema_version: CRYPTO_INGESTOR_SCHEMA_VERSION.to_string(),
            domain: config.domain,
            captured_ts,
            market_id: inputs.market.market_id.clone(),
            condition_id: inputs.market.condition_id.clone(),
            token_count: records.len(),
            snapshots: records,
            ws_report,
        },
        gamma_page,
        gamma_public_search_pages,
        books: book_pages,
        holders,
        trades,
    })
}

fn fetch_public_search_pages(
    gamma: &GammaClient,
    config: &CryptoIngestorConfig,
) -> Result<Vec<GammaPublicSearchPage>> {
    config
        .public_search_queries
        .iter()
        .map(|query| {
            gamma.fetch_public_search_markets(&GammaPublicSearchRequest::new(
                query,
                config.public_search_limit_per_type,
            ))
        })
        .collect()
}

fn merged_candidate_markets(
    gamma_page: &GammaMarketsPage,
    search_pages: &[GammaPublicSearchPage],
) -> Vec<GammaMarketRecord> {
    let mut seen = BTreeSet::new();
    let mut markets = Vec::new();
    for market in gamma_page
        .markets
        .iter()
        .chain(search_pages.iter().flat_map(|page| page.markets.iter()))
    {
        if seen.insert(market.market_id.clone()) {
            markets.push(market.clone());
        }
    }
    markets
}

fn fetch_books(tokens: &[String]) -> Result<Vec<crate::clob_client::ClobBookPage>> {
    let clob = ClobClient::new(ClobClientConfig::default())?;
    let mut pages = Vec::with_capacity(tokens.len());
    for token in tokens {
        pages.push(clob.fetch_book(token)?);
    }
    Ok(pages)
}

fn capture_ws_report(
    output_root: &Path,
    tokens: &[String],
    config: &MarketWsClientConfig,
) -> Result<MarketWsCaptureReport> {
    let subscription = MarketWsSubscription::new(tokens.to_vec());
    let client = MarketWsClient::new(config.clone())?;
    let session = client.capture_window(&subscription, 0)?;
    require_market_ws_session_data(&session, config)?;
    write_market_ws_capture_report(
        &output_root.join("market-ws"),
        &subscription,
        config,
        vec![session],
        MarketWsProofContext {
            proof_claim: "public market WebSocket capture joins the crypto ingestor cycle"
                .to_string(),
            selected_corpus:
                "one live active crypto market subscription with its outcome token ids".to_string(),
            why_smaller_insufficient: "zero WebSocket sessions would not prove the #29 handoff"
                .to_string(),
            why_larger_wasteful: "more sessions repeat the same public subscription/readback path"
                .to_string(),
            source_docs: vec![MARKET_WS_DOCS_URL.to_string()],
        },
    )
}

pub fn select_crypto_capture_market<'a>(
    markets: &'a [GammaMarketRecord],
    captured_ts: u64,
    config: &CryptoIngestorConfig,
) -> Result<&'a GammaMarketRecord> {
    let excluded_condition_ids = excluded_condition_ids(config);
    markets
        .iter()
        .filter(|market| {
            market.enable_order_book.unwrap_or(true)
                && !excluded_condition_ids.contains(&market.condition_id.to_ascii_lowercase())
                && market.outcome_shape == GammaOutcomeShape::Binary
                && market.clob_token_ids.len() >= 2
                && validate_pre_resolution(market, captured_ts).is_ok()
                && secs_to_resolution(market, captured_ts)
                    .is_some_and(|secs| within_resolution_window(secs, config))
        })
        .filter_map(|market| {
            secs_to_resolution(market, captured_ts).map(|secs| (secs, market))
        })
        .min_by_key(|(secs, _market)| *secs)
        .map(|(_secs, market)| market)
        .ok_or_else(|| {
            ingestor_error(
                ERR_CRYPTO_INGESTOR_NO_MARKET,
                "Gamma returned no active pre-resolution binary crypto market with CLOB tokens in the configured resolution window",
            )
        })
}

fn excluded_condition_ids(config: &CryptoIngestorConfig) -> BTreeSet<String> {
    config
        .excluded_condition_ids
        .iter()
        .map(|condition| condition.trim().to_ascii_lowercase())
        .filter(|condition| !condition.is_empty())
        .collect()
}

fn secs_to_resolution(market: &GammaMarketRecord, captured_ts: u64) -> Option<u64> {
    market
        .end_ts
        .and_then(|end_ts| end_ts.checked_sub(captured_ts))
}

fn within_resolution_window(secs: u64, config: &CryptoIngestorConfig) -> bool {
    secs >= config.min_secs_to_resolution
        && config
            .max_secs_to_resolution
            .is_none_or(|max_secs| secs <= max_secs)
}

fn validate_config(config: &CryptoIngestorConfig) -> Result<()> {
    if config.market_limit == 0
        || config.outcome_limit_per_market == 0
        || config.holder_limit == 0
        || config.trade_limit == 0
        || config.public_search_limit_per_type == 0
        || config.panel_version == 0
        || config.domain.trim().is_empty()
        || config.horizon_bucket.trim().is_empty()
    {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_INVALID_CONFIG,
            "market/outcome/holder/trade limits, panel version, domain, and horizon are required",
        ));
    }
    if config.public_search_limit_per_type > 20 || config.public_search_queries.len() > 8 {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_INVALID_CONFIG,
            "public-search limit_per_type must be <= 20 and query count must be <= 8",
        ));
    }
    if config
        .public_search_queries
        .iter()
        .any(|query| query.trim().is_empty() || query.len() > 80)
    {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_INVALID_CONFIG,
            "public-search queries must be non-empty and at most 80 chars",
        ));
    }
    if config.excluded_condition_ids.len() > 256
        || config
            .excluded_condition_ids
            .iter()
            .any(|condition| condition.trim().is_empty() || condition.len() > 128)
    {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_INVALID_CONFIG,
            "excluded_condition_ids must contain 1..=128 byte ids and at most 256 entries",
        ));
    }
    if config
        .max_secs_to_resolution
        .is_some_and(|max_secs| max_secs < config.min_secs_to_resolution)
    {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_INVALID_CONFIG,
            format!(
                "max_secs_to_resolution must be >= min_secs_to_resolution ({} < {})",
                config.max_secs_to_resolution.unwrap_or_default(),
                config.min_secs_to_resolution
            ),
        ));
    }
    Ok(())
}

fn capture_ts(value: u64) -> Result<u64> {
    if value != 0 {
        return Ok(value);
    }
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| ingestor_error(ERR_CRYPTO_INGESTOR_INVALID_CONFIG, err.to_string()))?
        .as_secs())
}
