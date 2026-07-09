use calyx_core::{CalyxError, CalyxErrorCode};

use crate::{DomainId, OracleError};

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
