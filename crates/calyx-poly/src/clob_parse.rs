use std::collections::BTreeMap;

use serde_json::Value;

use crate::book_liquidity::PublicBookLevel;
use crate::clob_client::{
    ERR_CLOB_BOOK_CROSSED, ERR_CLOB_BOOK_INVALID, ERR_CLOB_JSON, ERR_CLOB_SCALAR_INVALID,
    clob_error,
};
use crate::clob_types::{
    ClobBookStatus, ClobHistoryPoint, ClobLastTrade, ClobOrderBook, ClobPriceHistory,
    ClobScalarKind, ClobScalarQuote, ClobSide, ClobTokenPrices,
};
use crate::error::Result;

pub fn parse_clob_books_value(value: &Value) -> Result<Vec<ClobOrderBook>> {
    let rows = value.as_array().ok_or_else(|| {
        clob_error(
            ERR_CLOB_JSON,
            "CLOB /books response must be an array of books",
        )
    })?;
    rows.iter().map(parse_clob_order_book).collect()
}

pub fn parse_clob_order_book(value: &Value) -> Result<ClobOrderBook> {
    let condition_id = required_string(value, "market", ERR_CLOB_BOOK_INVALID)?;
    let token_id = required_string(value, "asset_id", ERR_CLOB_BOOK_INVALID)?;
    let timestamp_ms = required_u64(value, "timestamp", ERR_CLOB_BOOK_INVALID)?;
    let mut bids = parse_levels(value, "bids")?;
    let mut asks = parse_levels(value, "asks")?;
    bids.sort_by(|a, b| b.price.total_cmp(&a.price));
    asks.sort_by(|a, b| a.price.total_cmp(&b.price));
    let (best_bid, best_ask, midpoint, spread, status) = normalize_book_surface(&bids, &asks)?;

    Ok(ClobOrderBook {
        condition_id,
        token_id,
        timestamp_ms,
        hash: optional_string(value, "hash", ERR_CLOB_BOOK_INVALID)?,
        bids,
        asks,
        min_order_size: optional_number(value, "min_order_size", ERR_CLOB_BOOK_INVALID)?,
        tick_size: optional_number(value, "tick_size", ERR_CLOB_BOOK_INVALID)?,
        neg_risk: optional_bool(value, "neg_risk", ERR_CLOB_BOOK_INVALID)?,
        last_trade_price: optional_number(value, "last_trade_price", ERR_CLOB_BOOK_INVALID)?,
        best_bid,
        best_ask,
        midpoint,
        spread,
        status,
    })
}

pub fn parse_clob_scalar_value(
    token_id: &str,
    kind: ClobScalarKind,
    field: &str,
    value: &Value,
) -> Result<ClobScalarQuote> {
    Ok(ClobScalarQuote {
        token_id: token_id.to_string(),
        kind,
        value: scalar_for_kind(
            required_number(value, field, ERR_CLOB_SCALAR_INVALID)?,
            kind,
        )?,
    })
}

pub fn parse_clob_price_map_value(value: &Value) -> Result<Vec<ClobTokenPrices>> {
    let map = value
        .as_object()
        .ok_or_else(|| clob_error(ERR_CLOB_JSON, "CLOB /prices response must be an object map"))?;
    let mut rows = BTreeMap::new();
    for (token_id, raw) in map {
        let entry = raw.as_object().ok_or_else(|| {
            clob_error(
                ERR_CLOB_SCALAR_INVALID,
                format!("CLOB /prices token {token_id} must map to a side object"),
            )
        })?;
        let buy = optional_map_number(entry, "BUY", ERR_CLOB_SCALAR_INVALID)?
            .map(|price| scalar_for_kind(price, ClobScalarKind::BuyPrice))
            .transpose()?;
        let sell = optional_map_number(entry, "SELL", ERR_CLOB_SCALAR_INVALID)?
            .map(|price| scalar_for_kind(price, ClobScalarKind::SellPrice))
            .transpose()?;
        if buy.is_none() && sell.is_none() {
            return Err(clob_error(
                ERR_CLOB_SCALAR_INVALID,
                format!("CLOB /prices token {token_id} has neither BUY nor SELL"),
            ));
        }
        rows.insert(
            token_id.clone(),
            ClobTokenPrices {
                token_id: token_id.clone(),
                buy,
                sell,
            },
        );
    }
    Ok(rows.into_values().collect())
}

pub fn parse_clob_scalar_map_value(
    kind: ClobScalarKind,
    value: &Value,
) -> Result<Vec<ClobScalarQuote>> {
    let map = value.as_object().ok_or_else(|| {
        clob_error(
            ERR_CLOB_JSON,
            "CLOB scalar batch response must be an object map",
        )
    })?;
    let mut rows = BTreeMap::new();
    for (token_id, raw) in map {
        rows.insert(
            token_id.clone(),
            ClobScalarQuote {
                token_id: token_id.clone(),
                kind,
                value: scalar_for_kind(number_value(raw, "value", ERR_CLOB_SCALAR_INVALID)?, kind)?,
            },
        );
    }
    Ok(rows.into_values().collect())
}

pub fn parse_clob_history_value(token_id: &str, value: &Value) -> Result<ClobPriceHistory> {
    Ok(ClobPriceHistory {
        token_id: token_id.to_string(),
        points: parse_history_points(history_array(value)?)?,
    })
}

pub fn parse_clob_batch_history_value(value: &Value) -> Result<Vec<ClobPriceHistory>> {
    let history = value
        .as_object()
        .and_then(|map| map.get("history"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            clob_error(
                ERR_CLOB_JSON,
                "CLOB batch prices-history response must contain a history object map",
            )
        })?;
    let mut rows = BTreeMap::new();
    for (token_id, raw_points) in history {
        let points = raw_points.as_array().ok_or_else(|| {
            clob_error(
                ERR_CLOB_SCALAR_INVALID,
                format!("CLOB history for token {token_id} must be an array"),
            )
        })?;
        rows.insert(
            token_id.clone(),
            ClobPriceHistory {
                token_id: token_id.clone(),
                points: parse_history_points(points)?,
            },
        );
    }
    Ok(rows.into_values().collect())
}

pub fn parse_clob_last_trades_value(value: &Value) -> Result<Vec<ClobLastTrade>> {
    let rows = value.as_array().ok_or_else(|| {
        clob_error(
            ERR_CLOB_JSON,
            "CLOB last-trades-prices response must be an array",
        )
    })?;
    rows.iter()
        .map(|row| {
            Ok(ClobLastTrade {
                token_id: required_string(row, "token_id", ERR_CLOB_SCALAR_INVALID)?,
                side: parse_side(&required_string(row, "side", ERR_CLOB_SCALAR_INVALID)?)?,
                price: scalar_for_kind(
                    required_number(row, "price", ERR_CLOB_SCALAR_INVALID)?,
                    ClobScalarKind::BuyPrice,
                )?,
            })
        })
        .collect()
}

fn parse_levels(value: &Value, field: &str) -> Result<Vec<PublicBookLevel>> {
    let rows = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| clob_error(ERR_CLOB_BOOK_INVALID, format!("book missing {field} array")))?;
    rows.iter()
        .enumerate()
        .map(|(idx, row)| {
            let price = required_number(row, "price", ERR_CLOB_BOOK_INVALID)?;
            let size = required_number(row, "size", ERR_CLOB_BOOK_INVALID)?;
            if !(0.0..=1.0).contains(&price) || size <= 0.0 {
                return Err(clob_error(
                    ERR_CLOB_BOOK_INVALID,
                    format!("{field} level {idx} must have price in [0,1] and positive size"),
                ));
            }
            Ok(PublicBookLevel { price, size })
        })
        .collect()
}

type BookSurface = (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    ClobBookStatus,
);

fn normalize_book_surface(
    bids: &[PublicBookLevel],
    asks: &[PublicBookLevel],
) -> Result<BookSurface> {
    if bids.is_empty() || asks.is_empty() {
        return Ok((None, None, None, None, ClobBookStatus::ThinOrEmpty));
    }
    let best_bid = bids[0].price;
    let best_ask = asks[0].price;
    if best_bid >= best_ask {
        return Err(clob_error(
            ERR_CLOB_BOOK_CROSSED,
            format!("crossed or locked CLOB book: best_bid={best_bid:.6} best_ask={best_ask:.6}"),
        ));
    }
    Ok((
        Some(round12(best_bid)),
        Some(round12(best_ask)),
        Some(round12((best_bid + best_ask) / 2.0)),
        Some(round12(best_ask - best_bid)),
        ClobBookStatus::Ready,
    ))
}

fn parse_history_points(points: &[Value]) -> Result<Vec<ClobHistoryPoint>> {
    points
        .iter()
        .map(|point| {
            Ok(ClobHistoryPoint {
                t: required_u64(point, "t", ERR_CLOB_SCALAR_INVALID)?,
                p: scalar_for_kind(
                    required_number(point, "p", ERR_CLOB_SCALAR_INVALID)?,
                    ClobScalarKind::Midpoint,
                )?,
            })
        })
        .collect()
}

fn history_array(value: &Value) -> Result<&Vec<Value>> {
    value
        .as_object()
        .and_then(|map| map.get("history"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            clob_error(
                ERR_CLOB_JSON,
                "CLOB prices-history response must contain a history array",
            )
        })
}

fn scalar_for_kind(value: f64, kind: ClobScalarKind) -> Result<f64> {
    let valid = match kind {
        ClobScalarKind::BuyPrice | ClobScalarKind::SellPrice | ClobScalarKind::Midpoint => {
            (0.0..=1.0).contains(&value)
        }
        ClobScalarKind::Spread => (0.0..=1.0).contains(&value),
        ClobScalarKind::TickSize => value > 0.0 && value <= 1.0,
    };
    if !valid {
        return Err(clob_error(
            ERR_CLOB_SCALAR_INVALID,
            format!("CLOB scalar {kind:?} out of domain: {value}"),
        ));
    }
    Ok(round12(value))
}

fn required_string(value: &Value, field: &str, code: &str) -> Result<String> {
    optional_string(value, field, code)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| clob_error(code, format!("missing required string field {field}")))
}

fn optional_string(value: &Value, field: &str, code: &str) -> Result<Option<String>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Number(number)) => Ok(Some(number.to_string())),
        Some(other) => Err(clob_error(
            code,
            format!("field {field} expected string-compatible value, got {other}"),
        )),
    }
}

fn optional_bool(value: &Value, field: &str, code: &str) -> Result<Option<bool>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(other) => Err(clob_error(
            code,
            format!("field {field} expected bool, got {other}"),
        )),
    }
}

fn required_u64(value: &Value, field: &str, code: &str) -> Result<u64> {
    match value.get(field) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.parse::<u64>().ok(),
        _ => None,
    }
    .ok_or_else(|| clob_error(code, format!("field {field} expected u64-compatible value")))
}

fn required_number(value: &Value, field: &str, code: &str) -> Result<f64> {
    value
        .get(field)
        .map(|raw| number_value(raw, field, code))
        .transpose()?
        .ok_or_else(|| clob_error(code, format!("missing required numeric field {field}")))
}

fn optional_number(value: &Value, field: &str, code: &str) -> Result<Option<f64>> {
    value
        .get(field)
        .map(|raw| number_value(raw, field, code))
        .transpose()
}

fn optional_map_number(
    map: &serde_json::Map<String, Value>,
    field: &str,
    code: &str,
) -> Result<Option<f64>> {
    map.get(field)
        .map(|raw| number_value(raw, field, code))
        .transpose()
}

fn number_value(value: &Value, field: &str, code: &str) -> Result<f64> {
    let parsed = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
    .filter(|number| number.is_finite());
    parsed.ok_or_else(|| {
        clob_error(
            code,
            format!("field {field} expected finite numeric-compatible value"),
        )
    })
}

fn parse_side(value: &str) -> Result<ClobSide> {
    match value {
        "BUY" => Ok(ClobSide::Buy),
        "SELL" => Ok(ClobSide::Sell),
        other => Err(clob_error(
            ERR_CLOB_SCALAR_INVALID,
            format!("unexpected CLOB side {other}"),
        )),
    }
}

fn round12(value: f64) -> f64 {
    const SCALE: f64 = 1_000_000_000_000.0;
    let rounded = (value * SCALE).round() / SCALE;
    if rounded == 0.0 { 0.0 } else { rounded }
}
