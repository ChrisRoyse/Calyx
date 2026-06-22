//! PH50 super-intelligence predicate and reverse-query data contracts.

use std::fmt;

use calyx_core::LedgerRef;
use serde::{Deserialize, Serialize};

use crate::types::DomainId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    OracleClean,
    PanelSufficient,
    KernelExists,
    Calibrated,
    GoodhartDefended,
    MistakeClosed,
}

impl Tier {
    pub const ORDER: [Tier; 6] = [
        Tier::OracleClean,
        Tier::PanelSufficient,
        Tier::KernelExists,
        Tier::Calibrated,
        Tier::GoodhartDefended,
        Tier::MistakeClosed,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Tier::OracleClean => "oracle_clean",
            Tier::PanelSufficient => "panel_sufficient",
            Tier::KernelExists => "kernel_exists",
            Tier::Calibrated => "calibrated",
            Tier::GoodhartDefended => "goodhart_defended",
            Tier::MistakeClosed => "mistake_closed",
        }
    }

    pub fn predicate_order() -> &'static [Tier; 6] {
        &Self::ORDER
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierResult {
    pub tier: Tier,
    pub passed: bool,
    pub measured_value: f32,
    pub threshold: f32,
    pub cheapest_fix: Option<String>,
}

impl TierResult {
    pub fn new(
        tier: Tier,
        passed: bool,
        measured_value: f32,
        threshold: f32,
        cheapest_fix: Option<String>,
    ) -> Self {
        Self {
            tier,
            passed,
            measured_value,
            threshold,
            cheapest_fix,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuperIntelReport {
    pub domain: DomainId,
    pub tiers: Vec<TierResult>,
    pub failing_tier: Option<Tier>,
    pub cheapest_fix: Option<String>,
    pub overall: bool,
}

impl SuperIntelReport {
    pub fn new(domain: DomainId, tiers: Vec<TierResult>) -> Self {
        let overall = tiers.iter().all(|tier| tier.passed);
        let failing_tier = first_failing_tier(&tiers);
        let cheapest_fix = failing_tier.and_then(|failing| {
            tiers
                .iter()
                .find(|tier| tier.tier == failing)
                .and_then(|tier| tier.cheapest_fix.clone())
        });

        Self {
            domain,
            tiers,
            failing_tier,
            cheapest_fix,
            overall,
        }
    }

    pub fn failing_tier_report(&self) -> Option<&TierResult> {
        let failing_tier = self.failing_tier?;
        self.tiers.iter().find(|tier| tier.tier == failing_tier)
    }

    pub fn passed_count(&self) -> usize {
        self.tiers.iter().filter(|tier| tier.passed).count()
    }

    pub fn failed_count(&self) -> usize {
        self.tiers.len().saturating_sub(self.passed_count())
    }
}

impl fmt::Display for SuperIntelReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let failing = self
            .failing_tier
            .map(|tier| tier.to_string())
            .unwrap_or_else(|| "none".to_string());
        write!(
            formatter,
            "super_intelligence({}): {} passed/{} failed across {} tiers; failing_tier={}",
            self.domain,
            self.passed_count(),
            self.failed_count(),
            self.tiers.len(),
            failing
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cause {
    pub action_or_event: String,
    pub domain: DomainId,
    pub confidence: f32,
    pub provisional: bool,
    pub provenance: LedgerRef,
}

fn first_failing_tier(tiers: &[TierResult]) -> Option<Tier> {
    Tier::predicate_order().iter().copied().find(|ordered| {
        tiers
            .iter()
            .any(|tier| tier.tier == *ordered && !tier.passed)
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use calyx_core::LedgerRef;
    use proptest::prelude::*;
    use serde::Serialize;

    use super::*;

    #[test]
    fn super_intel_types_tier_order_has_six_prd21_variants() {
        assert_eq!(
            Tier::ORDER,
            [
                Tier::OracleClean,
                Tier::PanelSufficient,
                Tier::KernelExists,
                Tier::Calibrated,
                Tier::GoodhartDefended,
                Tier::MistakeClosed,
            ]
        );
        assert_eq!(Tier::ORDER.len(), 6);
    }

    #[test]
    fn super_intel_types_first_three_pass_tier_four_fails() {
        let report = SuperIntelReport::new(
            DomainId::from("ph50-calibration-fixture"),
            vec![
                pass(Tier::OracleClean),
                pass(Tier::PanelSufficient),
                pass(Tier::KernelExists),
                fail(Tier::Calibrated),
                pass(Tier::GoodhartDefended),
                pass(Tier::MistakeClosed),
            ],
        );

        assert!(!report.overall);
        assert_eq!(report.failing_tier, Some(Tier::Calibrated));
        assert_eq!(
            report.failing_tier_report().map(|tier| tier.tier),
            Some(Tier::Calibrated)
        );
    }

    #[test]
    fn super_intel_types_all_six_pass() {
        let report = SuperIntelReport::new(DomainId::from("ph50-clean-fixture"), all_pass());

        assert!(report.overall);
        assert_eq!(report.failing_tier, None);
        assert!(report.failing_tier_report().is_none());
    }

    #[test]
    fn super_intel_types_display_names_failing_tier() {
        let report = SuperIntelReport::new(
            DomainId::from("ph50-display-fixture"),
            vec![pass(Tier::OracleClean), fail(Tier::Calibrated)],
        );

        let display = report.to_string();
        assert!(display.contains("calibrated"));
        assert!(display.contains("1 passed/1 failed"));
    }

    #[test]
    fn super_intel_types_empty_tiers_are_vacuously_overall_true() {
        let report = SuperIntelReport::new(DomainId::from("ph50-empty-fixture"), Vec::new());

        assert!(report.overall);
        assert_eq!(report.failing_tier, None);
        assert_eq!(report.passed_count(), 0);
        assert_eq!(report.failed_count(), 0);
    }

    #[test]
    fn super_intel_types_single_fail_reports_that_tier() {
        let report = SuperIntelReport::new(
            DomainId::from("ph50-single-fail-fixture"),
            vec![fail(Tier::GoodhartDefended)],
        );

        assert!(!report.overall);
        assert_eq!(report.failing_tier, Some(Tier::GoodhartDefended));
    }

    #[test]
    fn super_intel_types_all_fail_reports_oracle_clean_first() {
        let report = SuperIntelReport::new(
            DomainId::from("ph50-all-fail-fixture"),
            Tier::predicate_order().iter().copied().map(fail).collect(),
        );

        assert!(!report.overall);
        assert_eq!(report.failing_tier, Some(Tier::OracleClean));
    }

    #[test]
    fn super_intel_types_failing_tier_uses_predicate_order() {
        let report = SuperIntelReport::new(
            DomainId::from("ph50-order-fixture"),
            vec![fail(Tier::Calibrated), fail(Tier::OracleClean)],
        );

        assert_eq!(report.failing_tier, Some(Tier::OracleClean));
    }

    #[test]
    fn super_intel_types_serde_roundtrip_is_byte_identical() {
        let report = SuperIntelReport::new(
            DomainId::from("ph50-serde-fixture"),
            vec![
                pass(Tier::OracleClean),
                pass(Tier::PanelSufficient),
                fail(Tier::KernelExists),
            ],
        );

        let first = serde_json::to_vec(&report).expect("serialize report");
        let decoded: SuperIntelReport = serde_json::from_slice(&first).expect("decode report");
        let second = serde_json::to_vec(&decoded).expect("reserialize report");

        assert_eq!(decoded, report);
        assert_eq!(first, second);
    }

    #[test]
    fn super_intel_types_cause_roundtrips_with_provisional_flag() {
        let cause = Cause {
            action_or_event: "label outcome-execution anchor".to_string(),
            domain: DomainId::from("ph50-cause-fixture"),
            confidence: 0.75,
            provisional: true,
            provenance: ledger(42),
        };

        let json = serde_json::to_vec(&cause).expect("serialize cause");
        let decoded: Cause = serde_json::from_slice(&json).expect("decode cause");

        assert_eq!(decoded, cause);
        assert!(decoded.provisional);
    }

    #[test]
    #[ignore = "manual FSV for issue #435 PH50 T01 type readbacks"]
    fn issue435_super_intel_types_fsv_writes_readbacks() {
        let root = std::env::var_os("CALYX_FSV_ROOT")
            .map(std::path::PathBuf::from)
            .expect("set CALYX_FSV_ROOT");
        fs::create_dir_all(&root).expect("create PH50 T01 FSV root");

        let tier_four_fails = SuperIntelReport::new(
            DomainId::from("ph50-fsv-tier-four"),
            vec![
                pass(Tier::OracleClean),
                pass(Tier::PanelSufficient),
                pass(Tier::KernelExists),
                fail(Tier::Calibrated),
                pass(Tier::GoodhartDefended),
                pass(Tier::MistakeClosed),
            ],
        );
        let all_six_pass = SuperIntelReport::new(DomainId::from("ph50-fsv-all-pass"), all_pass());
        let empty_tiers = SuperIntelReport::new(DomainId::from("ph50-fsv-empty"), Vec::new());
        let single_fail = SuperIntelReport::new(
            DomainId::from("ph50-fsv-single-fail"),
            vec![fail(Tier::MistakeClosed)],
        );
        let all_fail = SuperIntelReport::new(
            DomainId::from("ph50-fsv-all-fail"),
            Tier::predicate_order().iter().copied().map(fail).collect(),
        );
        let cause = Cause {
            action_or_event: "outcome execution lens missing".to_string(),
            domain: DomainId::from("ph50-fsv-cause"),
            confidence: 0.75,
            provisional: true,
            provenance: ledger(435),
        };

        write_json(&root.join("tier-four-fails.json"), &tier_four_fails);
        write_json(&root.join("all-six-pass.json"), &all_six_pass);
        write_json(&root.join("edge-empty-tiers.json"), &empty_tiers);
        write_json(&root.join("edge-single-fail.json"), &single_fail);
        write_json(&root.join("edge-all-fail.json"), &all_fail);
        write_json(&root.join("cause-provisional.json"), &cause);

        let first = serde_json::to_vec(&tier_four_fails).expect("serialize report");
        let decoded: SuperIntelReport = serde_json::from_slice(&first).expect("decode report");
        let second = serde_json::to_vec(&decoded).expect("reserialize report");
        assert_eq!(first, second);
        fs::write(
            root.join("serde-byte-identical.txt"),
            format!(
                "first_len={}\nsecond_len={}\nfirst_b3={}\nsecond_b3={}\nbyte_identical=true\n",
                first.len(),
                second.len(),
                blake3::hash(&first),
                blake3::hash(&second)
            ),
        )
        .expect("write serde byte report");
    }

    proptest! {
        #[test]
        fn super_intel_types_overall_and_failing_tier_follow_predicate_order(mask in 0u8..64) {
            let tiers = Tier::predicate_order()
                .iter()
                .enumerate()
                .map(|(index, tier)| {
                    let passed = (mask & (1 << index)) != 0;
                    TierResult::new(*tier, passed, f32::from(passed), 0.5, fix_for(*tier, passed))
                })
                .collect::<Vec<_>>();
            let report = SuperIntelReport::new(DomainId::from("ph50-prop-fixture"), tiers.clone());
            let expected_overall = tiers.iter().all(|tier| tier.passed);
            let expected_failing = first_failing_tier(&tiers);

            prop_assert_eq!(report.overall, expected_overall);
            prop_assert_eq!(report.failing_tier, expected_failing);
            prop_assert_eq!(report.failing_tier.is_none(), report.overall);
        }
    }

    fn all_pass() -> Vec<TierResult> {
        Tier::predicate_order().iter().copied().map(pass).collect()
    }

    fn pass(tier: Tier) -> TierResult {
        TierResult::new(tier, true, 1.0, 0.5, None)
    }

    fn fail(tier: Tier) -> TierResult {
        TierResult::new(tier, false, 0.25, 0.5, fix_for(tier, false))
    }

    fn fix_for(tier: Tier, passed: bool) -> Option<String> {
        (!passed).then(|| format!("repair {tier}"))
    }

    fn write_json<T: Serialize>(path: &Path, value: &T) {
        let bytes = serde_json::to_vec_pretty(value).expect("serialize fsv json");
        fs::write(path, bytes).expect("write fsv json");
    }

    fn ledger(seed: u64) -> LedgerRef {
        LedgerRef {
            seq: seed,
            hash: [seed as u8; 32],
        }
    }
}
