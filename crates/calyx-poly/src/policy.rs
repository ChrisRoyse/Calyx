//! Local-only runtime policy.
//!
//! Poly is a read-only forecasting system. This module is the fail-closed boundary every future
//! service or agent should call before doing work with external systems.

use serde::{Deserialize, Serialize};

/// Runtime action classes Poly can evaluate before execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolyAction {
    ReadPublicData,
    IngestSnapshot,
    UpdateAssociations,
    WriteForecastArtifact,
    AdmitForecast,
    ScoreForecast,
    LaunchForecastAgent,
    RunScheduler,
    UseTradingWebsite,
    SubmitClobOrder,
    SignEip712Order,
    DeriveL1TradingCredentials,
    DeriveL2TradingCredentials,
    DeriveTradingCredentials,
    SignOrder,
    SubmitOrder,
    CancelOrder,
    MonitorUserOrders,
    RedeemPosition,
    ManageBankroll,
    StartLiveExecutor,
}

impl PolyAction {
    /// Stable snake-case label for persisted policy evidence.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadPublicData => "read_public_data",
            Self::IngestSnapshot => "ingest_snapshot",
            Self::UpdateAssociations => "update_associations",
            Self::WriteForecastArtifact => "write_forecast_artifact",
            Self::AdmitForecast => "admit_forecast",
            Self::ScoreForecast => "score_forecast",
            Self::LaunchForecastAgent => "launch_forecast_agent",
            Self::RunScheduler => "run_scheduler",
            Self::UseTradingWebsite => "use_trading_website",
            Self::SubmitClobOrder => "submit_clob_order",
            Self::SignEip712Order => "sign_eip712_order",
            Self::DeriveL1TradingCredentials => "derive_l1_trading_credentials",
            Self::DeriveL2TradingCredentials => "derive_l2_trading_credentials",
            Self::DeriveTradingCredentials => "derive_trading_credentials",
            Self::SignOrder => "sign_order",
            Self::SubmitOrder => "submit_order",
            Self::CancelOrder => "cancel_order",
            Self::MonitorUserOrders => "monitor_user_orders",
            Self::RedeemPosition => "redeem_position",
            Self::ManageBankroll => "manage_bankroll",
            Self::StartLiveExecutor => "start_live_executor",
        }
    }

    /// Every trading-capable action that must be refused under #159/#162.
    pub const FORBIDDEN_TRADING_ACTIONS: [Self; 13] = [
        Self::UseTradingWebsite,
        Self::SubmitClobOrder,
        Self::SignEip712Order,
        Self::DeriveL1TradingCredentials,
        Self::DeriveL2TradingCredentials,
        Self::DeriveTradingCredentials,
        Self::SignOrder,
        Self::SubmitOrder,
        Self::CancelOrder,
        Self::MonitorUserOrders,
        Self::RedeemPosition,
        Self::ManageBankroll,
        Self::StartLiveExecutor,
    ];
}

/// Static local-only policy flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalOnlyPolicy {
    pub allow_forecast_agents: bool,
    pub require_infisical_for_llm: bool,
}

impl Default for LocalOnlyPolicy {
    fn default() -> Self {
        Self {
            allow_forecast_agents: true,
            require_infisical_for_llm: true,
        }
    }
}

/// Fail-closed policy decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub code: String,
    pub reason: String,
}

impl PolicyDecision {
    fn allow(reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            code: "CALYX_POLY_POLICY_ALLOWED".to_string(),
            reason: reason.into(),
        }
    }

    fn refuse(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            code: code.into(),
            reason: reason.into(),
        }
    }
}

impl LocalOnlyPolicy {
    /// Enforces the local-only project boundary for one requested action.
    pub fn enforce(&self, action: PolyAction) -> PolicyDecision {
        match action {
            PolyAction::ReadPublicData
            | PolyAction::IngestSnapshot
            | PolyAction::UpdateAssociations
            | PolyAction::WriteForecastArtifact
            | PolyAction::AdmitForecast
            | PolyAction::ScoreForecast
            | PolyAction::RunScheduler => PolicyDecision::allow("local read/forecast action"),
            PolyAction::LaunchForecastAgent if self.allow_forecast_agents => {
                if self.require_infisical_for_llm {
                    PolicyDecision::allow("forecast agent allowed with Infisical-managed secrets")
                } else {
                    PolicyDecision::refuse(
                        "CALYX_POLY_POLICY_INFISICAL_REQUIRED",
                        "forecast agents require Infisical-managed LLM secrets",
                    )
                }
            }
            PolyAction::LaunchForecastAgent => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_AGENT_DISABLED",
                "forecast agents are disabled by policy",
            ),
            PolyAction::UseTradingWebsite => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_TRADING_SURFACE_FORBIDDEN",
                "Polymarket trading surfaces are forbidden",
            ),
            PolyAction::SubmitClobOrder => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_CLOB_ORDER_SUBMISSION_FORBIDDEN",
                "CLOB order submission is forbidden",
            ),
            PolyAction::SignEip712Order => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_EIP712_ORDER_SIGNING_FORBIDDEN",
                "EIP-712 order signing is forbidden",
            ),
            PolyAction::DeriveL1TradingCredentials => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_L1_TRADING_CREDENTIALS_FORBIDDEN",
                "L1 trading credential derivation is forbidden",
            ),
            PolyAction::DeriveL2TradingCredentials => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_L2_TRADING_CREDENTIALS_FORBIDDEN",
                "L2 trading credential derivation is forbidden",
            ),
            PolyAction::DeriveTradingCredentials => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_TRADING_CREDENTIALS_FORBIDDEN",
                "trading credential derivation is forbidden",
            ),
            PolyAction::SignOrder => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_ORDER_SIGNING_FORBIDDEN",
                "order signing is forbidden",
            ),
            PolyAction::SubmitOrder => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_ORDER_SUBMISSION_FORBIDDEN",
                "order submission is forbidden",
            ),
            PolyAction::CancelOrder => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_ORDER_CANCEL_FORBIDDEN",
                "order cancellation is forbidden because Poly does not manage orders",
            ),
            PolyAction::MonitorUserOrders => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_USER_ORDER_STREAM_FORBIDDEN",
                "user order stream monitoring is forbidden",
            ),
            PolyAction::RedeemPosition => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_REDEMPTION_FORBIDDEN",
                "position redemption is forbidden",
            ),
            PolyAction::ManageBankroll => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_BANKROLL_FORBIDDEN",
                "bankroll management is forbidden",
            ),
            PolyAction::StartLiveExecutor => PolicyDecision::refuse(
                "CALYX_POLY_POLICY_LIVE_EXECUTOR_FORBIDDEN",
                "live executor startup is forbidden",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_local_forecast_actions() {
        let policy = LocalOnlyPolicy::default();
        assert!(policy.enforce(PolyAction::ReadPublicData).allowed);
        assert!(policy.enforce(PolyAction::IngestSnapshot).allowed);
        assert!(policy.enforce(PolyAction::UpdateAssociations).allowed);
        assert!(policy.enforce(PolyAction::WriteForecastArtifact).allowed);
        assert!(policy.enforce(PolyAction::AdmitForecast).allowed);
        assert!(policy.enforce(PolyAction::ScoreForecast).allowed);
        assert!(policy.enforce(PolyAction::LaunchForecastAgent).allowed);
        assert!(policy.enforce(PolyAction::RunScheduler).allowed);
    }

    #[test]
    fn refuses_trading_actions() {
        let policy = LocalOnlyPolicy::default();
        let cases = [
            (
                PolyAction::UseTradingWebsite,
                "CALYX_POLY_POLICY_TRADING_SURFACE_FORBIDDEN",
            ),
            (
                PolyAction::SubmitClobOrder,
                "CALYX_POLY_POLICY_CLOB_ORDER_SUBMISSION_FORBIDDEN",
            ),
            (
                PolyAction::SignEip712Order,
                "CALYX_POLY_POLICY_EIP712_ORDER_SIGNING_FORBIDDEN",
            ),
            (
                PolyAction::DeriveL1TradingCredentials,
                "CALYX_POLY_POLICY_L1_TRADING_CREDENTIALS_FORBIDDEN",
            ),
            (
                PolyAction::DeriveL2TradingCredentials,
                "CALYX_POLY_POLICY_L2_TRADING_CREDENTIALS_FORBIDDEN",
            ),
            (
                PolyAction::MonitorUserOrders,
                "CALYX_POLY_POLICY_USER_ORDER_STREAM_FORBIDDEN",
            ),
            (
                PolyAction::ManageBankroll,
                "CALYX_POLY_POLICY_BANKROLL_FORBIDDEN",
            ),
            (
                PolyAction::StartLiveExecutor,
                "CALYX_POLY_POLICY_LIVE_EXECUTOR_FORBIDDEN",
            ),
        ];

        for (action, code) in cases {
            let decision = policy.enforce(action);
            assert!(!decision.allowed);
            assert_eq!(decision.code, code);
        }
    }

    #[test]
    fn refuses_agent_without_infisical_requirement() {
        let policy = LocalOnlyPolicy {
            require_infisical_for_llm: false,
            ..LocalOnlyPolicy::default()
        };
        let decision = policy.enforce(PolyAction::LaunchForecastAgent);
        assert!(!decision.allowed);
        assert_eq!(decision.code, "CALYX_POLY_POLICY_INFISICAL_REQUIRED");
    }
}
