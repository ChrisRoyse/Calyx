use calyx_core::{CalyxError, CalyxErrorCode};

use crate::{DomainId, OracleError};

pub(crate) enum ScanError {
    Storage,
    Oracle(OracleError),
}

impl From<CalyxError> for ScanError {
    fn from(_error: CalyxError) -> Self {
        Self::Storage
    }
}

impl From<OracleError> for ScanError {
    fn from(error: OracleError) -> Self {
        Self::Oracle(error)
    }
}

pub(crate) fn storage_read(domain: &DomainId, operation: &'static str) -> OracleError {
    OracleError::StorageReadFailure {
        domain: domain.clone(),
        operation,
    }
}

pub(crate) fn corrupt(domain: &DomainId, evidence: &'static str) -> OracleError {
    OracleError::EvidenceCorrupt {
        domain: domain.clone(),
        evidence,
    }
}

pub(crate) fn recurrence_read(error: CalyxError, domain: &DomainId) -> OracleError {
    if error.code == CalyxErrorCode::AsterCorruptShard.code() {
        corrupt(domain, "recurrence series")
    } else {
        storage_read(domain, "read recurrence series")
    }
}

pub(crate) fn scan_read(
    error: ScanError,
    domain: &DomainId,
    operation: &'static str,
) -> OracleError {
    match error {
        ScanError::Storage => storage_read(domain, operation),
        ScanError::Oracle(error) => error,
    }
}
