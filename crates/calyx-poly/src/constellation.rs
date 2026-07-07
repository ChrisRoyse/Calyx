//! Maps Polymarket records to real Calyx records: [`MarketSnapshot`] → [`Constellation`],
//! [`Resolution`] → [`Anchor`].

use std::collections::BTreeMap;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    VaultId,
};

use crate::error::Result;
use crate::features;
use crate::grounding::{GAMMA_CLOSED_DERIVED_SOURCE_PREFIX, RESOLVED_SOURCE_PREFIX};
use crate::lenses::PolyPanel;
use crate::model::{MakerShareEvidenceSource, MarketSnapshot, Resolution};

/// Builds the verbatim numeric `scalars` map (kept alongside the lensed vectors, no loss).
pub fn scalars_of(s: &MarketSnapshot) -> BTreeMap<String, f64> {
    let mut m = BTreeMap::new();
    let mut put = |k: &str, v: Option<f64>| {
        if let Some(x) = v
            && x.is_finite()
        {
            m.insert(k.to_string(), x);
        }
    };
    put("price", s.price);
    put("mid", s.mid);
    put("best_bid", s.best_bid);
    put("best_ask", s.best_ask);
    put("spread", s.spread);
    put("tick_size", s.tick_size);
    put("volume_24h", s.volume_24h);
    put("liquidity", s.liquidity);
    put("one_hour_change", s.one_hour_change);
    put("one_day_change", s.one_day_change);
    put("ofi", s.ofi);
    put("yes_no_residual", s.yes_no_residual);
    put("secs_to_resolution", s.secs_to_resolution);
    // Derived scalars.
    if let Some(distance) = s.price.or(s.mid).and_then(features::distance_from_50) {
        m.insert("distance_from_50".to_string(), distance);
    }
    if !s.holders.is_empty() {
        let amounts: Vec<f64> = s.holders.iter().map(|h| h.amount).collect();
        m.insert("holder_count".to_string(), amounts.len() as f64);
        m.insert(
            "holder_herfindahl".to_string(),
            features::herfindahl(&amounts),
        );
        m.insert(
            "top_holder_share".to_string(),
            features::top_share(&amounts),
        );
    }
    let maker_sizes: Vec<f64> = s
        .makers
        .iter()
        .filter(|m| m.evidence_source == MakerShareEvidenceSource::RestingClobOrderBook)
        .map(|m| m.size)
        .collect();
    if !maker_sizes.is_empty() {
        let sizes = maker_sizes;
        m.insert("maker_count".to_string(), sizes.len() as f64);
        m.insert("maker_herfindahl".to_string(), features::herfindahl(&sizes));
        m.insert("top_maker_share".to_string(), features::top_share(&sizes));
    }
    if !s.counterparty_volumes.is_empty() {
        let volumes: Vec<f64> = s.counterparty_volumes.iter().map(|c| c.volume).collect();
        let distinct_volume: f64 = volumes.iter().filter(|x| x.is_finite() && **x > 0.0).sum();
        m.insert(
            "distinct_counterparty_count".to_string(),
            volumes.len() as f64,
        );
        m.insert("distinct_counterparty_volume".to_string(), distinct_volume);
        m.insert(
            "top_counterparty_share".to_string(),
            features::top_share(&volumes),
        );
        if let Some(raw_volume) = s.volume_24h
            && raw_volume.is_finite()
            && raw_volume > 0.0
        {
            m.insert(
                "distinct_counterparty_volume_ratio".to_string(),
                distinct_volume / raw_volume,
            );
        }
    }
    if !s.oracle_risk.oracle.trim().is_empty() {
        m.insert(
            "oracle_dispute_risk".to_string(),
            s.oracle_risk.dispute_risk,
        );
        m.insert(
            "oracle_active_dispute".to_string(),
            if s.oracle_risk.active_dispute {
                1.0
            } else {
                0.0
            },
        );
        m.insert(
            "oracle_liveness_seconds_remaining".to_string(),
            s.oracle_risk.liveness_seconds_remaining,
        );
    }
    m
}

/// Builds the verbatim string `metadata` map (categoricals, ids, region — "place/region/everything").
pub fn metadata_of(s: &MarketSnapshot) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("record_type".to_string(), "market_snapshot".to_string());
    m.insert("condition_id".to_string(), s.condition_id.clone());
    m.insert("token_id".to_string(), s.token_id.clone());
    m.insert("outcome_index".to_string(), s.outcome_index.to_string());
    m.insert("slug".to_string(), s.slug.clone());
    m.insert("neg_risk".to_string(), s.neg_risk.to_string());
    if let Some(question) = &s.question {
        m.insert("question".to_string(), question.clone());
    }
    if let Some(e) = &s.event_id {
        m.insert("event_id".to_string(), e.clone());
    }
    if let Some(c) = &s.category {
        m.insert("category".to_string(), c.clone());
        // Oracle-domain metadata key so calyx-oracle can match by domain.
        m.insert("oracle.domain".to_string(), c.clone());
    }
    if let Some(r) = &s.region {
        m.insert("region".to_string(), r.clone());
    }
    if let Some(rs) = &s.resolution_source {
        m.insert("resolution_source".to_string(), rs.clone());
    }
    if !s.tags.is_empty() {
        m.insert("tags".to_string(), s.tags.join(","));
    }
    m
}

/// Builds a Calyx constellation for a snapshot. Slots come from the panel; scalars/metadata are
/// verbatim; anchors are empty (grounded later on resolution). The `CxId` is content-addressed over
/// the *entire* observed snapshot, so identical observations dedup while any content difference
/// yields a distinct id. Fails closed if canonical identity bytes cannot be produced (issue #181,
/// #171).
pub fn build_constellation(
    s: &MarketSnapshot,
    panel: &PolyPanel,
    vault_id: VaultId,
    vault_salt: &[u8],
) -> Result<Constellation> {
    let input_bytes = s.canonical_input_bytes()?;
    let cx_id = CxId::from_input(&input_bytes, panel.version, vault_salt);
    let hash = *blake3::hash(&input_bytes).as_bytes();

    Ok(Constellation {
        cx_id,
        vault_id,
        panel_version: panel.version,
        created_at: s.snapshot_ts.saturating_mul(1000), // unix ms
        input_ref: InputRef {
            hash,
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots: panel.measure_all(s),
        scalars: scalars_of(s),
        metadata: metadata_of(s),
        anchors: Vec::new(),
        // Provenance is (re)assigned by the store on `put`; a zeroed placeholder is fine here.
        provenance: LedgerRef {
            seq: 0,
            hash: [0u8; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            degraded: false,
            novel_region: false,
            redacted_input: false,
        },
    })
}

/// Builds the grounding anchor for a resolved market, for the outcome token at `our_outcome_index`.
/// `TestPass{Bool}` = "did the bought side win", plus the winning label as the source detail.
pub fn resolution_anchor(r: &Resolution, our_outcome_index: u32) -> Anchor {
    let won = r.winning_outcome_index == our_outcome_index;
    Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(won),
        source: resolution_anchor_source(r, Some(&r.winning_label)),
        observed_at: r.resolved_ts.saturating_mul(1000),
        confidence: 1.0,
    }
}

/// Builds a labeled outcome anchor (the winning label) for richer grounding / negRisk.
pub fn resolution_label_anchor(r: &Resolution) -> Anchor {
    Anchor {
        kind: AnchorKind::Label("outcome".to_string()),
        value: AnchorValue::Enum(r.winning_label.clone()),
        source: resolution_anchor_source(r, None),
        observed_at: r.resolved_ts.saturating_mul(1000),
        confidence: 1.0,
    }
}

fn resolution_anchor_source(r: &Resolution, label: Option<&str>) -> String {
    let source = r.source.trim();
    if source == "gamma-closed-derived" {
        return match label {
            Some(label) => format!("{GAMMA_CLOSED_DERIVED_SOURCE_PREFIX}{label}"),
            None => format!("{GAMMA_CLOSED_DERIVED_SOURCE_PREFIX}outcome"),
        };
    }
    match label {
        Some(label) => format!("{RESOLVED_SOURCE_PREFIX}{source}:{label}"),
        None => format!("{RESOLVED_SOURCE_PREFIX}{source}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lenses::default_panel;
    use crate::model::{CounterpartyVolume, HolderShare, MakerShare, MakerShareEvidenceSource};

    fn sample() -> MarketSnapshot {
        MarketSnapshot {
            token_id: "tok".into(),
            condition_id: "0xcond".into(),
            outcome_index: 0,
            slug: "will-btc-100k".into(),
            question: Some("Will BTC reach 100k?".into()),
            event_id: Some("evt1".into()),
            category: Some("crypto".into()),
            region: Some("global".into()),
            tags: vec!["btc".into(), "price".into()],
            resolution_source: Some("uma".into()),
            neg_risk: false,
            snapshot_ts: 1_785_500_000,
            price: Some(0.62),
            mid: Some(0.62),
            best_bid: Some(0.61),
            best_ask: Some(0.63),
            spread: Some(0.02),
            tick_size: Some(0.01),
            volume_24h: Some(125_000.0),
            liquidity: Some(40_000.0),
            one_hour_change: Some(0.01),
            one_day_change: Some(-0.03),
            ofi: Some(0.2),
            yes_no_residual: Some(0.0),
            secs_to_resolution: Some(86_400.0),
            holders: vec![HolderShare {
                wallet: "0xwhale".into(),
                amount: 1000.0,
                outcome_index: 0,
            }],
            makers: vec![MakerShare {
                maker: "0xmaker".into(),
                size: 1000.0,
                evidence_source: MakerShareEvidenceSource::RestingClobOrderBook,
            }],
            counterparty_volumes: vec![CounterpartyVolume {
                counterparty: "0xcp".into(),
                volume: 1000.0,
            }],
            onchain_fills: vec![],
            temporal_reference_ts: None,
            sequence_position: None,
            sequence_total: None,
            oracle_risk: Default::default(),
            book: Default::default(),
        }
    }

    #[test]
    fn cx_id_is_idempotent() {
        let panel = default_panel(1, vec!["global".into()]);
        let vid: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let a = build_constellation(&sample(), &panel, vid, b"salt").unwrap();
        let b = build_constellation(&sample(), &panel, vid, b"salt").unwrap();
        assert_eq!(a.cx_id, b.cx_id);
    }

    #[test]
    fn distinct_microstructure_yields_distinct_cx_id() {
        // Regression for #181: two observations that agree on the old 6-identity-field subset
        // (token_id, ts, price, mid, spread, volume_24h) but differ in microstructure must NOT
        // collapse to one CxId. A whale posts deep liquidity / a bigger bid within the same second.
        let panel = default_panel(1, vec!["global".into()]);
        let vid: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let a = sample();
        let mut b = sample();
        // Identical old-identity fields; only microstructure changes.
        assert_eq!(a.token_id, b.token_id);
        assert_eq!(a.snapshot_ts, b.snapshot_ts);
        assert_eq!(a.price, b.price);
        assert_eq!(a.mid, b.mid);
        assert_eq!(a.spread, b.spread);
        assert_eq!(a.volume_24h, b.volume_24h);
        b.best_bid = Some(0.615);
        b.liquidity = Some(250_000.0);
        b.ofi = Some(0.4);
        let ca = build_constellation(&a, &panel, vid, b"salt").unwrap();
        let cb = build_constellation(&b, &panel, vid, b"salt").unwrap();
        assert_ne!(
            ca.cx_id, cb.cx_id,
            "distinct microstructure must produce distinct content addresses"
        );
    }

    #[test]
    fn non_finite_snapshot_fails_closed() {
        // #171: a non-finite numeric field must fail closed, never produce empty identity bytes.
        let panel = default_panel(1, vec!["global".into()]);
        let vid: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let mut s = sample();
        s.liquidity = Some(f64::INFINITY);
        let err = build_constellation(&s, &panel, vid, b"salt").expect_err("must fail closed");
        assert_eq!(err.code(), "CALYX_POLY_SNAPSHOT_IDENTITY_NON_FINITE");
    }

    #[test]
    fn constellation_has_scalars_and_metadata() {
        let panel = default_panel(1, vec!["global".into()]);
        let vid: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let cx = build_constellation(&sample(), &panel, vid, b"salt").unwrap();
        assert!(cx.scalars.contains_key("price"));
        assert!(cx.scalars.contains_key("distance_from_50"));
        assert_eq!(
            cx.metadata.get("category").map(String::as_str),
            Some("crypto")
        );
        assert!(cx.flags.ungrounded);
    }

    #[test]
    fn resolution_anchor_marks_winner() {
        let r = Resolution {
            condition_id: "0xcond".into(),
            winning_outcome_index: 0,
            winning_label: "YES".into(),
            resolved_ts: 1_785_600_000,
            source: "uma".into(),
            disputed: false,
        };
        let won = resolution_anchor(&r, 0);
        assert_eq!(won.value, AnchorValue::Bool(true));
        let lost = resolution_anchor(&r, 1);
        assert_eq!(lost.value, AnchorValue::Bool(false));
    }
}
