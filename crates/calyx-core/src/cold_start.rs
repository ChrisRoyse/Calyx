//! Cold-start trust-state guard for provisional vaults (PRD 30 section 5).
//!
//! A newly created vault can search immediately, but high-stakes answers must
//! not be labeled grounded until at least one real anchor has been recorded.

use serde::{Deserialize, Serialize};

use crate::error::{CalyxError, Result};

/// Module-local error for high-stakes use of an unanchored vault.
pub const CALYX_PROVISIONAL_VAULT: &str = "CALYX_PROVISIONAL_VAULT";

/// Whether a vault has enough real-world grounding to emit non-provisional
/// high-stakes results.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VaultTrustState {
    /// No grounded anchors have been recorded yet.
    Provisional,
    /// At least one grounded anchor has been recorded.
    Grounded { anchor_count: usize },
}

/// Fail-closed guard for trust-sensitive paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColdStartGuard {
    state: VaultTrustState,
}

impl ColdStartGuard {
    /// Starts every vault honestly in provisional mode.
    pub const fn new() -> Self {
        Self {
            state: VaultTrustState::Provisional,
        }
    }

    /// Returns the current trust state.
    pub const fn state(&self) -> &VaultTrustState {
        &self.state
    }

    /// Returns the number of grounded anchors seen by this guard.
    pub const fn anchor_count(&self) -> usize {
        match self.state {
            VaultTrustState::Provisional => 0,
            VaultTrustState::Grounded { anchor_count } => anchor_count,
        }
    }

    /// Records one grounded anchor and transitions to `Grounded` at count one.
    pub fn record_anchor(&mut self) {
        let anchor_count = self.anchor_count().saturating_add(1);
        self.state = VaultTrustState::Grounded { anchor_count };
    }

    /// Fails closed until at least one grounded anchor is present.
    pub fn assert_grounded(&self, operation: &str) -> Result<()> {
        match self.state {
            VaultTrustState::Grounded { anchor_count } if anchor_count >= 1 => Ok(()),
            _ => Err(CalyxError {
                code: CALYX_PROVISIONAL_VAULT,
                message: format!(
                    "operation {operation:?} requires a grounded vault; no anchors recorded"
                ),
                remediation: "record at least one grounded anchor before returning non-provisional high-stakes output",
            }),
        }
    }

    /// Search is permitted from day zero, but results remain provisional.
    pub const fn search_always_ok(&self) -> bool {
        true
    }
}

impl Default for ColdStartGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_guard_starts_provisional_and_search_is_allowed() {
        let guard = ColdStartGuard::new();

        assert_eq!(guard.state(), &VaultTrustState::Provisional);
        assert_eq!(guard.anchor_count(), 0);
        assert!(guard.search_always_ok());
    }

    #[test]
    fn provisional_guard_rejects_high_stakes_operation_with_operation_name() {
        let guard = ColdStartGuard::new();
        let error = guard.assert_grounded("oracle_answer").unwrap_err();

        assert_eq!(error.code, CALYX_PROVISIONAL_VAULT);
        assert!(error.message.contains("oracle_answer"));
    }

    #[test]
    fn first_anchor_transitions_to_grounded() {
        let mut guard = ColdStartGuard::new();

        guard.record_anchor();

        assert_eq!(
            guard.state(),
            &VaultTrustState::Grounded { anchor_count: 1 }
        );
        assert!(guard.assert_grounded("oracle_answer").is_ok());
    }

    #[test]
    fn multiple_anchors_increment_count() {
        let mut guard = ColdStartGuard::new();

        guard.record_anchor();
        guard.record_anchor();
        guard.record_anchor();

        assert_eq!(
            guard.state(),
            &VaultTrustState::Grounded { anchor_count: 3 }
        );
        assert_eq!(guard.anchor_count(), 3);
    }

    #[test]
    fn search_stays_allowed_after_grounding() {
        let mut guard = ColdStartGuard::new();
        guard.record_anchor();

        assert!(guard.search_always_ok());
    }
}
