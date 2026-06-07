//! Grounded outcome anchors.

use serde::{Deserialize, Serialize};

use crate::AnchorKind;

use super::Ts;

/// A grounded real-outcome observation attached to a constellation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    /// Outcome axis.
    pub kind: AnchorKind,
    /// Observed value on the axis.
    pub value: AnchorValue,
    /// Oracle, human labeler, reward source, or external reality source.
    pub source: String,
    /// Server-observed timestamp.
    pub observed_at: Ts,
    /// Confidence in `[0, 1]`; deterministic oracles use `1.0`.
    pub confidence: f32,
}

/// Value carried by a grounded anchor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorValue {
    /// Boolean outcome.
    Bool(bool),
    /// Named categorical outcome.
    Enum(String),
    /// Numeric outcome or reward.
    Number(f64),
    /// One-hot categorical support.
    OneHot(Vec<String>),
    /// Textual label when the source cannot reduce to a category yet.
    Text(String),
}
