//! Structured Oracle error catalog.

use std::error::Error;
use std::fmt;

use calyx_core::CalyxError;

use crate::types::{DomainId, SufficiencyBound};

pub const CALYX_ORACLE_INSUFFICIENT: &str = "CALYX_ORACLE_INSUFFICIENT";
pub const CALYX_ORACLE_FLAKY_ANCHOR: &str = "CALYX_ORACLE_FLAKY_ANCHOR";
pub const CALYX_ORACLE_NO_RECURRENCE: &str = "CALYX_ORACLE_NO_RECURRENCE";
pub const CALYX_ORACLE_DOMAIN_NOT_FOUND: &str = "CALYX_ORACLE_DOMAIN_NOT_FOUND";
pub const CALYX_ORACLE_LEDGER_WRITE_FAILURE: &str = "CALYX_ORACLE_LEDGER_WRITE_FAILURE";

#[derive(Debug, Clone, PartialEq)]
pub enum OracleError {
    Insufficient { bound: SufficiencyBound },
    FlakyAnchor { self_consistency: f32 },
    NoRecurrence { domain: DomainId },
    DomainNotFound,
    LedgerWriteFailure,
}

impl OracleError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Insufficient { .. } => CALYX_ORACLE_INSUFFICIENT,
            Self::FlakyAnchor { .. } => CALYX_ORACLE_FLAKY_ANCHOR,
            Self::NoRecurrence { .. } => CALYX_ORACLE_NO_RECURRENCE,
            Self::DomainNotFound => CALYX_ORACLE_DOMAIN_NOT_FOUND,
            Self::LedgerWriteFailure => CALYX_ORACLE_LEDGER_WRITE_FAILURE,
        }
    }

    pub const fn remediation(&self) -> &'static str {
        match self {
            Self::Insufficient { .. } => "add outcome/execution lenses before prediction",
            Self::FlakyAnchor { .. } => {
                "re-measure the grounded oracle anchor and quarantine flaky outcomes"
            }
            Self::NoRecurrence { .. } => "collect grounded recurrence pairs for the domain",
            Self::DomainNotFound => "register the oracle domain before prediction",
            Self::LedgerWriteFailure => "retry after repairing the ledger write path",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Insufficient { bound } => format!(
                "I(panel;oracle)={} is below the domain requirement; sufficient={}",
                bound.i_panel_oracle, bound.sufficient
            ),
            Self::FlakyAnchor { self_consistency } => format!(
                "oracle anchor self-consistency {self_consistency} is too low for a trusted ceiling"
            ),
            Self::NoRecurrence { domain } => {
                format!("domain {domain} lacks enough grounded recurrence evidence")
            }
            Self::DomainNotFound => "oracle domain was not found".to_string(),
            Self::LedgerWriteFailure => "oracle provenance ledger write failed".to_string(),
        }
    }
}

impl fmt::Display for OracleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}; remediation: {}",
            self.code(),
            self.message(),
            self.remediation()
        )
    }
}

impl Error for OracleError {}

impl From<OracleError> for CalyxError {
    fn from(error: OracleError) -> Self {
        Self {
            code: error.code(),
            message: error.message(),
            remediation: error.remediation(),
        }
    }
}
