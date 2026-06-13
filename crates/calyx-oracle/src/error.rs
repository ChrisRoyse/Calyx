//! Structured Oracle error catalog.

use std::error::Error;
use std::fmt;

use calyx_core::{CalyxError, LensId};

use crate::types::{DomainId, SufficiencyBound};

pub const CALYX_ORACLE_INSUFFICIENT: &str = "CALYX_ORACLE_INSUFFICIENT";
pub const CALYX_ORACLE_FLAKY_ANCHOR: &str = "CALYX_ORACLE_FLAKY_ANCHOR";
pub const CALYX_ORACLE_NO_RECURRENCE: &str = "CALYX_ORACLE_NO_RECURRENCE";
pub const CALYX_ORACLE_DOMAIN_NOT_FOUND: &str = "CALYX_ORACLE_DOMAIN_NOT_FOUND";
pub const CALYX_ORACLE_LEDGER_WRITE_FAILURE: &str = "CALYX_ORACLE_LEDGER_WRITE_FAILURE";
pub const CALYX_ORACLE_SLOT_CONFLICT: &str = "CALYX_ORACLE_SLOT_CONFLICT";

#[derive(Debug, Clone, PartialEq)]
pub enum OracleError {
    Insufficient {
        bound: SufficiencyBound,
    },
    FlakyAnchor {
        self_consistency: f32,
    },
    NoRecurrence {
        domain: DomainId,
    },
    DomainNotFound,
    LedgerWriteFailure,
    SlotConflict {
        overlap: Vec<LensId>,
        missing: Vec<LensId>,
        extra: Vec<LensId>,
        tag_mismatch: Vec<LensId>,
    },
    AssayFailure {
        source: CalyxError,
    },
}

impl OracleError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Insufficient { .. } => CALYX_ORACLE_INSUFFICIENT,
            Self::FlakyAnchor { .. } => CALYX_ORACLE_FLAKY_ANCHOR,
            Self::NoRecurrence { .. } => CALYX_ORACLE_NO_RECURRENCE,
            Self::DomainNotFound => CALYX_ORACLE_DOMAIN_NOT_FOUND,
            Self::LedgerWriteFailure => CALYX_ORACLE_LEDGER_WRITE_FAILURE,
            Self::SlotConflict { .. } => CALYX_ORACLE_SLOT_CONFLICT,
            Self::AssayFailure { source } => source.code,
        }
    }

    pub fn remediation(&self) -> &'static str {
        match self {
            Self::Insufficient { .. } => "add outcome/execution lenses before prediction",
            Self::FlakyAnchor { .. } => {
                "re-measure the grounded oracle anchor and quarantine flaky outcomes"
            }
            Self::NoRecurrence { .. } => "collect grounded recurrence pairs for the domain",
            Self::DomainNotFound => "register the oracle domain before prediction",
            Self::LedgerWriteFailure => "retry after repairing the ledger write path",
            Self::SlotConflict { .. } => {
                "make clamp/free disjoint and exhaustive; tag clamped slots measured and free slots inferred or provisional"
            }
            Self::AssayFailure { source } => source.remediation,
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
            Self::SlotConflict {
                overlap,
                missing,
                extra,
                tag_mismatch,
            } => format!(
                "completion slot partition conflict: overlap=[{}], missing=[{}], extra=[{}], tag_mismatch=[{}]",
                format_lens_ids(overlap),
                format_lens_ids(missing),
                format_lens_ids(extra),
                format_lens_ids(tag_mismatch)
            ),
            Self::AssayFailure { source } => {
                format!("assay sufficiency evidence unavailable: {}", source.message)
            }
        }
    }
}

fn format_lens_ids(ids: &[LensId]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
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

impl From<CalyxError> for OracleError {
    fn from(source: CalyxError) -> Self {
        Self::AssayFailure { source }
    }
}

impl From<OracleError> for CalyxError {
    fn from(error: OracleError) -> Self {
        match error {
            OracleError::AssayFailure { source } => source,
            other => Self {
                code: other.code(),
                message: other.message(),
                remediation: other.remediation(),
            },
        }
    }
}
