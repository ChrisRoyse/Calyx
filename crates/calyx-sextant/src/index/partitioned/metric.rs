use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartitionDistanceMetric {
    #[default]
    UnitL2,
    RawL2,
}

impl PartitionDistanceMetric {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnitL2 => "unit-l2",
            Self::RawL2 => "raw-l2",
        }
    }
}

impl FromStr for PartitionDistanceMetric {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "unit-l2" | "unit_l2" | "cosine" => Ok(Self::UnitL2),
            "raw-l2" | "raw_l2" | "l2" => Ok(Self::RawL2),
            other => Err(format!(
                "unknown partitioned distance metric {other:?}; expected unit-l2 or raw-l2"
            )),
        }
    }
}
