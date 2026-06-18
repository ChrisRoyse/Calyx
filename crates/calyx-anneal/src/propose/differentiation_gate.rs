use calyx_assay::contract::MIN_RELIABILITY_SEEDS;
use calyx_assay::{
    CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED, CALYX_ASSAY_UNRESOLVED, PanelResourceBudget,
    ResourceAwareAdmissionDecision, ResourceUsage, admit_lens_with_resources,
};
use calyx_core::{
    CalyxError, Clock, Constellation, LensCost, LensId, Placement, Result, SystemClock, Ts,
};
use calyx_registry::{CapabilityCard, CapabilitySignalKind};
use serde::{Deserialize, Serialize};

use crate::ShadowRevertReason;

use super::candidate_synth::CandidateLens;
use super::deficit_localize::CALYX_ASSAY_INVALID_METRIC;

pub const DIFFERENTIATION_MIN_BITS: f64 = 0.05;
pub const DIFFERENTIATION_MAX_CORR: f64 = 0.6;
pub const PROFILE_TIMEOUT_MS: Ts = 30_000;
pub const CALYX_REGISTRY_PROFILE_TIMEOUT: &str = "CALYX_REGISTRY_PROFILE_TIMEOUT";

const METRIC_EPSILON: f64 = 1e-12;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GateOutcome {
    Admitted {
        bits: f64,
        max_corr: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource: Option<ResourceAwareAdmissionDecision>,
    },
    Rejected {
        reason: RejectReason,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum RejectReason {
    InsufficientBits {
        bits: f64,
        threshold: f64,
    },
    NonLearnedSignal {
        signal_kind: CapabilitySignalKind,
        required: CapabilitySignalKind,
    },
    TooCorrelated {
        corr: f64,
        offending_lens: LensId,
        threshold: f64,
    },
    ProfileTimeout,
    ResourceBudgetExceeded {
        vram_mb: f64,
        ram_mb: f64,
        ms_per_input: f64,
        max_vram_mb: f64,
        max_ram_mb: f64,
        max_ms_per_input: f64,
    },
    HotAddFailed {
        code: String,
    },
    SubstrateReverted {
        shadow_reason: ShadowRevertReason,
    },
    NoSufficiencyGain {
        before: f64,
        after: f64,
    },
}

pub trait LensProfiler {
    fn profile(
        &self,
        candidate: &CandidateLens,
        corpus_sample: &[Constellation],
    ) -> Result<CapabilityCard>;
}

pub trait PairNMI {
    fn lens_embeddings(
        &self,
        lens: &LensId,
        corpus_sample: &[Constellation],
    ) -> Result<Vec<Vec<f32>>>;

    fn nmi(&self, lens_a: &LensId, lens_b_embeddings: &[Vec<f32>]) -> Result<f64>;
}

pub struct DifferentiationGate<'a> {
    clock: &'a dyn Clock,
    profile_timeout_ms: Ts,
    resource_budget: Option<PanelResourceBudget>,
}

impl<'a> DifferentiationGate<'a> {
    pub fn new(clock: &'a dyn Clock) -> Self {
        Self {
            clock,
            profile_timeout_ms: PROFILE_TIMEOUT_MS,
            resource_budget: None,
        }
    }

    pub fn with_resource_budget(mut self, budget: PanelResourceBudget) -> Self {
        self.resource_budget = Some(budget);
        self
    }

    pub fn gate(
        &self,
        candidate: &CandidateLens,
        panel: &[LensId],
        profiler: &dyn LensProfiler,
        nmi: &dyn PairNMI,
        corpus: &[Constellation],
    ) -> Result<GateOutcome> {
        let started = self.clock.now();
        let card = match profiler.profile(candidate, corpus) {
            Ok(card) => card,
            Err(error) if error.code == CALYX_REGISTRY_PROFILE_TIMEOUT => {
                return Ok(GateOutcome::Rejected {
                    reason: RejectReason::ProfileTimeout,
                });
            }
            Err(error) => return Err(error),
        };
        if self.clock.now().saturating_sub(started) > self.profile_timeout_ms {
            return Ok(GateOutcome::Rejected {
                reason: RejectReason::ProfileTimeout,
            });
        }

        let bits = profile_bits(&card)?;
        if bits < DIFFERENTIATION_MIN_BITS {
            return Ok(GateOutcome::Rejected {
                reason: RejectReason::InsufficientBits {
                    bits,
                    threshold: DIFFERENTIATION_MIN_BITS,
                },
            });
        }
        if !card.signal_kind.is_learned_encoder() {
            return Ok(GateOutcome::Rejected {
                reason: RejectReason::NonLearnedSignal {
                    signal_kind: card.signal_kind,
                    required: CapabilitySignalKind::LearnedEncoder,
                },
            });
        }

        let mut max_corr = 0.0;
        let mut offending_lens = None;
        for lens in panel {
            let embeddings = nmi.lens_embeddings(lens, corpus)?;
            let corr = validate_corr(nmi.nmi(&card.lens_id, &embeddings)?, lens)?;
            if corr > max_corr {
                max_corr = corr;
                offending_lens = Some(*lens);
            }
        }
        if max_corr > DIFFERENTIATION_MAX_CORR {
            return Ok(GateOutcome::Rejected {
                reason: RejectReason::TooCorrelated {
                    corr: max_corr,
                    offending_lens: offending_lens.expect("panel lens recorded with max corr"),
                    threshold: DIFFERENTIATION_MAX_CORR,
                },
            });
        }

        let resource = match self.resource_budget {
            Some(budget) => match admit_resource(bits, max_corr, &card, budget) {
                Ok(decision) => Some(decision),
                Err(error) if error.code == CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED => {
                    return Ok(GateOutcome::Rejected {
                        reason: resource_budget_exceeded(&card, budget),
                    });
                }
                Err(error) => return Err(error),
            },
            None => None,
        };

        Ok(GateOutcome::Admitted {
            bits,
            max_corr,
            resource,
        })
    }
}

pub fn gate(
    candidate: &CandidateLens,
    panel: &[LensId],
    profiler: &dyn LensProfiler,
    nmi: &dyn PairNMI,
    corpus: &[Constellation],
) -> Result<GateOutcome> {
    let clock = SystemClock;
    DifferentiationGate::new(&clock).gate(candidate, panel, profiler, nmi, corpus)
}

pub fn describe_gate_outcome(outcome: &GateOutcome) -> String {
    match outcome {
        GateOutcome::Admitted {
            bits,
            max_corr,
            resource,
        } => {
            format!(
                "LensAdmitted bits={bits:.4} threshold={DIFFERENTIATION_MIN_BITS:.4} max_corr={max_corr:.4} threshold={DIFFERENTIATION_MAX_CORR:.4}"
            ) + &resource
                .as_ref()
                .map(|decision| {
                    format!(
                        " vram_mb={:.3} ram_mb={:.3} ms_per_input={:.3}",
                        decision.usage.vram_mb, decision.usage.ram_mb, decision.usage.ms_per_input
                    )
                })
                .unwrap_or_default()
        }
        GateOutcome::Rejected {
            reason: RejectReason::InsufficientBits { bits, threshold },
        } => format!("LensRejected insufficient_bits bits={bits:.4} threshold={threshold:.4}"),
        GateOutcome::Rejected {
            reason:
                RejectReason::NonLearnedSignal {
                    signal_kind,
                    required,
                },
        } => format!(
            "LensRejected non_learned_signal signal_kind={} required={}",
            signal_kind.as_str(),
            required.as_str()
        ),
        GateOutcome::Rejected {
            reason:
                RejectReason::TooCorrelated {
                    corr,
                    offending_lens,
                    threshold,
                },
        } => format!(
            "LensRejected too_correlated corr={corr:.4} offending_lens={offending_lens} threshold={threshold:.4}"
        ),
        GateOutcome::Rejected {
            reason: RejectReason::ProfileTimeout,
        } => format!(
            "LensRejected profile_timeout threshold_ms={}",
            PROFILE_TIMEOUT_MS
        ),
        GateOutcome::Rejected {
            reason:
                RejectReason::ResourceBudgetExceeded {
                    vram_mb,
                    ram_mb,
                    ms_per_input,
                    max_vram_mb,
                    max_ram_mb,
                    max_ms_per_input,
                },
        } => format!(
            "LensRejected resource_budget_exceeded usage=({vram_mb:.3}MiB,{ram_mb:.3}MiB,{ms_per_input:.3}ms) budget=({max_vram_mb:.3}MiB,{max_ram_mb:.3}MiB,{max_ms_per_input:.3}ms)"
        ),
        GateOutcome::Rejected {
            reason: RejectReason::HotAddFailed { code },
        } => format!("LensRejected hot_add_failed code={code}"),
        GateOutcome::Rejected {
            reason: RejectReason::SubstrateReverted { shadow_reason },
        } => format!("LensRejected substrate_reverted reason={shadow_reason:?}"),
        GateOutcome::Rejected {
            reason: RejectReason::NoSufficiencyGain { before, after },
        } => format!("LensRejected no_sufficiency_gain sufficiency={before:.6}->{after:.6}"),
    }
}

fn profile_bits(card: &CapabilityCard) -> Result<f64> {
    let bits = card
        .signal
        .map(f64::from)
        .ok_or_else(|| invalid_metric("capability card missing assay signal bits_per_anchor"))?;
    if let Some(reliability) = &card.signal_reliability {
        for (name, value) in [
            ("ci_low", reliability.ci_low),
            ("ci_high", reliability.ci_high),
            ("seed_sigma", reliability.seed_sigma),
        ] {
            validate_nonnegative(name, f64::from(value))?;
        }
        if reliability.seed_count < MIN_RELIABILITY_SEEDS || reliability.unresolved {
            return Err(assay_unresolved(format!(
                "capability signal unresolved: bits={bits:.6} ci=[{:.6},{:.6}] seed_sigma={:.6} seed_count={}",
                reliability.ci_low,
                reliability.ci_high,
                reliability.seed_sigma,
                reliability.seed_count
            )));
        }
        return validate_nonnegative("bits_lower_ci", f64::from(reliability.ci_low));
    }
    validate_nonnegative("bits_per_anchor", bits)
}

fn validate_corr(value: f64, lens: &LensId) -> Result<f64> {
    let value = validate_nonnegative("nmi", value)?;
    if value > 1.0 + METRIC_EPSILON {
        return Err(invalid_metric(format!(
            "NMI for lens {lens} must be <= 1.0, got {value}"
        )));
    }
    Ok(value.min(1.0))
}

fn admit_resource(
    bits: f64,
    max_corr: f64,
    card: &CapabilityCard,
    budget: PanelResourceBudget,
) -> Result<ResourceAwareAdmissionDecision> {
    let cost: LensCost = card.cost.into();
    admit_lens_with_resources(
        bits as f32,
        max_corr as f32,
        cost,
        placement_for_cost(cost),
        budget,
    )
}

fn resource_budget_exceeded(card: &CapabilityCard, budget: PanelResourceBudget) -> RejectReason {
    let usage = ResourceUsage::from_lens_cost(card.cost.into());
    RejectReason::ResourceBudgetExceeded {
        vram_mb: f64::from(usage.vram_mb),
        ram_mb: f64::from(usage.ram_mb),
        ms_per_input: f64::from(usage.ms_per_input),
        max_vram_mb: f64::from(budget.max_vram_mb),
        max_ram_mb: f64::from(budget.max_ram_mb),
        max_ms_per_input: f64::from(budget.max_ms_per_input),
    }
}

fn placement_for_cost(cost: LensCost) -> Placement {
    if cost.vram_bytes == 0 {
        Placement::Cpu
    } else {
        Placement::Gpu
    }
}

fn validate_nonnegative(name: &'static str, value: f64) -> Result<f64> {
    if !value.is_finite() || value < -METRIC_EPSILON {
        return Err(invalid_metric(format!(
            "{name} must be finite and non-negative, got {value}"
        )));
    }
    Ok(if value.abs() <= METRIC_EPSILON {
        0.0
    } else {
        value
    })
}

fn invalid_metric(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_INVALID_METRIC,
        message: message.into(),
        remediation: "re-measure candidate capability and pair NMI before gating a lens",
    }
}

fn assay_unresolved(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_UNRESOLVED,
        message: message.into(),
        remediation: "collect more grouped anchors and re-run multi-seed Assay measurement",
    }
}
