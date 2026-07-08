//! Anchor grounding kinds and the Provisional→Trusted trust lifecycle (issue #209).
//!
//! Every bit / association / diagnostic Poly measures is grounded on an [`Anchor`]. The handbook
//! (§3 "Anchor", §5.3, §7.7 honesty gate) draws a hard line between two anchor origins:
//!
//! - A **resolved UMA outcome** is certain — its anchor carries `confidence == 1.0` and any record
//!   grounded on it is [`TrustTag::Trusted`].
//! - A **proxy anchor** on a still-open market (up_1h / up_24h / crossed_0.5, issue #76) is an
//!   *estimate*, not certainty. Its confidence is finite in the open interval `(0, 1)` and any
//!   record grounded on it is [`TrustTag::Provisional`] until the real outcome resolves (issue #77).
//!
//! `calyx_assay::trust_for_anchor` classifies purely on confidence being finite in `(0, 1]`, so it
//! cannot tell a `confidence == 0.7` proxy from a resolved outcome — it would call both grounded.
//! Poly therefore derives trust from the anchor *kind*, not the confidence value alone, and refuses
//! to promote a record to Trusted except through a real resolved-outcome backfill. See issue #209
//! and the sibling gap tracked for the assay-side heuristic.

use calyx_assay::TrustTag;
use calyx_core::{Anchor, AnchorKind, AnchorValue};
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

/// Source-field prefix stamped on a resolved-UMA outcome anchor.
pub const RESOLVED_SOURCE_PREFIX: &str = "uma:";
/// Source-field prefix stamped on a Gamma closed-price-derived outcome anchor.
pub const GAMMA_CLOSED_DERIVED_SOURCE_PREFIX: &str = "gamma-closed-derived:";
/// Source-field prefix stamped on a proxy anchor built from a live (unresolved) market.
pub const PROXY_SOURCE_PREFIX: &str = "proxy:";

/// A proxy anchor built construction with confidence outside the open interval `(0, 1)`.
pub const ERR_PROXY_CONFIDENCE: &str = "CALYX_POLY_PROXY_ANCHOR_CONFIDENCE_OUT_OF_RANGE";
/// A resolved anchor presented with confidence other than exactly `1.0`.
pub const ERR_RESOLVED_CONFIDENCE: &str = "CALYX_POLY_RESOLVED_ANCHOR_CONFIDENCE_NOT_CERTAIN";
/// An anchor whose source prefix matches no known grounding origin.
pub const ERR_UNKNOWN_GROUNDING: &str = "CALYX_POLY_ANCHOR_UNKNOWN_GROUNDING_ORIGIN";
/// No load-bearing anchors were supplied for a trust rollup.
pub const ERR_NO_GROUNDING: &str = "CALYX_POLY_TRUST_ROLLUP_NO_ANCHORS";
/// A Gamma-derived outcome was legacy-stamped as UMA finality.
pub const ERR_GAMMA_DERIVED_AS_UMA: &str = "CALYX_POLY_GAMMA_DERIVED_AS_UMA_FINAL";
/// Supersession requires a Gamma-derived source anchor and a UMA-final target anchor.
pub const ERR_GAMMA_SUPERSESSION_KIND: &str = "CALYX_POLY_GAMMA_SUPERSESSION_KIND_MISMATCH";

/// The origin of the anchor that grounded a measured record — the axis the [`TrustTag`] derives from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundingKind {
    /// A resolved UMA outcome: certain, `confidence == 1.0`, grounds Trusted records.
    ResolvedUma,
    /// A closed-market Gamma price inference: useful outcome evidence, but not UMA finality.
    GammaClosedDerived,
    /// A proxy anchor on a live market: an estimate, `confidence` in `(0, 1)`, grounds Provisional
    /// records until backfilled with the real outcome (issue #77).
    Proxy(ProxyKind),
}

/// The three live-market proxy outcome axes (issue #76).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    /// Price rose over the next hour.
    Up1h,
    /// Price rose over the next 24 hours.
    Up24h,
    /// Price crossed the 0.5 midline.
    Crossed05,
}

impl ProxyKind {
    /// Stable source-tag suffix for the proxy axis.
    pub fn tag(self) -> &'static str {
        match self {
            ProxyKind::Up1h => "up_1h",
            ProxyKind::Up24h => "up_24h",
            ProxyKind::Crossed05 => "crossed_0.5",
        }
    }
}

impl GroundingKind {
    /// The trust every record grounded on this anchor origin carries.
    pub fn trust(self) -> TrustTag {
        match self {
            GroundingKind::ResolvedUma => TrustTag::Trusted,
            GroundingKind::GammaClosedDerived => TrustTag::Provisional,
            GroundingKind::Proxy(_) => TrustTag::Provisional,
        }
    }
}

/// Builds a **proxy** outcome anchor for a still-open market. Fails closed unless `confidence` is
/// finite in the open interval `(0, 1)` — a proxy is an estimate and must never claim certainty
/// (issue #209 §4). `observed_at_secs` is unix seconds; stored as unix ms to match Calyx anchors.
pub fn proxy_anchor(
    kind: ProxyKind,
    value: bool,
    confidence: f64,
    observed_at_secs: u64,
) -> Result<Anchor> {
    // Validate the *stored* f32 value so construction and classification agree under the cast.
    let stored = confidence as f32;
    if !(confidence.is_finite() && stored > 0.0 && stored < 1.0) {
        return Err(PolyError::grounding(
            ERR_PROXY_CONFIDENCE,
            format!(
                "proxy anchor confidence {confidence} must be finite in the open interval (0, 1) \
                 (also when rounded to f32); a proxy on a live market is an estimate, not a \
                 resolved certainty"
            ),
        ));
    }
    Ok(Anchor {
        kind: AnchorKind::Label("proxy_outcome".to_string()),
        value: AnchorValue::Bool(value),
        source: format!("{PROXY_SOURCE_PREFIX}{}", kind.tag()),
        observed_at: observed_at_secs.saturating_mul(1000),
        confidence: stored,
    })
}

/// Classifies an anchor's grounding origin from its source prefix, and validates that its confidence
/// matches that origin's contract (resolved → exactly 1.0; proxy → open `(0, 1)`). Fails closed on
/// an unknown origin or a confidence that contradicts the origin.
pub fn grounding_kind_of(anchor: &Anchor) -> Result<GroundingKind> {
    let source = anchor.source.trim();
    if let Some(tag) = source.strip_prefix(PROXY_SOURCE_PREFIX) {
        if !(anchor.confidence.is_finite() && anchor.confidence > 0.0 && anchor.confidence < 1.0) {
            return Err(PolyError::grounding(
                ERR_PROXY_CONFIDENCE,
                format!(
                    "proxy anchor '{source}' has confidence {} outside (0, 1)",
                    anchor.confidence
                ),
            ));
        }
        let kind = match tag {
            "up_1h" => ProxyKind::Up1h,
            "up_24h" => ProxyKind::Up24h,
            "crossed_0.5" => ProxyKind::Crossed05,
            other => {
                return Err(PolyError::grounding(
                    ERR_UNKNOWN_GROUNDING,
                    format!("proxy anchor axis '{other}' is not a known proxy kind"),
                ));
            }
        };
        return Ok(GroundingKind::Proxy(kind));
    }
    if source.starts_with(GAMMA_CLOSED_DERIVED_SOURCE_PREFIX) {
        if anchor.confidence != 1.0 {
            return Err(PolyError::grounding(
                ERR_RESOLVED_CONFIDENCE,
                format!(
                    "Gamma-derived outcome anchor '{source}' must carry confidence exactly 1.0, got {}",
                    anchor.confidence
                ),
            ));
        }
        return Ok(GroundingKind::GammaClosedDerived);
    }
    if source.starts_with(RESOLVED_SOURCE_PREFIX) {
        let rest = source
            .strip_prefix(RESOLVED_SOURCE_PREFIX)
            .unwrap_or_default();
        if rest.starts_with("gamma-") {
            return Err(PolyError::grounding(
                ERR_GAMMA_DERIVED_AS_UMA,
                format!(
                    "Gamma-derived outcome anchor '{source}' is legacy-stamped as UMA finality"
                ),
            ));
        }
        if anchor.confidence != 1.0 {
            return Err(PolyError::grounding(
                ERR_RESOLVED_CONFIDENCE,
                format!(
                    "resolved anchor '{source}' must carry confidence exactly 1.0, got {}",
                    anchor.confidence
                ),
            ));
        }
        return Ok(GroundingKind::ResolvedUma);
    }
    Err(PolyError::grounding(
        ERR_UNKNOWN_GROUNDING,
        format!(
            "anchor source '{source}' matches no known grounding origin ('{RESOLVED_SOURCE_PREFIX}', \
             '{GAMMA_CLOSED_DERIVED_SOURCE_PREFIX}', or '{PROXY_SOURCE_PREFIX}')"
        ),
    ))
}

/// Rolls up the trust of a set of load-bearing anchors into a single [`TrustTag`] for a record
/// measured against all of them. Trusted **iff every** anchor is a resolved outcome; a single proxy
/// makes the whole record Provisional (honesty gate — no silent promotion). Fails closed on an empty
/// set or any unclassifiable anchor.
pub fn rollup_trust(anchors: &[Anchor]) -> Result<TrustTag> {
    if anchors.is_empty() {
        return Err(PolyError::grounding(
            ERR_NO_GROUNDING,
            "trust rollup requires at least one grounding anchor",
        ));
    }
    let mut all_trusted = true;
    for anchor in anchors {
        if grounding_kind_of(anchor)?.trust() != TrustTag::Trusted {
            all_trusted = false;
        }
    }
    Ok(if all_trusted {
        TrustTag::Trusted
    } else {
        TrustTag::Provisional
    })
}

/// A backfill was asked to promote against an anchor that is not a resolved UMA outcome.
pub const ERR_BACKFILL_NOT_RESOLVED: &str = "CALYX_POLY_BACKFILL_ANCHOR_NOT_RESOLVED";
/// The resolved outcome contradicts the proxy prediction it is backfilling.
pub const ERR_BACKFILL_CONTRADICTION: &str = "CALYX_POLY_BACKFILL_PROXY_CONTRADICTION";

/// The audit record of a single Provisional→Trusted trust transition (issue #209 §2). Emitted when a
/// resolved outcome backfills a proxy-grounded record; carries both anchors' sources so the
/// transition is reproducible from disk / a ledger entry — never a silent promotion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustTransition {
    /// Trust before the backfill (always Provisional for a real transition).
    pub from: TrustTag,
    /// Trust after the backfill.
    pub to: TrustTag,
    /// The proxy anchor source that grounded the Provisional record.
    pub proxy_source: String,
    /// The proxy axis that was predicted.
    pub proxy_kind: ProxyKind,
    /// The resolved-outcome anchor source that grounds the promotion.
    pub resolved_source: String,
    /// The proxy's predicted boolean outcome.
    pub proxy_outcome: bool,
    /// The resolved boolean outcome.
    pub resolved_outcome: bool,
}

/// How a UMA-final outcome superseded a prior Gamma closed-price-derived outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionSupersessionKind {
    /// Gamma and UMA agreed; trust can upgrade from derived/provisional to UMA-final/trusted.
    UpgradeOnAgreement,
    /// Gamma and UMA disagreed; the UMA outcome corrects the derived result and callers must re-score.
    CorrectionOnDisagreement,
}

/// Audit record for a Gamma-derived outcome being superseded by a UMA-final outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionSupersession {
    pub kind: ResolutionSupersessionKind,
    pub from: TrustTag,
    pub to: TrustTag,
    pub gamma_source: String,
    pub uma_source: String,
    pub gamma_outcome: bool,
    pub uma_outcome: bool,
}

pub fn supersede_gamma_closed_resolution(
    gamma: &Anchor,
    uma: &Anchor,
) -> Result<ResolutionSupersession> {
    if grounding_kind_of(gamma)? != GroundingKind::GammaClosedDerived {
        return Err(PolyError::grounding(
            ERR_GAMMA_SUPERSESSION_KIND,
            "supersession requires the prior anchor to be Gamma closed-price-derived",
        ));
    }
    if grounding_kind_of(uma)? != GroundingKind::ResolvedUma {
        return Err(PolyError::grounding(
            ERR_GAMMA_SUPERSESSION_KIND,
            "supersession target must be a resolved UMA outcome anchor",
        ));
    }
    let gamma_outcome = anchor_bool(gamma)?;
    let uma_outcome = anchor_bool(uma)?;
    Ok(ResolutionSupersession {
        kind: if gamma_outcome == uma_outcome {
            ResolutionSupersessionKind::UpgradeOnAgreement
        } else {
            ResolutionSupersessionKind::CorrectionOnDisagreement
        },
        from: TrustTag::Provisional,
        to: TrustTag::Trusted,
        gamma_source: gamma.source.clone(),
        uma_source: uma.source.clone(),
        gamma_outcome,
        uma_outcome,
    })
}

/// Promotes a Provisional record grounded on `proxy` to Trusted when the real outcome `resolved`
/// backfills it (issue #209 §2). This is the trust-transition primitive the #77 backfill scheduler
/// calls on resolution; it never promotes silently:
///
/// - `resolved` must be a genuine [`GroundingKind::ResolvedUma`] anchor with a boolean outcome —
///   otherwise fail closed (no promotion from a non-resolved anchor).
/// - `proxy` must be a [`GroundingKind::Proxy`] anchor with a boolean outcome.
/// - If the resolved outcome contradicts the proxy's prediction, fail closed with
///   [`ERR_BACKFILL_CONTRADICTION`]: a proxy that pointed the wrong way is not silently promoted —
///   the caller (#77) must re-measure against the resolved anchor, not upgrade a wrong estimate.
///
/// Returns the [`TrustTransition`] audit record on success.
pub fn promote_on_resolution(proxy: &Anchor, resolved: &Anchor) -> Result<TrustTransition> {
    let GroundingKind::Proxy(proxy_kind) = grounding_kind_of(proxy)? else {
        return Err(PolyError::grounding(
            ERR_BACKFILL_NOT_RESOLVED,
            "promote_on_resolution requires a proxy anchor as the record's current grounding",
        ));
    };
    if grounding_kind_of(resolved)? != GroundingKind::ResolvedUma {
        return Err(PolyError::grounding(
            ERR_BACKFILL_NOT_RESOLVED,
            "promote_on_resolution requires a resolved UMA outcome anchor to promote to Trusted",
        ));
    }
    let proxy_outcome = anchor_bool(proxy)?;
    let resolved_outcome = anchor_bool(resolved)?;
    if proxy_outcome != resolved_outcome {
        return Err(PolyError::grounding(
            ERR_BACKFILL_CONTRADICTION,
            format!(
                "resolved outcome {resolved_outcome} contradicts proxy '{}' prediction \
                 {proxy_outcome}; re-measure against the resolved anchor, do not promote a wrong estimate",
                proxy.source
            ),
        ));
    }
    Ok(TrustTransition {
        from: TrustTag::Provisional,
        to: TrustTag::Trusted,
        proxy_source: proxy.source.clone(),
        proxy_kind,
        resolved_source: resolved.source.clone(),
        proxy_outcome,
        resolved_outcome,
    })
}

fn anchor_bool(anchor: &Anchor) -> Result<bool> {
    match &anchor.value {
        AnchorValue::Bool(v) => Ok(*v),
        other => Err(PolyError::grounding(
            ERR_BACKFILL_CONTRADICTION,
            format!("trust transition requires boolean anchor outcomes, got {other:?}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_anchor_enforces_open_interval() {
        let ok = proxy_anchor(ProxyKind::Up24h, true, 0.7, 1_785_000_000).unwrap();
        assert_eq!(ok.confidence, 0.7);
        assert_eq!(ok.observed_at, 1_785_000_000 * 1000);
        assert_eq!(ok.source, "proxy:up_24h");
        // 1.0 is certainty — forbidden for a proxy.
        let err = proxy_anchor(ProxyKind::Up24h, true, 1.0, 1).unwrap_err();
        assert_eq!(err.code(), ERR_PROXY_CONFIDENCE);
        // 0.0 and non-finite are forbidden too.
        assert_eq!(
            proxy_anchor(ProxyKind::Up1h, false, 0.0, 1)
                .unwrap_err()
                .code(),
            ERR_PROXY_CONFIDENCE
        );
        assert_eq!(
            proxy_anchor(ProxyKind::Up1h, false, f64::NAN, 1)
                .unwrap_err()
                .code(),
            ERR_PROXY_CONFIDENCE
        );
    }

    #[test]
    fn classifies_and_derives_trust() {
        let proxy = proxy_anchor(ProxyKind::Crossed05, true, 0.55, 1).unwrap();
        assert_eq!(
            grounding_kind_of(&proxy).unwrap(),
            GroundingKind::Proxy(ProxyKind::Crossed05)
        );
        assert_eq!(
            grounding_kind_of(&proxy).unwrap().trust(),
            TrustTag::Provisional
        );

        let resolved = Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(true),
            source: "uma:polymarket:YES".to_string(),
            observed_at: 1000,
            confidence: 1.0,
        };
        assert_eq!(
            grounding_kind_of(&resolved).unwrap(),
            GroundingKind::ResolvedUma
        );
        assert_eq!(
            grounding_kind_of(&resolved).unwrap().trust(),
            TrustTag::Trusted
        );
    }

    #[test]
    fn resolved_anchor_below_certainty_fails_closed() {
        let bad = Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(true),
            source: "uma:x".to_string(),
            observed_at: 1,
            confidence: 0.9,
        };
        assert_eq!(
            grounding_kind_of(&bad).unwrap_err().code(),
            ERR_RESOLVED_CONFIDENCE
        );
    }

    #[test]
    fn unknown_source_fails_closed() {
        let bad = Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(true),
            source: "guess:x".to_string(),
            observed_at: 1,
            confidence: 1.0,
        };
        assert_eq!(
            grounding_kind_of(&bad).unwrap_err().code(),
            ERR_UNKNOWN_GROUNDING
        );
    }

    #[test]
    fn rollup_is_provisional_if_any_proxy() {
        let resolved = Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(true),
            source: "uma:x".to_string(),
            observed_at: 1,
            confidence: 1.0,
        };
        let proxy = proxy_anchor(ProxyKind::Up1h, true, 0.6, 1).unwrap();
        assert_eq!(
            rollup_trust(std::slice::from_ref(&resolved)).unwrap(),
            TrustTag::Trusted
        );
        assert_eq!(
            rollup_trust(&[resolved.clone(), proxy.clone()]).unwrap(),
            TrustTag::Provisional
        );
        assert_eq!(rollup_trust(&[]).unwrap_err().code(), ERR_NO_GROUNDING);
    }
}
