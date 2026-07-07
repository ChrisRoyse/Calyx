use serde_json::Value;

use crate::Result;
use crate::book_liquidity::PublicBookLevel;
use crate::clob_types::ClobSide;
use crate::ws_market_types::{
    ERR_WS_MARKET_EVENT_INVALID, ERR_WS_MARKET_JSON, MarketWsBestBidAsk, MarketWsBook,
    MarketWsControlMessage, MarketWsLastTradePrice, MarketWsLifecycleEvent, MarketWsParsedEvent,
    MarketWsPriceChange, MarketWsPriceChangeLevel, MarketWsTextEnvelope, MarketWsTickSizeChange,
    MarketWsUnknownEvent, ws_market_error,
};

pub fn parse_market_ws_text(text: &str) -> Result<MarketWsTextEnvelope> {
    if text == "PONG" {
        return Ok(MarketWsTextEnvelope {
            control: Some(MarketWsControlMessage::Pong),
            events: Vec::new(),
        });
    }
    let value = serde_json::from_str::<Value>(text).map_err(|err| {
        ws_market_error(
            ERR_WS_MARKET_JSON,
            format!("decode market WebSocket text frame: {err}"),
        )
    })?;
    let items = match value {
        Value::Object(_) => vec![value],
        Value::Array(items) => items,
        _ => {
            return Err(ws_market_error(
                ERR_WS_MARKET_EVENT_INVALID,
                "market WebSocket JSON frame must be an object or array of objects",
            ));
        }
    };
    let events = items.iter().map(parse_event_value).collect::<Result<_>>()?;
    Ok(MarketWsTextEnvelope {
        control: None,
        events,
    })
}

fn parse_event_value(value: &Value) -> Result<MarketWsParsedEvent> {
    let event_type = required_string(value, &["event_type"])?;
    match event_type.as_str() {
        "book" => Ok(MarketWsParsedEvent::Book(parse_book(value)?)),
        "price_change" => Ok(MarketWsParsedEvent::PriceChange(parse_price_change(value)?)),
        "last_trade_price" => Ok(MarketWsParsedEvent::LastTradePrice(parse_last_trade_price(
            value,
        )?)),
        "best_bid_ask" => Ok(MarketWsParsedEvent::BestBidAsk(parse_best_bid_ask(value)?)),
        "tick_size_change" => Ok(MarketWsParsedEvent::TickSizeChange(parse_tick_size_change(
            value,
        )?)),
        "new_market" | "market_resolved" => Ok(MarketWsParsedEvent::Lifecycle(parse_lifecycle(
            value, event_type,
        ))),
        _ => Ok(MarketWsParsedEvent::Unknown(MarketWsUnknownEvent {
            event_type,
            raw: value.clone(),
        })),
    }
}

fn parse_book(value: &Value) -> Result<MarketWsBook> {
    Ok(MarketWsBook {
        asset_id: required_string(value, &["asset_id", "assetId"])?,
        market: optional_string(value, &["market"])?,
        timestamp_ms: optional_u64(value, &["timestamp", "timestamp_ms"])?,
        hash: optional_string(value, &["hash"])?,
        bids: parse_levels(value, "bids")?,
        asks: parse_levels(value, "asks")?,
    })
}

fn parse_price_change(value: &Value) -> Result<MarketWsPriceChange> {
    let raw_changes = value
        .get("price_changes")
        .or_else(|| value.get("changes"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ws_market_error(
                ERR_WS_MARKET_EVENT_INVALID,
                "price_change frame missing price_changes array",
            )
        })?;
    let changes = raw_changes
        .iter()
        .map(parse_price_change_level)
        .collect::<Result<Vec<_>>>()?;
    if changes.is_empty() {
        return Err(ws_market_error(
            ERR_WS_MARKET_EVENT_INVALID,
            "price_change frame must contain at least one price level change",
        ));
    }
    Ok(MarketWsPriceChange {
        market: optional_string(value, &["market"])?,
        timestamp_ms: optional_u64(value, &["timestamp", "timestamp_ms"])?,
        changes,
    })
}

fn parse_price_change_level(value: &Value) -> Result<MarketWsPriceChangeLevel> {
    let size = required_decimal(value, &["size"])?;
    if size < 0.0 {
        return Err(ws_market_error(
            ERR_WS_MARKET_EVENT_INVALID,
            "price_change size must be finite and non-negative",
        ));
    }
    Ok(MarketWsPriceChangeLevel {
        asset_id: required_string(value, &["asset_id", "assetId"])?,
        side: required_side(value, &["side"])?,
        price: bounded_price(required_decimal(value, &["price"])?, "price_change price")?,
        size,
        removes_level: size == 0.0,
        hash: optional_string(value, &["hash"])?,
        best_bid: optional_price(value, &["best_bid", "bestBid"])?,
        best_ask: optional_price(value, &["best_ask", "bestAsk"])?,
    })
}

fn parse_last_trade_price(value: &Value) -> Result<MarketWsLastTradePrice> {
    Ok(MarketWsLastTradePrice {
        asset_id: required_string(value, &["asset_id", "assetId"])?,
        market: optional_string(value, &["market"])?,
        price: bounded_price(
            required_decimal(value, &["price"])?,
            "last_trade_price price",
        )?,
        size: optional_nonnegative(value, &["size"])?,
        side: optional_side(value, &["side"])?,
        fee_rate_bps: optional_nonnegative(value, &["fee_rate_bps", "feeRateBps"])?,
        timestamp_ms: optional_u64(value, &["timestamp", "timestamp_ms"])?,
        transaction_hash: optional_string(value, &["transaction_hash", "transactionHash"])?,
    })
}

fn parse_best_bid_ask(value: &Value) -> Result<MarketWsBestBidAsk> {
    Ok(MarketWsBestBidAsk {
        asset_id: required_string(value, &["asset_id", "assetId"])?,
        market: optional_string(value, &["market"])?,
        best_bid: optional_price(value, &["best_bid", "bestBid"])?,
        best_ask: optional_price(value, &["best_ask", "bestAsk"])?,
        timestamp_ms: optional_u64(value, &["timestamp", "timestamp_ms"])?,
    })
}

fn parse_tick_size_change(value: &Value) -> Result<MarketWsTickSizeChange> {
    Ok(MarketWsTickSizeChange {
        asset_id: optional_string(value, &["asset_id", "assetId"])?,
        market: optional_string(value, &["market"])?,
        old_tick_size: optional_price(value, &["old_tick_size", "oldTickSize"])?,
        new_tick_size: bounded_price(
            required_decimal(
                value,
                &["new_tick_size", "newTickSize", "tick_size", "tickSize"],
            )?,
            "tick size",
        )?,
        timestamp_ms: optional_u64(value, &["timestamp", "timestamp_ms"])?,
    })
}

fn parse_lifecycle(value: &Value, event_type: String) -> MarketWsLifecycleEvent {
    MarketWsLifecycleEvent {
        event_type,
        market: optional_string(value, &["market"]).ok().flatten(),
        asset_id: optional_string(value, &["asset_id", "assetId"])
            .ok()
            .flatten(),
        condition_id: optional_string(value, &["condition_id", "conditionId"])
            .ok()
            .flatten(),
        raw: value.clone(),
    }
}

fn parse_levels(value: &Value, field: &str) -> Result<Vec<PublicBookLevel>> {
    let rows = value.get(field).and_then(Value::as_array).ok_or_else(|| {
        ws_market_error(
            ERR_WS_MARKET_EVENT_INVALID,
            format!("book frame missing {field} array"),
        )
    })?;
    rows.iter()
        .map(|row| {
            let price = bounded_price(required_decimal(row, &["price", "p"])?, field)?;
            let size = required_decimal(row, &["size", "s"])?;
            if size <= 0.0 {
                return Err(ws_market_error(
                    ERR_WS_MARKET_EVENT_INVALID,
                    format!("{field} level size must be positive"),
                ));
            }
            Ok(PublicBookLevel { price, size })
        })
        .collect()
}

fn required_string(value: &Value, fields: &[&str]) -> Result<String> {
    optional_string(value, fields)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            ws_market_error(
                ERR_WS_MARKET_EVENT_INVALID,
                format!("market WebSocket event missing string field {fields:?}"),
            )
        })
}

fn optional_string(value: &Value, fields: &[&str]) -> Result<Option<String>> {
    for field in fields {
        if let Some(raw) = value.get(*field) {
            return match raw {
                Value::Null => Ok(None),
                Value::String(text) => Ok(Some(text.clone())),
                Value::Number(number) => Ok(Some(number.to_string())),
                other => Err(ws_market_error(
                    ERR_WS_MARKET_EVENT_INVALID,
                    format!("field {field} expected string-compatible value, got {other}"),
                )),
            };
        }
    }
    Ok(None)
}

fn required_decimal(value: &Value, fields: &[&str]) -> Result<f64> {
    optional_decimal(value, fields)?.ok_or_else(|| {
        ws_market_error(
            ERR_WS_MARKET_EVENT_INVALID,
            format!("market WebSocket event missing numeric field {fields:?}"),
        )
    })
}

fn optional_decimal(value: &Value, fields: &[&str]) -> Result<Option<f64>> {
    for field in fields {
        if let Some(raw) = value.get(*field) {
            return match raw {
                Value::Null => Ok(None),
                Value::Number(number) => Ok(number.as_f64().filter(|n| n.is_finite())),
                Value::String(text) => parse_decimal_text(text).map(Some),
                other => Err(ws_market_error(
                    ERR_WS_MARKET_EVENT_INVALID,
                    format!("field {field} expected numeric-compatible value, got {other}"),
                )),
            };
        }
    }
    Ok(None)
}

fn parse_decimal_text(text: &str) -> Result<f64> {
    let trimmed = text.trim();
    let normalized = if let Some(rest) = trimmed.strip_prefix('.') {
        format!("0.{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("-.") {
        format!("-0.{rest}")
    } else {
        trimmed.to_string()
    };
    normalized
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            ws_market_error(
                ERR_WS_MARKET_EVENT_INVALID,
                format!("malformed numeric string {text:?}"),
            )
        })
}

fn optional_u64(value: &Value, fields: &[&str]) -> Result<Option<u64>> {
    for field in fields {
        if let Some(raw) = value.get(*field) {
            return match raw {
                Value::Null => Ok(None),
                Value::Number(number) => Ok(number.as_u64()),
                Value::String(text) if text.trim().is_empty() => Ok(None),
                Value::String(text) => text.trim().parse::<u64>().map(Some).map_err(|err| {
                    ws_market_error(
                        ERR_WS_MARKET_EVENT_INVALID,
                        format!("field {field} expected integer timestamp: {err}"),
                    )
                }),
                other => Err(ws_market_error(
                    ERR_WS_MARKET_EVENT_INVALID,
                    format!("field {field} expected integer timestamp, got {other}"),
                )),
            };
        }
    }
    Ok(None)
}

fn required_side(value: &Value, fields: &[&str]) -> Result<ClobSide> {
    optional_side(value, fields)?.ok_or_else(|| {
        ws_market_error(
            ERR_WS_MARKET_EVENT_INVALID,
            format!("market WebSocket event missing side field {fields:?}"),
        )
    })
}

fn optional_side(value: &Value, fields: &[&str]) -> Result<Option<ClobSide>> {
    let Some(side) = optional_string(value, fields)? else {
        return Ok(None);
    };
    match side.trim().to_ascii_uppercase().as_str() {
        "BUY" => Ok(Some(ClobSide::Buy)),
        "SELL" => Ok(Some(ClobSide::Sell)),
        _ => Err(ws_market_error(
            ERR_WS_MARKET_EVENT_INVALID,
            format!("unsupported market WebSocket side {side:?}"),
        )),
    }
}

fn optional_price(value: &Value, fields: &[&str]) -> Result<Option<f64>> {
    optional_decimal(value, fields)?
        .map(|price| bounded_price(price, "price"))
        .transpose()
}

fn optional_nonnegative(value: &Value, fields: &[&str]) -> Result<Option<f64>> {
    optional_decimal(value, fields)?
        .map(|number| {
            if number >= 0.0 {
                Ok(number)
            } else {
                Err(ws_market_error(
                    ERR_WS_MARKET_EVENT_INVALID,
                    format!("field {fields:?} must be non-negative"),
                ))
            }
        })
        .transpose()
}

fn bounded_price(price: f64, name: &str) -> Result<f64> {
    if (0.0..=1.0).contains(&price) {
        Ok(price)
    } else {
        Err(ws_market_error(
            ERR_WS_MARKET_EVENT_INVALID,
            format!("{name} must be finite and in [0,1]"),
        ))
    }
}
