use std::time::Duration;

use calyx_core::{LensId, Result, SlotKey};
use serde::{Deserialize, Serialize};

use super::invalid_argument;
use super::schema::validate_name;

pub const DEFAULT_TEMPORAL_BOOST_WEIGHTS: [f32; 3] = [0.50, 0.35, 0.15];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelRef {
    pub panel_version: u32,
    pub lenses: Vec<LensId>,
}

impl PanelRef {
    pub fn new(lens_id: LensId) -> Self {
        Self {
            panel_version: 1,
            lenses: vec![lens_id],
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.lenses.is_empty() {
            return Err(invalid_argument(
                "panel reference must contain at least one lens",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecondaryIndexKind {
    Btree,
    Inverted,
    Ann,
    Kernel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecondaryIndexSpec {
    pub name: String,
    pub kind: SecondaryIndexKind,
    pub fields: Vec<String>,
}

impl SecondaryIndexSpec {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_name("secondary index name", &self.name)?;
        if self.fields.is_empty() {
            return Err(invalid_argument(format!(
                "secondary index `{}` requires at least one field",
                self.name
            )));
        }
        for field in &self.fields {
            validate_name("secondary index field", field)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DedupPolicy {
    Off,
    Exact,
    TctCosine {
        required_slots: Vec<SlotKey>,
        tau: f32,
        action: DedupAction,
    },
}

impl DedupPolicy {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Self::TctCosine {
            required_slots,
            tau,
            ..
        } = self
        {
            if required_slots.is_empty() {
                return Err(invalid_argument(
                    "TctCosine dedup requires at least one required slot",
                ));
            }
            if !tau.is_finite() || *tau <= 0.0 || *tau > 1.0 {
                return Err(invalid_argument(format!(
                    "TctCosine tau must satisfy 0.0 < tau <= 1.0, got {tau}"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DedupAction {
    Reject,
    RecurrenceSeries,
    Merge,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalPolicy {
    pub boost_weights: [f32; 3],
}

impl Default for TemporalPolicy {
    fn default() -> Self {
        Self {
            boost_weights: DEFAULT_TEMPORAL_BOOST_WEIGHTS,
        }
    }
}

impl TemporalPolicy {
    pub(crate) fn validate(&self) -> Result<()> {
        let mut sum = 0.0_f32;
        for weight in self.boost_weights {
            if !weight.is_finite() || weight < 0.0 {
                return Err(invalid_argument(format!(
                    "temporal boost weights must be finite and non-negative, got {weight}"
                )));
            }
            sum += weight;
        }
        if sum > 1.0 + f32::EPSILON {
            return Err(invalid_argument(format!(
                "temporal boost weights must sum to <= 1.0, got {sum}"
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetentionPolicy {
    Forever,
    DropAfter(Duration),
    RollupOnly,
}

impl RetentionPolicy {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Self::DropAfter(duration) = self
            && duration.is_zero()
        {
            return Err(invalid_argument("DropAfter retention duration must be > 0"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IsolationLevel {
    ReadCommitted,
    Serializable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxnPolicy {
    pub isolation: IsolationLevel,
    pub cost_cap_ms: Option<u32>,
}

impl Default for TxnPolicy {
    fn default() -> Self {
        Self {
            isolation: IsolationLevel::ReadCommitted,
            cost_cap_ms: None,
        }
    }
}

impl TxnPolicy {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.cost_cap_ms == Some(0) {
            return Err(invalid_argument("txn cost_cap_ms must be > 0 when set"));
        }
        Ok(())
    }
}
