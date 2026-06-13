//! Consent and purpose-tag checks for privacy-governed Calyx processing.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{CalyxError, Result, Ts};

/// Module-local consent violation code for PH61 privacy gates.
pub const CALYX_CONSENT_VIOLATION: &str = "CALYX_CONSENT_VIOLATION";

/// Consent timestamps use Calyx server timestamp units: Unix milliseconds.
pub type Timestamp = Ts;

/// Lawful basis attached to a consent tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LawfulBasis {
    Consent,
    LegitimateInterest,
    ContractPerformance,
    LegalObligation,
    VitalInterests,
    PublicTask,
}

impl fmt::Display for LawfulBasis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Consent => "consent",
            Self::LegitimateInterest => "legitimate_interest",
            Self::ContractPerformance => "contract_performance",
            Self::LegalObligation => "legal_obligation",
            Self::VitalInterests => "vital_interests",
            Self::PublicTask => "public_task",
        })
    }
}

/// Declared purpose for a Calyx operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Search,
    Intelligence,
    Reranking,
    Analytics,
    Export,
    AuditOnly,
}

/// Consent metadata carried by a constellation or vault-wide default policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentTag {
    pub lawful_basis: LawfulBasis,
    pub permitted_purposes: Vec<Purpose>,
    pub expires_at: Option<Timestamp>,
}

/// Returns true when a consent tag is past its validity window.
pub fn consent_expired(tag: &ConsentTag, now: Timestamp) -> bool {
    tag.expires_at.is_some_and(|expires_at| now >= expires_at)
}

/// Fail-closed purpose check for downstream processing.
pub fn check_consent(tag: &ConsentTag, requested_purpose: Purpose, now: Timestamp) -> Result<()> {
    if requested_purpose == Purpose::AuditOnly {
        return Ok(());
    }
    if consent_expired(tag, now) {
        return Err(consent_violation(format!(
            "consent for lawful basis {} expired before purpose {:?}",
            tag.lawful_basis, requested_purpose
        )));
    }
    if tag.permitted_purposes.contains(&requested_purpose) {
        Ok(())
    } else {
        Err(consent_violation(format!(
            "purpose {:?} is not permitted by consent tag",
            requested_purpose
        )))
    }
}

fn consent_violation(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_CONSENT_VIOLATION,
        message: message.into(),
        remediation: "request only permitted purposes or refresh consent before processing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_785_400_000_000;

    fn tag(permitted_purposes: Vec<Purpose>, expires_at: Option<Timestamp>) -> ConsentTag {
        ConsentTag {
            lawful_basis: LawfulBasis::Consent,
            permitted_purposes,
            expires_at,
        }
    }

    #[test]
    fn check_consent_rejects_unpermitted_purpose() {
        let tag = tag(vec![Purpose::Search], Some(NOW + 1));
        let error = check_consent(&tag, Purpose::Intelligence, NOW).unwrap_err();
        assert_eq!(error.code, CALYX_CONSENT_VIOLATION);
    }

    #[test]
    fn check_consent_accepts_permitted_non_expired_purpose() {
        let tag = tag(vec![Purpose::Search, Purpose::Intelligence], Some(NOW + 1));
        assert!(check_consent(&tag, Purpose::Intelligence, NOW).is_ok());
    }

    #[test]
    fn expired_consent_rejects_even_permitted_purpose() {
        let tag = tag(vec![Purpose::Search], Some(NOW - 1));
        let error = check_consent(&tag, Purpose::Search, NOW).unwrap_err();
        assert_eq!(error.code, CALYX_CONSENT_VIOLATION);
    }

    #[test]
    fn edge_cases_cover_empty_audit_and_indefinite_consent() {
        let empty = tag(Vec::new(), Some(NOW + 1));
        let error = check_consent(&empty, Purpose::Search, NOW).unwrap_err();
        println!("EDGE_EMPTY_BEFORE permitted=[] requested=Search");
        println!("EDGE_EMPTY_AFTER Err({})", error.code);
        assert_eq!(error.code, CALYX_CONSENT_VIOLATION);

        println!("EDGE_AUDIT_BEFORE permitted=[] requested=AuditOnly");
        println!(
            "EDGE_AUDIT_AFTER {:?}",
            check_consent(&empty, Purpose::AuditOnly, NOW)
        );
        assert!(check_consent(&empty, Purpose::AuditOnly, NOW).is_ok());

        let indefinite = tag(vec![Purpose::Analytics], None);
        let expired = consent_expired(&indefinite, Timestamp::MAX);
        let allowed = check_consent(&indefinite, Purpose::Analytics, NOW);
        println!("EDGE_INDEFINITE_BEFORE expires_at=None now=Timestamp::MAX");
        println!("EDGE_INDEFINITE_AFTER expired={expired} check={allowed:?}");
        assert!(!expired);
        assert!(allowed.is_ok());
    }

    #[test]
    fn fail_closed_processing_does_not_return_constellation() {
        fn guarded_constellation(tag: &ConsentTag, purpose: Purpose) -> Result<&'static str> {
            check_consent(tag, purpose, NOW)?;
            Ok("synthetic-constellation")
        }

        let tag = tag(vec![Purpose::Search], Some(NOW + 1));
        let error = guarded_constellation(&tag, Purpose::Export).unwrap_err();
        println!("FAIL_CLOSED_BEFORE requested=Export return=synthetic-constellation");
        println!(
            "FAIL_CLOSED_AFTER Err({}) no_constellation_returned",
            error.code
        );
        assert_eq!(error.code, CALYX_CONSENT_VIOLATION);
    }

    #[test]
    fn lawful_basis_display_is_stable() {
        assert_eq!(LawfulBasis::Consent.to_string(), "consent");
        assert_eq!(
            LawfulBasis::LegitimateInterest.to_string(),
            "legitimate_interest"
        );
        assert_eq!(
            LawfulBasis::ContractPerformance.to_string(),
            "contract_performance"
        );
        assert_eq!(LawfulBasis::LegalObligation.to_string(), "legal_obligation");
        assert_eq!(LawfulBasis::VitalInterests.to_string(), "vital_interests");
        assert_eq!(LawfulBasis::PublicTask.to_string(), "public_task");
    }

    #[test]
    fn consent_fsv_readback_prints_known_outcomes() {
        let denied = tag(vec![Purpose::Search], Some(NOW + 1));
        let denied_error = check_consent(&denied, Purpose::Intelligence, NOW).unwrap_err();
        println!(
            "check_consent(Intelligence, permitted=[Search]) = Err({})",
            denied_error.code
        );

        let allowed = tag(vec![Purpose::Search, Purpose::Intelligence], Some(NOW + 1));
        println!(
            "check_consent(Intelligence, permitted=[Search, Intelligence]) = {:?}",
            check_consent(&allowed, Purpose::Intelligence, NOW)
        );

        assert_eq!(denied_error.code, CALYX_CONSENT_VIOLATION);
        assert!(check_consent(&allowed, Purpose::Intelligence, NOW).is_ok());
    }
}
