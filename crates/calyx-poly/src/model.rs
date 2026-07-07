//! Polymarket record types produced by the ingestor from the four data doors + on-chain.
//!
//! These are plain serde structs — the "eyes" of the system. [`crate::constellation`] maps a
//! [`MarketSnapshot`] to a real [`calyx_core::Constellation`]; a [`Resolution`] becomes an anchor.

use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

/// One price level in an order book.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Level {
    /// Price (0..1).
    pub price: f64,
    /// Resting size at this level.
    pub size: f64,
}

/// A snapshot of an order book (top levels).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Book {
    /// Bid levels, best first.
    pub bids: Vec<Level>,
    /// Ask levels, best first.
    pub asks: Vec<Level>,
}

/// A holder's position in one outcome token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolderShare {
    /// Proxy wallet address.
    pub wallet: String,
    /// Share amount held.
    pub amount: f64,
    /// Which outcome token (0 = YES, 1 = NO, or negRisk index).
    pub outcome_index: u32,
}

/// Provenance for rows shaped like maker concentration evidence.
///
/// Public Data API holder/position/trader wallets are useful wallet-concentration evidence, but
/// they are not resting CLOB maker-address size. Only `RestingClobOrderBook` rows may feed maker
/// concentration screens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MakerShareEvidenceSource {
    /// True resting book size grouped by maker address.
    RestingClobOrderBook,
    /// Holder or position wallet rows, explicitly not maker evidence.
    HolderOrPositionWallet,
    /// Trade/activity/counterparty wallet rows, explicitly not maker evidence.
    TraderOrCounterpartyWallet,
    /// Legacy or unknown provenance; fail closed when used as maker evidence.
    #[default]
    Unknown,
}

/// Resting book size grouped by maker address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerShare {
    /// CLOB maker or proxy wallet address.
    pub maker: String,
    /// Resting size attributed to this maker across visible book levels.
    pub size: f64,
    /// Source proof for this row. Must be `RestingClobOrderBook` before market-integrity screens
    /// treat the row as true maker concentration.
    #[serde(default)]
    pub evidence_source: MakerShareEvidenceSource,
}

/// Recent on-chain matched volume attributed to one distinct counterparty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterpartyVolume {
    /// Counterparty wallet or proxy address.
    pub counterparty: String,
    /// Matched volume attributed to this counterparty in the observation window.
    pub volume: f64,
}

/// Taker-side direction for one public on-chain fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnchainFillSide {
    /// Taker bought the outcome token.
    Buy,
    /// Taker sold the outcome token.
    Sell,
}

/// One public/read-only on-chain matched fill used for flow-toxicity measurements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnchainFill {
    /// Transaction hash containing the fill event.
    pub tx_hash: String,
    /// Log index within the transaction or block.
    pub log_index: u32,
    /// Fill event timestamp (unix seconds).
    pub timestamp: u64,
    /// Maker/proxy wallet address observed in the fill.
    pub maker: String,
    /// Taker/proxy wallet address observed in the fill.
    pub taker: String,
    /// Taker-side fill direction.
    pub side: OnchainFillSide,
    /// Fill price in implied-probability units.
    pub price: f64,
    /// Matched outcome-token size.
    pub size: f64,
}

/// UMA/oracle resolution evidence captured from read-only source data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct OracleRiskEvidence {
    /// Oracle source identifier, for example `uma`.
    pub oracle: String,
    /// Known dispute probability or score in `[0, 1]`.
    pub dispute_risk: f64,
    /// Whether the market currently has an active oracle dispute.
    pub active_dispute: bool,
    /// Seconds remaining in the optimistic-oracle challenge window.
    pub liveness_seconds_remaining: f64,
}

/// One observation of one market outcome token at one time — the primary record we predict on.
///
/// Numeric fields are `Option` so a feed gap yields an `Absent` slot rather than a fabricated zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSnapshot {
    /// CLOB outcome token id (the per-outcome ERC-1155 id).
    pub token_id: String,
    /// CTF condition id (identifies the market).
    pub condition_id: String,
    /// Outcome index this token represents (0 = YES).
    pub outcome_index: u32,
    /// Market slug.
    pub slug: String,
    /// Human-readable market question text.
    pub question: Option<String>,
    /// Event id grouping related markets (negRisk siblings).
    pub event_id: Option<String>,
    /// Category (politics/sports/crypto/…).
    pub category: Option<String>,
    /// Region/geography derived from the event/tags (state, country, city).
    pub region: Option<String>,
    /// Topical tags.
    pub tags: Vec<String>,
    /// Resolution source string.
    pub resolution_source: Option<String>,
    /// Whether this is a negRisk multi-outcome market.
    pub neg_risk: bool,
    /// Snapshot time (unix seconds).
    pub snapshot_ts: u64,

    /// Last/mid implied probability for this token.
    pub price: Option<f64>,
    /// Midpoint price.
    pub mid: Option<f64>,
    /// Best bid.
    pub best_bid: Option<f64>,
    /// Best ask.
    pub best_ask: Option<f64>,
    /// Bid/ask spread.
    pub spread: Option<f64>,
    /// Minimum tick size.
    pub tick_size: Option<f64>,
    /// 24h volume (raw; wash-contaminated — prefer distinct-counterparty).
    pub volume_24h: Option<f64>,
    /// Liquidity.
    pub liquidity: Option<f64>,
    /// One-hour price change.
    pub one_hour_change: Option<f64>,
    /// One-day price change.
    pub one_day_change: Option<f64>,
    /// Order-flow imbalance over the recent window (from on-chain fills).
    pub ofi: Option<f64>,
    /// Binary internal-arb residual `yes + no − 1` (if the sister token price is known).
    pub yes_no_residual: Option<f64>,
    /// Seconds until scheduled resolution.
    pub secs_to_resolution: Option<f64>,
    /// Top holders of this outcome token.
    #[serde(default)]
    pub holders: Vec<HolderShare>,
    /// Resting maker-address size evidence for thin/manipulable-market screens.
    #[serde(default)]
    pub makers: Vec<MakerShare>,
    /// Distinct on-chain counterparty volume evidence for wash-trade screens.
    #[serde(default)]
    pub counterparty_volumes: Vec<CounterpartyVolume>,
    /// Public on-chain fill sequence for VPIN-style toxicity measurements.
    #[serde(default)]
    pub onchain_fills: Vec<OnchainFill>,
    /// Reference/query time for temporal retrieval sidecars.
    pub temporal_reference_ts: Option<u64>,
    /// Position of this observation within the market's ordered history.
    pub sequence_position: Option<u64>,
    /// Total ordered observations in the market history used for E4 position.
    pub sequence_total: Option<u64>,
    /// Read-only oracle/dispute evidence for oracle-risk screens.
    #[serde(default)]
    pub oracle_risk: OracleRiskEvidence,
    /// Order book snapshot.
    #[serde(default)]
    pub book: Book,
}

impl MarketSnapshot {
    /// Returns `true` iff every present numeric field is finite. JSON has no representation for
    /// `NaN`/`Infinity`, so a non-finite value cannot be content-addressed deterministically and
    /// identity must fail closed rather than serialize it to an ambiguous `null`.
    fn all_numeric_finite(&self) -> bool {
        let opt_ok = |v: Option<f64>| v.is_none_or(|x| x.is_finite());
        opt_ok(self.price)
            && opt_ok(self.mid)
            && opt_ok(self.best_bid)
            && opt_ok(self.best_ask)
            && opt_ok(self.spread)
            && opt_ok(self.tick_size)
            && opt_ok(self.volume_24h)
            && opt_ok(self.liquidity)
            && opt_ok(self.one_hour_change)
            && opt_ok(self.one_day_change)
            && opt_ok(self.ofi)
            && opt_ok(self.yes_no_residual)
            && opt_ok(self.secs_to_resolution)
            && self
                .book
                .bids
                .iter()
                .chain(self.book.asks.iter())
                .all(|l| l.price.is_finite() && l.size.is_finite())
            && self.holders.iter().all(|h| h.amount.is_finite())
            && self.makers.iter().all(|m| m.size.is_finite())
            && self
                .counterparty_volumes
                .iter()
                .all(|c| c.volume.is_finite())
            && self
                .onchain_fills
                .iter()
                .all(|f| f.price.is_finite() && f.size.is_finite())
            && self.oracle_risk.dispute_risk.is_finite()
            && self.oracle_risk.liveness_seconds_remaining.is_finite()
    }

    /// Canonical identity bytes for content-addressing.
    ///
    /// Serializes the **entire** observed snapshot — every typed field, including book depth,
    /// liquidity, OFI, holders, makers, counterparty volumes, and oracle-risk evidence — so two
    /// byte-identical observations dedup to one `CxId` (idempotent re-ingest) while *any*
    /// difference in observed content yields a **distinct** `CxId`. The previous behavior hashed a
    /// hand-picked 6-field subset (`token_id, ts, price, mid, spread, volume_24h`), which silently
    /// collapsed distinct observations (e.g. a whale posting deep liquidity within the same second)
    /// onto one content address, and `VaultStore::put` then dropped the second while returning
    /// `Ok` — a silent data-loss bug (issue #181). There is no time bucketing: `snapshot_ts` is
    /// used verbatim in seconds.
    ///
    /// Fails closed with a stable structured error if the snapshot carries a non-finite numeric
    /// field or serialization fails — never an empty-bytes fallback that would collapse unrelated
    /// snapshots onto one content address (issue #171).
    pub fn canonical_input_bytes(&self) -> Result<Vec<u8>> {
        if !self.all_numeric_finite() {
            return Err(PolyError::snapshot_identity(
                "CALYX_POLY_SNAPSHOT_IDENTITY_NON_FINITE",
                format!(
                    "snapshot token_id={} ts={} carries a non-finite numeric field; reject or \
                     normalize it upstream before content-addressing",
                    self.token_id, self.snapshot_ts
                ),
            ));
        }
        serde_json::to_vec(self).map_err(|err| {
            PolyError::snapshot_identity(
                "CALYX_POLY_SNAPSHOT_IDENTITY_SERIALIZE_FAILED",
                format!(
                    "canonical snapshot identity serialization failed for token_id={} ts={}: {err}",
                    self.token_id, self.snapshot_ts
                ),
            )
        })
    }
}

/// A resolved market outcome (from UMA). Grounds every snapshot of the market.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// Condition id of the resolved market.
    pub condition_id: String,
    /// Winning outcome index.
    pub winning_outcome_index: u32,
    /// Human-readable winning label (e.g. "YES").
    pub winning_label: String,
    /// Resolution time (unix seconds).
    pub resolved_ts: u64,
    /// Resolution source/oracle.
    pub source: String,
    /// Whether the resolution was disputed (oracle-risk signal).
    pub disputed: bool,
}
