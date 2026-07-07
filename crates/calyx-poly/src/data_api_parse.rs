use serde_json::Value;

use crate::data_api_client::{ERR_DATA_API_JSON, ERR_DATA_API_ROW_INVALID, data_api_error};
use crate::data_api_types::{
    DataApiActivityRecord, DataApiEvidenceStatus, DataApiHolderGroup, DataApiHolderRecord,
    DataApiMarketPositionGroup, DataApiOpenInterestRecord, DataApiPositionRecord,
    DataApiTradeRecord, DataApiTradeSide,
};
use crate::error::Result;

pub fn parse_data_api_trades_value(value: &Value) -> Result<Vec<DataApiTradeRecord>> {
    array_rows(value, "trades")?
        .iter()
        .map(parse_trade_record)
        .collect()
}

pub fn parse_data_api_holders_value(
    value: &Value,
) -> Result<(Vec<DataApiHolderGroup>, DataApiEvidenceStatus)> {
    if value.is_null() {
        return Ok((Vec::new(), DataApiEvidenceStatus::Absent));
    }
    let groups = array_rows(value, "holders")?
        .iter()
        .map(parse_holder_group)
        .collect::<Result<Vec<_>>>()?;
    let status = if groups.iter().all(|group| group.holders.is_empty()) {
        DataApiEvidenceStatus::Absent
    } else {
        DataApiEvidenceStatus::Ready
    };
    Ok((groups, status))
}

pub fn parse_data_api_market_positions_value(
    value: &Value,
) -> Result<(Vec<DataApiMarketPositionGroup>, DataApiEvidenceStatus)> {
    let groups = array_rows(value, "market positions")?
        .iter()
        .map(parse_market_position_group)
        .collect::<Result<Vec<_>>>()?;
    let status = if groups.iter().all(|group| group.positions.is_empty()) {
        DataApiEvidenceStatus::Absent
    } else {
        DataApiEvidenceStatus::Ready
    };
    Ok((groups, status))
}

pub fn parse_data_api_positions_value(
    value: &Value,
) -> Result<(Vec<DataApiPositionRecord>, DataApiEvidenceStatus)> {
    let positions = array_rows(value, "positions")?
        .iter()
        .map(parse_position_record)
        .collect::<Result<Vec<_>>>()?;
    let status = if positions.is_empty() {
        DataApiEvidenceStatus::Absent
    } else {
        DataApiEvidenceStatus::Ready
    };
    Ok((positions, status))
}

pub fn parse_data_api_activity_value(
    value: &Value,
) -> Result<(Vec<DataApiActivityRecord>, DataApiEvidenceStatus)> {
    let activity = array_rows(value, "activity")?
        .iter()
        .map(parse_activity_record)
        .collect::<Result<Vec<_>>>()?;
    let status = if activity.is_empty() {
        DataApiEvidenceStatus::Absent
    } else {
        DataApiEvidenceStatus::Ready
    };
    Ok((activity, status))
}

pub fn parse_data_api_open_interest_value(value: &Value) -> Result<Vec<DataApiOpenInterestRecord>> {
    array_rows(value, "open interest")?
        .iter()
        .map(|row| {
            Ok(DataApiOpenInterestRecord {
                market: required_string(row, "market")?,
                value: nonnegative_number(row, "value")?,
            })
        })
        .collect()
}

fn parse_holder_group(value: &Value) -> Result<DataApiHolderGroup> {
    let token_id = required_string(value, "token")?;
    let holders = value
        .get("holders")
        .and_then(Value::as_array)
        .ok_or_else(|| row_error("holder group missing holders array"))?
        .iter()
        .map(|row| parse_holder_record(row, &token_id))
        .collect::<Result<Vec<_>>>()?;
    Ok(DataApiHolderGroup { token_id, holders })
}

fn parse_holder_record(value: &Value, token_id: &str) -> Result<DataApiHolderRecord> {
    let asset = required_string(value, "asset")?;
    if asset != token_id {
        return Err(row_error(format!(
            "holder asset {asset} did not match group token {token_id}"
        )));
    }
    Ok(DataApiHolderRecord {
        proxy_wallet: required_string(value, "proxyWallet")?,
        asset,
        amount: positive_number(value, "amount")?,
        outcome_index: required_u32(value, "outcomeIndex")?,
    })
}

fn parse_market_position_group(value: &Value) -> Result<DataApiMarketPositionGroup> {
    let token_id = required_string(value, "token")?;
    let positions = value
        .get("positions")
        .and_then(Value::as_array)
        .ok_or_else(|| row_error("market-position group missing positions array"))?
        .iter()
        .map(parse_position_record)
        .collect::<Result<Vec<_>>>()?;
    Ok(DataApiMarketPositionGroup {
        token_id,
        positions,
    })
}

fn parse_position_record(value: &Value) -> Result<DataApiPositionRecord> {
    Ok(DataApiPositionRecord {
        proxy_wallet: required_string(value, "proxyWallet")?,
        asset: required_string(value, "asset")?,
        condition_id: required_string(value, "conditionId")?,
        size: nonnegative_number(value, "size")?,
        current_price: optional_number_any(value, &["curPrice", "currPrice"])?,
        current_value: optional_number(value, "currentValue")?,
        outcome: optional_string(value, "outcome")?,
        outcome_index: required_u32(value, "outcomeIndex")?,
    })
}

fn parse_trade_record(value: &Value) -> Result<DataApiTradeRecord> {
    Ok(DataApiTradeRecord {
        proxy_wallet: required_string(value, "proxyWallet")?,
        side: parse_side(&required_string(value, "side")?)?,
        asset: required_string(value, "asset")?,
        condition_id: required_string(value, "conditionId")?,
        size: positive_number(value, "size")?,
        price: unit_number(value, "price")?,
        timestamp: required_u64(value, "timestamp")?,
        outcome_index: required_u32(value, "outcomeIndex")?,
        transaction_hash: optional_string(value, "transactionHash")?,
    })
}

fn parse_activity_record(value: &Value) -> Result<DataApiActivityRecord> {
    Ok(DataApiActivityRecord {
        proxy_wallet: required_string(value, "proxyWallet")?,
        activity_type: required_string(value, "type")?,
        condition_id: required_string(value, "conditionId")?,
        asset: optional_string(value, "asset")?,
        side: optional_string(value, "side")?
            .map(|side| parse_side(&side))
            .transpose()?,
        size: optional_positive_number(value, "size")?,
        usdc_size: optional_positive_number(value, "usdcSize")?,
        price: optional_unit_number(value, "price")?,
        timestamp: required_u64(value, "timestamp")?,
        transaction_hash: optional_string(value, "transactionHash")?,
        outcome_index: optional_u32(value, "outcomeIndex")?,
    })
}

fn array_rows<'a>(value: &'a Value, label: &str) -> Result<&'a Vec<Value>> {
    value.as_array().ok_or_else(|| {
        data_api_error(
            ERR_DATA_API_JSON,
            format!("Data API {label} response must be an array"),
        )
    })
}

fn parse_side(value: &str) -> Result<DataApiTradeSide> {
    match value {
        "BUY" => Ok(DataApiTradeSide::Buy),
        "SELL" => Ok(DataApiTradeSide::Sell),
        other => Err(row_error(format!("unexpected Data API trade side {other}"))),
    }
}

fn required_string(value: &Value, field: &str) -> Result<String> {
    optional_string(value, field)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| row_error(format!("missing required string field {field}")))
}

fn optional_string(value: &Value, field: &str) -> Result<Option<String>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Number(number)) => Ok(Some(number.to_string())),
        Some(other) => Err(row_error(format!(
            "field {field} expected string-compatible value, got {other}"
        ))),
    }
}

fn required_u64(value: &Value, field: &str) -> Result<u64> {
    match value.get(field) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.parse::<u64>().ok(),
        _ => None,
    }
    .ok_or_else(|| row_error(format!("field {field} expected u64-compatible value")))
}

fn required_u32(value: &Value, field: &str) -> Result<u32> {
    let raw = required_u64(value, field)?;
    u32::try_from(raw).map_err(|err| row_error(format!("field {field} exceeds u32: {err}")))
}

fn optional_u32(value: &Value, field: &str) -> Result<Option<u32>> {
    value
        .get(field)
        .map(|_| required_u32(value, field))
        .transpose()
}

fn unit_number(value: &Value, field: &str) -> Result<f64> {
    let number = required_number(value, field)?;
    if !(0.0..=1.0).contains(&number) {
        return Err(row_error(format!("field {field} expected value in [0,1]")));
    }
    Ok(number)
}

fn optional_unit_number(value: &Value, field: &str) -> Result<Option<f64>> {
    optional_number(value, field)?
        .map(|number| {
            if (0.0..=1.0).contains(&number) {
                Ok(number)
            } else {
                Err(row_error(format!("field {field} expected value in [0,1]")))
            }
        })
        .transpose()
}

fn positive_number(value: &Value, field: &str) -> Result<f64> {
    let number = required_number(value, field)?;
    if number <= 0.0 {
        return Err(row_error(format!("field {field} expected positive number")));
    }
    Ok(number)
}

fn optional_positive_number(value: &Value, field: &str) -> Result<Option<f64>> {
    optional_number(value, field)?
        .map(|number| {
            if number > 0.0 {
                Ok(number)
            } else {
                Err(row_error(format!("field {field} expected positive number")))
            }
        })
        .transpose()
}

fn nonnegative_number(value: &Value, field: &str) -> Result<f64> {
    let number = required_number(value, field)?;
    if number < 0.0 {
        return Err(row_error(format!(
            "field {field} expected non-negative number"
        )));
    }
    Ok(number)
}

fn required_number(value: &Value, field: &str) -> Result<f64> {
    value
        .get(field)
        .map(|raw| number_value(raw, field))
        .transpose()?
        .ok_or_else(|| row_error(format!("missing required numeric field {field}")))
}

fn optional_number_any(value: &Value, fields: &[&str]) -> Result<Option<f64>> {
    for field in fields {
        if value.get(*field).is_some() {
            return optional_number(value, field);
        }
    }
    Ok(None)
}

fn optional_number(value: &Value, field: &str) -> Result<Option<f64>> {
    value
        .get(field)
        .map(|raw| number_value(raw, field))
        .transpose()
}

fn number_value(value: &Value, field: &str) -> Result<f64> {
    let parsed = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
    .filter(|number| number.is_finite());
    parsed.ok_or_else(|| {
        row_error(format!(
            "field {field} expected finite numeric-compatible value"
        ))
    })
}

fn row_error(message: impl Into<String>) -> crate::PolyError {
    data_api_error(ERR_DATA_API_ROW_INVALID, message)
}
