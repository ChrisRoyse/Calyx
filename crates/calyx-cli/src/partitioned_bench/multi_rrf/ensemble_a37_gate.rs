use calyx_assay::{A37_DIVERSITY_GATE_PASSED, A37DiversityGate};
use calyx_core::CalyxError;

use crate::error::{CliError, CliResult};

pub(super) fn validate(gate: &A37DiversityGate, required: bool) -> CliResult {
    if !required {
        return Ok(());
    }
    if gate.status == A37_DIVERSITY_GATE_PASSED
        && gate.family_span_pass
        && gate.redundancy_bound_pass
        && gate.no_collapse_pass
    {
        return Ok(());
    }
    Err(CliError::Calyx(CalyxError {
        code: "CALYX_FSV_A37_ENSEMBLE_CARD_REFUSED",
        message: format!(
            "A37 ensemble card refused: status={} family_span_pass={} redundancy_bound_pass={} no_collapse_pass={} n_eff={:.6} n_eff_floor={:.6} mean_pairwise_corr={:.6} mean_pairwise_nmi={:.6}",
            gate.status,
            gate.family_span_pass,
            gate.redundancy_bound_pass,
            gate.no_collapse_pass,
            gate.n_eff,
            gate.n_eff_floor,
            gate.mean_pairwise_corr,
            gate.mean_pairwise_nmi
        ),
        remediation: "pass an A37 gate_passed EnsembleCard before using partitioned-rrf recall/SLO as gate evidence",
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use calyx_assay::{A37_DIVERSITY_DIAGNOSTIC_ONLY, A37_DIVERSITY_SCHEMA_VERSION};
    use calyx_core::SlotId;

    use super::*;

    #[test]
    fn gate_mode_accepts_a37_pass() {
        validate(&gate(A37_DIVERSITY_GATE_PASSED, true, true, true), true).unwrap();
    }

    #[test]
    fn gate_mode_rejects_diagnostic_a37_card() {
        let err = validate(
            &gate(A37_DIVERSITY_DIAGNOSTIC_ONLY, true, true, false),
            true,
        )
        .unwrap_err();

        assert_eq!(err.code(), "CALYX_FSV_A37_ENSEMBLE_CARD_REFUSED");
        assert!(err.message().contains("no_collapse_pass=false"));
    }

    #[test]
    fn diagnostic_mode_accepts_a37_refusal_for_reporting() {
        validate(
            &gate(A37_DIVERSITY_DIAGNOSTIC_ONLY, true, false, false),
            false,
        )
        .unwrap();
    }

    fn gate(
        status: &str,
        family_span_pass: bool,
        redundancy_bound_pass: bool,
        no_collapse_pass: bool,
    ) -> A37DiversityGate {
        A37DiversityGate {
            schema_version: A37_DIVERSITY_SCHEMA_VERSION,
            role: "a37_associational_diversity_gate".to_string(),
            status: status.to_string(),
            content_lens_count: 10,
            temporal_sidecar_count: 0,
            temporal_counts_toward_content_floor: false,
            temporal_lane_role: "time_manipulation_walk_forward_backward_as_of_sidecar".to_string(),
            association_family_count: 2,
            association_families: BTreeMap::from([
                ("dense_semantic_general".to_string(), vec![SlotId::new(0)]),
                ("lexical_sparse".to_string(), vec![SlotId::new(1)]),
            ]),
            temporal_sidecar_slots: Vec::new(),
            family_span_pass,
            redundancy_bound_pass,
            no_collapse_pass,
            n_eff: 8.5,
            n_eff_floor: 6.0,
            mean_pairwise_corr: 0.2,
            mean_pairwise_nmi: 0.1,
            max_redundancy: 0.6,
            sum_unique_pid_bits: 0.5,
            min_marginal_bits: 0.05,
            verdict: "unit".to_string(),
        }
    }
}
