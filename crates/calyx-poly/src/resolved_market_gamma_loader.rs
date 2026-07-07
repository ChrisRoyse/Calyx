//! Gamma closed-market loader for the local computed-kernel recall run (#223).
//!
//! The loader turns real-shaped Gamma market JSON into owned `(MarketSnapshot, Resolution)` pairs,
//! but only admits rows that prove they are pre-resolution snapshots. Terminal closed-market rows
//! are counted and rejected loud instead of being passed as low-signal examples.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{PolyError, Result};
use crate::model::{MarketSnapshot, Resolution};
use crate::resolved_market_corpus::ResolvedMarketInput;

pub const ERR_GAMMA_RECALL_READ_DIR: &str = "POLY_RECALL_RUN_READ_DIR";

/// Binary markets must have one outcome priced at/above this to be a clean resolution.
const RESOLVED_HI: f64 = 0.99;
/// The losing side must be at/below this to avoid ambiguous terminal states.
const RESOLVED_LO: f64 = 0.01;
/// Prices within this of 0 or 1 are terminal and leak the outcome.
const TERMINAL_EPS: f64 = 1e-3;

/// Per-reason admission census. Every market read is accounted for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GammaRecallCensus {
    pub files_read: usize,
    pub markets_seen: usize,
    pub skipped_not_binary_or_ids: usize,
    pub unresolved_no_clean_winner: usize,
    pub rejected_terminal_degenerate: usize,
    pub rejected_lookahead: usize,
    pub admitted: usize,
}

/// Owned loader output; callers can borrow it as `ResolvedMarketInput` for the corpus builder.
#[derive(Clone, Debug)]
pub struct GammaResolvedMarkets {
    pub markets: Vec<(MarketSnapshot, Resolution)>,
    pub census: GammaRecallCensus,
}

impl GammaResolvedMarkets {
    pub fn inputs(&self) -> Vec<ResolvedMarketInput<'_>> {
        self.markets
            .iter()
            .map(|(snapshot, resolution)| ResolvedMarketInput {
                snapshot,
                resolution,
            })
            .collect()
    }
}

/// Read every `*.json` under `dir` recursively and load admissible resolved markets.
pub fn load_admissible_markets(dir: &Path) -> Result<GammaResolvedMarkets> {
    let mut files = Vec::new();
    collect_json_files(dir, &mut files)?;
    let mut markets = Vec::new();
    let mut census = GammaRecallCensus::default();
    for file in &files {
        census.files_read += 1;
        let text = fs::read_to_string(file).map_err(|e| {
            PolyError::diagnostics(
                ERR_GAMMA_RECALL_READ_DIR,
                format!("read {}: {e}", file.display()),
            )
        })?;
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            PolyError::diagnostics(
                ERR_GAMMA_RECALL_READ_DIR,
                format!("parse {}: {e}", file.display()),
            )
        })?;
        let arr = match &value {
            Value::Array(a) => a.clone(),
            Value::Object(o) => o
                .get("data")
                .or_else(|| o.get("markets"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        for item in arr {
            census.markets_seen += 1;
            match admit_market(&item) {
                Admission::Admit(pair) => markets.push(*pair),
                Admission::SkipParse => census.skipped_not_binary_or_ids += 1,
                Admission::Unresolved => census.unresolved_no_clean_winner += 1,
                Admission::TerminalDegenerate => census.rejected_terminal_degenerate += 1,
                Admission::LookAhead => census.rejected_lookahead += 1,
            }
        }
    }
    census.admitted = markets.len();
    Ok(GammaResolvedMarkets { markets, census })
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|e| {
        PolyError::diagnostics(
            ERR_GAMMA_RECALL_READ_DIR,
            format!("read_dir {}: {e}", dir.display()),
        )
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out)?;
        } else if is_body_page_json(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_body_page_json(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("page-")
        && name.ends_with(".json")
        && !name.ends_with(".metadata.json")
        && !name.ends_with(".request.json")
}

enum Admission {
    Admit(Box<(MarketSnapshot, Resolution)>),
    SkipParse,
    Unresolved,
    TerminalDegenerate,
    LookAhead,
}

#[derive(Deserialize)]
struct GammaMarket {
    #[serde(rename = "conditionId")]
    condition_id: Option<String>,
    question: Option<String>,
    slug: Option<String>,
    category: Option<String>,
    outcomes: Option<String>,
    #[serde(rename = "outcomePrices")]
    outcome_prices: Option<String>,
    #[serde(rename = "clobTokenIds")]
    clob_token_ids: Option<String>,
    spread: Option<Value>,
    #[serde(rename = "volume24hr")]
    volume_24hr: Option<Value>,
    #[serde(rename = "liquidityNum")]
    liquidity_num: Option<Value>,
    #[serde(rename = "bestBid")]
    best_bid: Option<Value>,
    #[serde(rename = "bestAsk")]
    best_ask: Option<Value>,
    #[serde(rename = "lastTradePrice")]
    last_trade_price: Option<Value>,
    #[serde(rename = "closedTime")]
    closed_time: Option<String>,
    #[serde(rename = "endDate")]
    end_date: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
}

fn admit_market(item: &Value) -> Admission {
    let Ok(m) = serde_json::from_value::<GammaMarket>(item.clone()) else {
        return Admission::SkipParse;
    };
    let (Some(cond), Some(outcomes_s), Some(prices_s), Some(tokens_s)) = (
        &m.condition_id,
        &m.outcomes,
        &m.outcome_prices,
        &m.clob_token_ids,
    ) else {
        return Admission::SkipParse;
    };
    let outcomes = parse_str_array(outcomes_s);
    let prices = parse_num_array(prices_s);
    let tokens = parse_str_array(tokens_s);
    if outcomes.len() != 2 || prices.len() != 2 || tokens.len() != 2 {
        return Admission::SkipParse;
    }

    let winner = match (prices[0] >= RESOLVED_HI, prices[1] >= RESOLVED_HI) {
        (true, false) if prices[1] <= RESOLVED_LO => 0u32,
        (false, true) if prices[0] <= RESOLVED_LO => 1u32,
        _ => return Admission::Unresolved,
    };

    let price = num(m.last_trade_price.as_ref()).or_else(|| {
        match (num(m.best_bid.as_ref()), num(m.best_ask.as_ref())) {
            (Some(b), Some(a)) => Some((b + a) / 2.0),
            _ => None,
        }
    });
    let spread = num(m.spread.as_ref());
    let volume_24h = num(m.volume_24hr.as_ref());
    let liquidity = num(m.liquidity_num.as_ref());

    let degenerate =
        !positive(volume_24h) || !positive(liquidity) || !live_spread(spread) || !live_price(price);
    if degenerate {
        return Admission::TerminalDegenerate;
    }

    let snapshot_ts = m.created_at.as_deref().and_then(iso8601_to_unix);
    let resolved_ts = m
        .closed_time
        .as_deref()
        .or(m.end_date.as_deref())
        .and_then(iso8601_to_unix);
    let (Some(snap_ts), Some(res_ts)) = (snapshot_ts, resolved_ts) else {
        return Admission::LookAhead;
    };
    if snap_ts >= res_ts {
        return Admission::LookAhead;
    }

    let snapshot = MarketSnapshot {
        token_id: tokens[0].clone(),
        condition_id: cond.clone(),
        outcome_index: 0,
        slug: m.slug.clone().unwrap_or_else(|| cond.clone()),
        question: m.question.clone().or_else(|| m.slug.clone()),
        event_id: None,
        category: m.category.clone(),
        region: None,
        tags: Vec::new(),
        resolution_source: Some("gamma-closed-derived".to_string()),
        neg_risk: false,
        snapshot_ts: snap_ts,
        price,
        mid: price,
        best_bid: num(m.best_bid.as_ref()),
        best_ask: num(m.best_ask.as_ref()),
        spread,
        tick_size: None,
        volume_24h,
        liquidity,
        one_hour_change: None,
        one_day_change: None,
        ofi: None,
        yes_no_residual: None,
        secs_to_resolution: Some(res_ts.saturating_sub(snap_ts) as f64),
        holders: Vec::new(),
        makers: Vec::new(),
        counterparty_volumes: Vec::new(),
        onchain_fills: Vec::new(),
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    };
    let resolution = Resolution {
        condition_id: cond.clone(),
        winning_outcome_index: winner,
        winning_label: outcomes
            .get(winner as usize)
            .cloned()
            .unwrap_or_else(|| winner.to_string()),
        resolved_ts: res_ts,
        source: "gamma-closed-derived".to_string(),
        disputed: false,
    };
    Admission::Admit(Box::new((snapshot, resolution)))
}

fn positive(x: Option<f64>) -> bool {
    matches!(x, Some(v) if v > 0.0)
}

fn live_spread(x: Option<f64>) -> bool {
    matches!(x, Some(s) if (0.0..1.0).contains(&s))
}

fn live_price(x: Option<f64>) -> bool {
    matches!(x, Some(p) if (TERMINAL_EPS..=1.0 - TERMINAL_EPS).contains(&p))
}

fn parse_str_array(s: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
}

fn parse_num_array(s: &str) -> Vec<f64> {
    serde_json::from_str::<Vec<String>>(s)
        .map(|v| v.iter().filter_map(|x| x.parse::<f64>().ok()).collect())
        .unwrap_or_default()
}

fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64().filter(|x| x.is_finite()),
        Value::String(s) => s.parse::<f64>().ok().filter(|x| x.is_finite()),
        _ => None,
    }
}

fn iso8601_to_unix(s: &str) -> Option<u64> {
    let bytes = s.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || (bytes[10] != b'T' && bytes[10] != b' ')
    {
        return None;
    }
    let p = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (p(0, 4)?, p(5, 7)?, p(8, 10)?);
    let (h, mi, se) = (p(11, 13)?, p(14, 16)?, p(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3_600 + mi * 60 + se;
    u64::try_from(secs).ok()
}

#[cfg(test)]
#[path = "resolved_market_gamma_loader_tests.rs"]
mod tests;
