//! Read-only CLOB book-depth and liquidity feature extraction (issue #94).
//!
//! Poly consumes public market-data snapshots and writes normalized local evidence rows. The
//! derived fields are forecast evidence only: this module never computes order instructions,
//! executable size, stake size, bankroll, or PnL.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

pub const BOOK_LIQUIDITY_SCHEMA_VERSION: &str = "poly.book_liquidity.v1";
pub const PUBLIC_BOOK_SNAPSHOT_ARTIFACT_KIND: &str = "poly_public_book_snapshot";
pub const BOOK_LIQUIDITY_FEATURE_ARTIFACT_KIND: &str = "poly_book_liquidity_features";

pub const ERR_BOOK_LIQUIDITY_INVALID_REQUEST: &str = "CALYX_POLY_BOOK_LIQUIDITY_INVALID_REQUEST";
pub const ERR_BOOK_LIQUIDITY_INVALID_LEVEL: &str = "CALYX_POLY_BOOK_LIQUIDITY_INVALID_LEVEL";
pub const ERR_BOOK_LIQUIDITY_CROSSED: &str = "CALYX_POLY_BOOK_LIQUIDITY_CROSSED";
pub const ERR_BOOK_LIQUIDITY_READBACK_MISMATCH: &str =
    "CALYX_POLY_BOOK_LIQUIDITY_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublicBookLevel {
    pub price: f64,
    pub size: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublicBookSnapshot {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source_kind: String,
    pub source_url: String,
    pub condition_id: String,
    pub token_id: String,
    pub snapshot_ts: u64,
    pub captured_ts: u64,
    pub bids: Vec<PublicBookLevel>,
    pub asks: Vec<PublicBookLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_24h: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BookLiquidityFeatureRequest {
    pub snapshot: PublicBookSnapshot,
    pub now_ts: u64,
    pub max_age_seconds: u64,
    pub min_visible_liquidity: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookLiquidityStatus {
    Ready,
    EmptyBook,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BookLiquidityFeatureRow {
    pub schema_version: String,
    pub artifact_kind: String,
    pub token_id: String,
    pub condition_id: String,
    pub source_kind: String,
    pub source_url: String,
    pub snapshot_ts: u64,
    pub captured_ts: u64,
    pub now_ts: u64,
    pub stale_after_ts: u64,
    pub status: BookLiquidityStatus,
    pub degraded: bool,
    pub reason: String,
    pub bid_level_count: usize,
    pub ask_level_count: usize,
    pub bid_depth: f64,
    pub ask_depth: f64,
    pub visible_book_volume: f64,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub midpoint: Option<f64>,
    pub spread: Option<f64>,
    pub visible_liquidity: Option<f64>,
    pub depth_imbalance: Option<f64>,
    pub volume_24h: Option<f64>,
    pub min_visible_liquidity: f64,
    pub liquidity_ok: bool,
    pub raw_snapshot_hash: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BookLiquidityRun {
    pub raw_snapshot_path: PathBuf,
    pub feature_path: PathBuf,
    pub raw_snapshot: PublicBookSnapshot,
    pub feature_row: BookLiquidityFeatureRow,
}

pub fn run_book_liquidity_feature_extraction(
    request: &BookLiquidityFeatureRequest,
    output_root: &Path,
) -> Result<BookLiquidityRun> {
    validate_request_shape(request)?;
    let raw_snapshot_path = write_public_book_snapshot(output_root, &request.snapshot)?;
    let raw_readback = read_public_book_snapshot(&raw_snapshot_path)?;
    if raw_readback != request.snapshot {
        return Err(readback_mismatch(format!(
            "raw public book snapshot {} did not read back as written",
            raw_snapshot_path.display()
        )));
    }
    let feature_row = compute_book_liquidity_features(request)?;
    let feature_path = write_book_liquidity_features(output_root, &feature_row)?;
    let feature_readback = read_book_liquidity_features(&feature_path)?;
    if feature_readback != feature_row {
        return Err(readback_mismatch(format!(
            "book liquidity feature row {} did not read back as written",
            feature_path.display()
        )));
    }
    Ok(BookLiquidityRun {
        raw_snapshot_path,
        feature_path,
        raw_snapshot: raw_readback,
        feature_row: feature_readback,
    })
}

pub fn compute_book_liquidity_features(
    request: &BookLiquidityFeatureRequest,
) -> Result<BookLiquidityFeatureRow> {
    validate_request_shape(request)?;
    let snapshot = &request.snapshot;
    let bid_depth = normalize_feature(total_size(&snapshot.bids));
    let ask_depth = normalize_feature(total_size(&snapshot.asks));
    let visible_book_volume = normalize_feature(bid_depth + ask_depth);
    let stale_after_ts = snapshot.snapshot_ts.saturating_add(request.max_age_seconds);

    let (best_bid, best_ask, midpoint, spread, visible_liquidity) =
        if snapshot.bids.is_empty() || snapshot.asks.is_empty() {
            (None, None, None, None, None)
        } else {
            let bid = normalize_feature(best_bid(&snapshot.bids));
            let ask = normalize_feature(best_ask(&snapshot.asks));
            if bid >= ask {
                return Err(PolyError::diagnostics(
                    ERR_BOOK_LIQUIDITY_CROSSED,
                    format!("crossed or locked book: best_bid={bid:.6} best_ask={ask:.6}"),
                ));
            }
            (
                Some(bid),
                Some(ask),
                Some(normalize_feature((bid + ask) / 2.0)),
                Some(normalize_feature(ask - bid)),
                Some(normalize_feature(bid_depth.min(ask_depth))),
            )
        };
    let stale = request.now_ts > stale_after_ts;
    let status = if snapshot.bids.is_empty() || snapshot.asks.is_empty() {
        BookLiquidityStatus::EmptyBook
    } else if stale {
        BookLiquidityStatus::Stale
    } else {
        BookLiquidityStatus::Ready
    };
    let degraded = status != BookLiquidityStatus::Ready;
    let reason = match status {
        BookLiquidityStatus::Ready => "ready",
        BookLiquidityStatus::EmptyBook => "empty bid or ask side",
        BookLiquidityStatus::Stale => "snapshot is stale",
    }
    .to_string();
    let liquidity_ok = matches!(status, BookLiquidityStatus::Ready)
        && visible_liquidity.is_some_and(|x| x >= request.min_visible_liquidity);

    Ok(BookLiquidityFeatureRow {
        schema_version: BOOK_LIQUIDITY_SCHEMA_VERSION.to_string(),
        artifact_kind: BOOK_LIQUIDITY_FEATURE_ARTIFACT_KIND.to_string(),
        token_id: snapshot.token_id.clone(),
        condition_id: snapshot.condition_id.clone(),
        source_kind: snapshot.source_kind.clone(),
        source_url: snapshot.source_url.clone(),
        snapshot_ts: snapshot.snapshot_ts,
        captured_ts: snapshot.captured_ts,
        now_ts: request.now_ts,
        stale_after_ts,
        status,
        degraded,
        reason,
        bid_level_count: snapshot.bids.len(),
        ask_level_count: snapshot.asks.len(),
        bid_depth,
        ask_depth,
        visible_book_volume,
        best_bid,
        best_ask,
        midpoint,
        spread,
        visible_liquidity,
        depth_imbalance: imbalance(bid_depth, ask_depth),
        volume_24h: snapshot.volume_24h,
        min_visible_liquidity: request.min_visible_liquidity,
        liquidity_ok,
        raw_snapshot_hash: snapshot_hash(snapshot)?,
    })
}

pub fn write_public_book_snapshot(dir: &Path, snapshot: &PublicBookSnapshot) -> Result<PathBuf> {
    write_json(dir, &raw_file_name(snapshot), snapshot)
}

pub fn read_public_book_snapshot(path: &Path) -> Result<PublicBookSnapshot> {
    read_json(path)
}

pub fn write_book_liquidity_features(dir: &Path, row: &BookLiquidityFeatureRow) -> Result<PathBuf> {
    write_json(dir, &feature_file_name(row), row)
}

pub fn read_book_liquidity_features(path: &Path) -> Result<BookLiquidityFeatureRow> {
    read_json(path)
}

fn validate_request_shape(request: &BookLiquidityFeatureRequest) -> Result<()> {
    let snapshot = &request.snapshot;
    if snapshot.schema_version != BOOK_LIQUIDITY_SCHEMA_VERSION
        || snapshot.artifact_kind != PUBLIC_BOOK_SNAPSHOT_ARTIFACT_KIND
    {
        return invalid_request("unexpected public book snapshot schema or artifact kind");
    }
    if snapshot.token_id.trim().is_empty()
        || snapshot.condition_id.trim().is_empty()
        || snapshot.source_kind.trim().is_empty()
    {
        return invalid_request("token_id, condition_id, and source_kind are required");
    }
    if !snapshot
        .source_url
        .starts_with("https://clob.polymarket.com/book")
    {
        return invalid_request("book snapshots must come from the public CLOB /book endpoint");
    }
    if !request.min_visible_liquidity.is_finite() || request.min_visible_liquidity < 0.0 {
        return invalid_request("min_visible_liquidity must be finite and non-negative");
    }
    if let Some(volume) = snapshot.volume_24h
        && (!volume.is_finite() || volume < 0.0)
    {
        return invalid_request("volume_24h must be finite and non-negative when present");
    }
    for (idx, level) in snapshot.bids.iter().enumerate() {
        validate_level("bid", idx, level)?;
    }
    for (idx, level) in snapshot.asks.iter().enumerate() {
        validate_level("ask", idx, level)?;
    }
    Ok(())
}

fn validate_level(side: &str, idx: usize, level: &PublicBookLevel) -> Result<()> {
    if !level.price.is_finite()
        || !(0.0..=1.0).contains(&level.price)
        || !level.size.is_finite()
        || level.size <= 0.0
    {
        return Err(PolyError::diagnostics(
            ERR_BOOK_LIQUIDITY_INVALID_LEVEL,
            format!("{side} level {idx} must have finite price in [0,1] and positive finite size"),
        ));
    }
    Ok(())
}

fn total_size(levels: &[PublicBookLevel]) -> f64 {
    levels.iter().map(|level| level.size).sum()
}

fn best_bid(levels: &[PublicBookLevel]) -> f64 {
    levels
        .iter()
        .map(|level| level.price)
        .max_by(|a, b| a.total_cmp(b))
        .expect("non-empty bids")
}

fn best_ask(levels: &[PublicBookLevel]) -> f64 {
    levels
        .iter()
        .map(|level| level.price)
        .min_by(|a, b| a.total_cmp(b))
        .expect("non-empty asks")
}

fn imbalance(bid_depth: f64, ask_depth: f64) -> Option<f64> {
    let total = bid_depth + ask_depth;
    (total > 0.0).then_some(normalize_feature(
        ((bid_depth - ask_depth) / total).clamp(-1.0, 1.0),
    ))
}

fn snapshot_hash(snapshot: &PublicBookSnapshot) -> Result<String> {
    let bytes = serde_json::to_vec(snapshot).map_err(|err| {
        PolyError::diagnostics(
            ERR_BOOK_LIQUIDITY_INVALID_REQUEST,
            format!("serialize public book snapshot for hashing: {err}"),
        )
    })?;
    Ok(hex(&Sha256::digest(bytes)))
}

fn readback_mismatch(message: String) -> PolyError {
    PolyError::diagnostics(ERR_BOOK_LIQUIDITY_READBACK_MISMATCH, message)
}

fn invalid_request<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_BOOK_LIQUIDITY_INVALID_REQUEST,
        message.into(),
    ))
}

fn raw_file_name(snapshot: &PublicBookSnapshot) -> String {
    format!(
        "raw_book_snapshot_{}_{}.json",
        sanitize(&snapshot.token_id),
        snapshot.snapshot_ts
    )
}

fn feature_file_name(row: &BookLiquidityFeatureRow) -> String {
    format!(
        "book_liquidity_{}_{}.json",
        sanitize(&row.token_id),
        row.snapshot_ts
    )
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn normalize_feature(value: f64) -> f64 {
    const SCALE: f64 = 1_000_000_000_000.0;
    let normalized = (value * SCALE).round() / SCALE;
    if normalized == 0.0 { 0.0 } else { normalized }
}
