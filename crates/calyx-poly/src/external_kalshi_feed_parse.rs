use serde_json::Value;

use crate::error::{PolyError, Result};
use crate::external_kalshi_feed::{ERR_KALSHI_ENCODE_INVALID, ERR_KALSHI_MARKET_INVALID};
use crate::external_kalshi_feed_types::{
    ExternalSignalOutcomeObservation, KalshiEncodedSignal, KalshiMarketRecord,
};

pub fn parse_kalshi_markets_value(value: &Value) -> Result<Vec<KalshiMarketRecord>> {
    let rows = value
        .as_object()
        .and_then(|object| object.get("markets"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            kalshi_error(
                ERR_KALSHI_MARKET_INVALID,
                "Kalshi markets response must contain a markets array",
            )
        })?;
    rows.iter().map(parse_kalshi_market).collect()
}

pub fn parse_kalshi_market(value: &Value) -> Result<KalshiMarketRecord> {
    Ok(KalshiMarketRecord {
        ticker: required_string(value, "ticker")?,
        event_ticker: optional_string(value, "event_ticker")?,
        title: required_string(value, "title")?,
        subtitle: optional_string(value, "subtitle")?,
        status: required_string(value, "status")?,
        market_type: optional_string(value, "market_type")?,
        close_time: optional_string(value, "close_time")?,
        expiration_time: optional_string(value, "expiration_time")?,
        settlement_ts: optional_string(value, "settlement_ts")?,
        result: optional_string(value, "result")?,
        expiration_value: optional_string(value, "expiration_value")?,
        yes_bid_dollars: optional_decimal(value, "yes_bid_dollars")?,
        yes_ask_dollars: optional_decimal(value, "yes_ask_dollars")?,
        no_bid_dollars: optional_decimal(value, "no_bid_dollars")?,
        no_ask_dollars: optional_decimal(value, "no_ask_dollars")?,
        last_price_dollars: optional_decimal(value, "last_price_dollars")?,
        previous_price_dollars: optional_decimal(value, "previous_price_dollars")?,
        settlement_value_dollars: optional_decimal(value, "settlement_value_dollars")?,
        liquidity_dollars: optional_decimal(value, "liquidity_dollars")?,
        volume_fp: optional_decimal(value, "volume_fp")?,
        volume_24h_fp: optional_decimal(value, "volume_24h_fp")?,
        open_interest_fp: optional_decimal(value, "open_interest_fp")?,
    })
}

pub fn encode_kalshi_market_signal(market: &KalshiMarketRecord) -> Result<KalshiEncodedSignal> {
    let (price, spread) = primary_yes_signal(market)?;
    let values = vec![
        price as f32,
        spread as f32,
        log1p_feature(market.liquidity_dollars)?,
        log1p_feature(market.volume_fp)?,
        log1p_feature(market.open_interest_fp)?,
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(kalshi_encode_error(
            "Kalshi encoded signal contained non-finite value",
        ));
    }
    Ok(KalshiEncodedSignal {
        source: "kalshi".to_string(),
        ticker: market.ticker.clone(),
        feature_names: vec![
            "yes_price_signal".to_string(),
            "yes_spread".to_string(),
            "liquidity_log1p".to_string(),
            "volume_log1p".to_string(),
            "open_interest_log1p".to_string(),
        ],
        values,
    })
}

pub fn kalshi_market_outcome_label(market: &KalshiMarketRecord) -> Option<bool> {
    if let Some(result) = normalized_result(&market.result) {
        return Some(result);
    }
    if let Some(value) = normalized_result(&market.expiration_value) {
        return Some(value);
    }
    market
        .settlement_value_dollars
        .and_then(|value| match value {
            v if v >= 0.999 => Some(true),
            v if v <= 0.001 => Some(false),
            _ => None,
        })
}

pub fn kalshi_market_signal_observations(
    markets: &[KalshiMarketRecord],
) -> Result<Vec<ExternalSignalOutcomeObservation>> {
    let mut observations = Vec::new();
    for market in markets {
        if let Some(outcome) = kalshi_market_outcome_label(market) {
            let signal = encode_kalshi_market_signal(market)?;
            observations.push(ExternalSignalOutcomeObservation {
                signal_value: signal.values[0],
                outcome,
            });
        }
    }
    Ok(observations)
}

fn primary_yes_signal(market: &KalshiMarketRecord) -> Result<(f64, f64)> {
    let spread = match (market.yes_bid_dollars, market.yes_ask_dollars) {
        (Some(bid), Some(ask)) if ask >= bid => Some(ask - bid),
        (Some(_), Some(_)) => return Err(kalshi_encode_error("yes ask was below yes bid")),
        _ => None,
    };
    let price = match (market.yes_bid_dollars, market.yes_ask_dollars, spread) {
        (Some(bid), Some(ask), Some(spread)) if spread <= 0.5 => Some((bid + ask) / 2.0),
        _ => market
            .last_price_dollars
            .or(market.previous_price_dollars)
            .or(market.yes_bid_dollars)
            .or(market.yes_ask_dollars),
    }
    .ok_or_else(|| kalshi_encode_error("Kalshi market has no usable yes price signal"))?;
    validate_probability(price, "yes price signal")?;
    if let Some(spread) = spread {
        validate_probability(spread, "yes spread")?;
    }
    Ok((price, spread.unwrap_or(0.0)))
}

fn log1p_feature(value: Option<f64>) -> Result<f32> {
    let value = value.unwrap_or(0.0);
    if !value.is_finite() || value < 0.0 {
        return Err(kalshi_encode_error("non-negative finite feature required"));
    }
    Ok(value.ln_1p() as f32)
}

fn validate_probability(value: f64, field: &str) -> Result<()> {
    if !(0.0..=1.0).contains(&value) || !value.is_finite() {
        return Err(kalshi_encode_error(format!(
            "{field} must be finite and in [0, 1], got {value}"
        )));
    }
    Ok(())
}

fn required_string(value: &Value, field: &str) -> Result<String> {
    optional_string(value, field)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            kalshi_error(
                ERR_KALSHI_MARKET_INVALID,
                format!("Kalshi market missing required field {field}"),
            )
        })
}

fn optional_string(value: &Value, field: &str) -> Result<Option<String>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Number(number)) => Ok(Some(number.to_string())),
        Some(other) => Err(kalshi_error(
            ERR_KALSHI_MARKET_INVALID,
            format!("Kalshi field {field} expected string-compatible value, got {other}"),
        )),
    }
}

fn optional_decimal(value: &Value, field: &str) -> Result<Option<f64>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => parse_decimal(text, field).map(Some),
        Some(Value::Number(number)) => number
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| {
                kalshi_error(
                    ERR_KALSHI_MARKET_INVALID,
                    format!("Kalshi numeric field {field} is malformed or non-finite"),
                )
            })
            .map(Some),
        Some(other) => Err(kalshi_error(
            ERR_KALSHI_MARKET_INVALID,
            format!("Kalshi numeric field {field} expected number/string, got {other}"),
        )),
    }
}

fn parse_decimal(text: &str, field: &str) -> Result<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            kalshi_error(
                ERR_KALSHI_MARKET_INVALID,
                format!("Kalshi numeric field {field} is malformed or non-finite"),
            )
        })
}

fn normalized_result(value: &Option<String>) -> Option<bool> {
    match value.as_deref()?.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn kalshi_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::raw_source(code, message)
}

fn kalshi_encode_error(message: impl Into<String>) -> PolyError {
    PolyError::raw_source(ERR_KALSHI_ENCODE_INVALID, message)
}
