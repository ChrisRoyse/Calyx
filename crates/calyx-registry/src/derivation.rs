//! Versioning for the lens-identity derivation function.
//!
//! `LensSpec::lens_id()` and `FrozenLensContract::lens_id()` are pure functions
//! of a set of declared inputs. Those inputs legitimately evolve: a policy
//! string is corrected, an enum gains a variant, a manifest hash gains a part.
//! When they do, every already-frozen lens is re-identified with **no artifact
//! change and no tamper evidence**, and a persisted corpus fails its contract
//! audit with an opaque hash mismatch that reads like corruption (#1943, #1952).
//!
//! This module makes that a *typed, explicable migration event*. It does not
//! weaken any guard: a mismatch still fails closed. It only replaces "two
//! hashes differ" with "derivation version N→M, this specific policy changed,
//! here is whether your stored measurements survive it".
//!
//! # Why the version is not itself a hash input
//!
//! #1943 proposed stamping the version *into* the identity inputs. That is
//! self-defeating: a field inside the hash cannot explain why the hash moved,
//! and bumping it would re-identify every lens a second time. The stamp is
//! therefore a **sidecar** on the persisted record — written next to the
//! identity, never mixed into it. Existing identities are unchanged by this
//! module; adding it orphans nothing.
//!
//! # The distinction that decides whether migration is honest
//!
//! Not all derivation changes are equal, and conflating them is how a corpus
//! gets silently relabelled:
//!
//! * [`DerivationImpact::IdentityOnly`] — the derivation function changed but
//!   the bytes measured and the meaning assigned to them did not. Stored
//!   vectors are still the *same measurement*; re-stamping them under the new
//!   identity is honest.
//! * [`DerivationImpact::MeasurementChanging`] — the input bytes or their
//!   meaning changed. Stored vectors are a *different measurement*. They must
//!   be re-measured. Re-attestation is forbidden and this module never offers
//!   it.

use std::fmt;

use calyx_core::CalyxError;

#[cfg(test)]
mod tests;

/// Derivation version implemented by this binary.
///
/// Bump this in the same commit as any change to an input of
/// `LensSpec::lens_id()` or `FrozenLensContract::lens_id()`, and append a
/// matching [`DerivationStep`]. `golden_identity` in this module's tests fails
/// loudly when a derivation changes without a bump.
pub const LENS_DERIVATION_VERSION: u32 = 4;

/// First version whose records carry a derivation stamp.
///
/// A persisted record with no stamp was written before this mechanism existed
/// and is treated as "at most `FIRST_STAMPED_VERSION - 1`".
pub const FIRST_STAMPED_VERSION: u32 = LENS_DERIVATION_VERSION;

/// Module-local code: a persisted lens identity was derived by an older
/// derivation function than this binary implements.
pub const LENS_DERIVATION_STALE: &str = "CALYX_LENS_DERIVATION_STALE";

/// Module-local code: an immutable panel template's lens identities were
/// derived by an older derivation function than this binary implements.
pub const PANEL_TEMPLATE_DERIVATION_STALE: &str = "CALYX_PANEL_TEMPLATE_DERIVATION_STALE";

/// Whether a derivation step preserved the measurement it identifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DerivationImpact {
    /// The identity function moved; the measurement did not. Stored vectors
    /// remain valid and may be honestly re-stamped under the new identity.
    IdentityOnly,
    /// The measured bytes or their meaning moved. Stored vectors are a
    /// different measurement and MUST be re-measured. Never re-stamp.
    MeasurementChanging,
}

impl DerivationImpact {
    /// Returns the stable wire token for this impact class.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IdentityOnly => "identity-only",
            Self::MeasurementChanging => "measurement-changing",
        }
    }

    /// Returns whether stored measurements survive this class of change.
    pub const fn is_re_attestable(self) -> bool {
        matches!(self, Self::IdentityOnly)
    }

    /// Combines two impacts, taking the strictest.
    const fn worst(self, other: Self) -> Self {
        match (self, other) {
            (Self::MeasurementChanging, _) | (_, Self::MeasurementChanging) => {
                Self::MeasurementChanging
            }
            _ => Self::IdentityOnly,
        }
    }
}

impl fmt::Display for DerivationImpact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One recorded change to the lens-identity derivation function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DerivationStep {
    /// Version this step moves away from.
    pub from: u32,
    /// Version this step moves to.
    pub to: u32,
    /// Abbreviated commit that made the change.
    pub commit: &'static str,
    /// Issue that motivated or recorded the change.
    pub issue: &'static str,
    /// Whether stored measurements survive the change.
    pub impact: DerivationImpact,
    /// Exactly what moved, in operator-readable terms.
    pub summary: &'static str,
    /// `LensRuntime` variant names whose identity this step can move. A lens on
    /// a runtime outside this list is not re-identified by this step.
    pub affects: &'static [&'static str],
    /// Bare contract/identity field names this step can move.
    ///
    /// This is what makes a derivation explanation *checkable*: a drift is only
    /// attributed to this step when every field that actually moved appears
    /// here. A lens that also drifted in, say, `weights_sha256` is not
    /// explained by a step that cannot move `weights_sha256`, and stays
    /// tamper-suspect.
    pub moves_fields: &'static [&'static str],
}

impl DerivationStep {
    /// Returns whether this step can move the identity of the named runtime.
    pub fn affects_runtime(&self, runtime: &str) -> bool {
        self.affects.contains(&runtime)
    }

    /// Returns whether this step can move the named field.
    ///
    /// Accepts either a bare field name or a prefixed audit field such as
    /// `runtime.corpus_hash`; only the segment after the last `.` is matched.
    pub fn moves_field(&self, field: &str) -> bool {
        let bare = field.rsplit('.').next().unwrap_or(field);
        self.moves_fields.contains(&bare)
    }

    /// Renders this step as a single operator-readable delta line.
    pub fn describe(&self) -> String {
        format!(
            "v{}->v{} [{}] {} ({}, {})",
            self.from, self.to, self.impact, self.summary, self.commit, self.issue
        )
    }
}

/// Every recorded change to the lens-identity derivation, oldest first.
///
/// Invariants (enforced by tests): `from + 1 == to` for every step, steps are
/// contiguous starting at 1, and the last `to` equals [`LENS_DERIVATION_VERSION`].
pub const DERIVATION_CHANGELOG: &[DerivationStep] = &[
    DerivationStep {
        from: 1,
        to: 2,
        commit: "9b591c3e",
        issue: "#1943",
        impact: DerivationImpact::IdentityOnly,
        summary: "lens_spec_from_manifest appended endpoint_hash to the \
                  length-delimited corpus_hash content_address; because each part is \
                  hashed as len||bytes, even an empty appended part moved every \
                  manifest-derived corpus_hash. The measured bytes were unchanged",
        affects: &[
            "TeiHttp",
            "CandleLocal",
            "Onnx",
            "OnnxColbert",
            "FastembedSparse",
            "FastembedBgem3",
            "FastembedReranker",
            "FastembedQwen3",
            "StaticLookup",
            "MultimodalAdapter",
        ],
        moves_fields: &["corpus_hash", "lens_id"],
    },
    DerivationStep {
        from: 2,
        to: 3,
        commit: "1475a8ce",
        issue: "#1943",
        impact: DerivationImpact::IdentityOnly,
        summary: "LensRuntime gained TeiHttpSegmented and the slot-vector typing gained \
                  SegmentedMulti; identity embeds format!(\"{runtime:?}\") and \
                  \"shape={output:?}\", so the Debug surface of both enums entered the \
                  hash. No emitted vector changed",
        affects: &[
            "TeiHttp",
            "TeiHttpSegmented",
            "Algorithmic",
            "CandleLocal",
            "Onnx",
            "OnnxColbert",
            "FastembedSparse",
            "FastembedBgem3",
            "FastembedReranker",
            "FastembedQwen3",
            "StaticLookup",
            "MultimodalAdapter",
            "ExternalCmd",
        ],
        moves_fields: &["lens_id"],
    },
    DerivationStep {
        from: 3,
        to: 4,
        commit: "19dcc189",
        issue: "#1902/#1952",
        impact: DerivationImpact::MeasurementChanging,
        summary: "TEI HTTP request policy moved from the implicit \"tei-http-runtime\" \
                  corpus_hash to \"tei-http-request-v2;truncate=false\". The client \
                  previously sent truncate=true, so any input over the router token limit \
                  was head-truncated and measured as a prefix; it is now measured whole. \
                  Vectors stored before this step are a measurement of different bytes \
                  wherever the source exceeded the limit",
        affects: &["TeiHttp"],
        moves_fields: &["corpus_hash", "lens_id"],
    },
];

/// Outcome of comparing a persisted derivation stamp against this binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DerivationVerdict {
    /// The record was derived by this binary's derivation function.
    Current,
    /// The record predates derivation stamping; its exact version is unknown
    /// and every step in the changelog is a candidate explanation.
    Unstamped { steps: Vec<DerivationStep> },
    /// The record was derived by a known older derivation function.
    Stale { steps: Vec<DerivationStep> },
    /// The record was written by a newer binary than this one.
    Ahead { persisted: u32 },
}

impl DerivationVerdict {
    /// Returns the steps separating the record from this binary, if any.
    pub fn steps(&self) -> &[DerivationStep] {
        match self {
            Self::Unstamped { steps } | Self::Stale { steps } => steps,
            Self::Current | Self::Ahead { .. } => &[],
        }
    }

    /// Returns whether this verdict represents agreement with this binary.
    pub const fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }

    /// Returns the strictest impact across the separating steps.
    ///
    /// `None` means there are no separating steps to classify.
    pub fn impact(&self) -> Option<DerivationImpact> {
        self.steps()
            .iter()
            .map(|step| step.impact)
            .reduce(DerivationImpact::worst)
    }

    /// Returns whether stored measurements survive the whole span.
    ///
    /// A span with no steps is not automatically re-attestable: `Ahead` means
    /// this binary cannot reason about the record at all.
    pub fn is_re_attestable(&self) -> bool {
        match self {
            Self::Current => true,
            Self::Ahead { .. } => false,
            Self::Unstamped { .. } | Self::Stale { .. } => self
                .impact()
                .is_some_and(DerivationImpact::is_re_attestable),
        }
    }

    /// Narrows the verdict to the steps that can move the named runtime.
    pub fn for_runtime(&self, runtime: &str) -> Self {
        let filter = |steps: &Vec<DerivationStep>| {
            steps
                .iter()
                .filter(|step| step.affects_runtime(runtime))
                .copied()
                .collect::<Vec<_>>()
        };
        match self {
            Self::Unstamped { steps } => Self::Unstamped {
                steps: filter(steps),
            },
            Self::Stale { steps } => Self::Stale {
                steps: filter(steps),
            },
            other => other.clone(),
        }
    }

    /// Renders the separating steps as operator-readable delta lines.
    pub fn describe_steps(&self) -> String {
        let steps = self.steps();
        if steps.is_empty() {
            return "<no recorded derivation delta>".to_string();
        }
        steps
            .iter()
            .map(DerivationStep::describe)
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Renders the version transition this verdict represents.
    pub fn describe_transition(&self) -> String {
        match self {
            Self::Current => format!("v{LENS_DERIVATION_VERSION} (current)"),
            Self::Unstamped { .. } => {
                format!("unstamped (pre-v{FIRST_STAMPED_VERSION}) -> v{LENS_DERIVATION_VERSION}")
            }
            Self::Stale { steps } => {
                let from = steps
                    .first()
                    .map_or(LENS_DERIVATION_VERSION, |step| step.from);
                format!("v{from} -> v{LENS_DERIVATION_VERSION}")
            }
            Self::Ahead { persisted } => {
                format!("v{persisted} (record) -> v{LENS_DERIVATION_VERSION} (binary)")
            }
        }
    }
}

/// Classifies a persisted derivation stamp against this binary's derivation.
pub fn classify(persisted: Option<u32>) -> DerivationVerdict {
    match persisted {
        None => DerivationVerdict::Unstamped {
            steps: DERIVATION_CHANGELOG.to_vec(),
        },
        Some(version) if version == LENS_DERIVATION_VERSION => DerivationVerdict::Current,
        Some(version) if version > LENS_DERIVATION_VERSION => {
            DerivationVerdict::Ahead { persisted: version }
        }
        Some(version) => DerivationVerdict::Stale {
            steps: DERIVATION_CHANGELOG
                .iter()
                .filter(|step| step.from >= version)
                .copied()
                .collect(),
        },
    }
}

/// Builds the typed stale-derivation error for a persisted registry lens.
pub fn lens_derivation_stale(verdict: &DerivationVerdict, detail: impl fmt::Display) -> CalyxError {
    CalyxError {
        code: LENS_DERIVATION_STALE,
        message: format!(
            "lens identity derivation is stale: contract version {}; \
             stored measurements are {}; deltas: {}; {detail}",
            verdict.describe_transition(),
            re_attestation_phrase(verdict),
            verdict.describe_steps(),
        ),
        remediation: remediation_for(verdict),
    }
}

/// Builds the typed stale-derivation error for an immutable panel template.
pub fn panel_template_derivation_stale(
    verdict: &DerivationVerdict,
    detail: impl fmt::Display,
) -> CalyxError {
    CalyxError {
        code: PANEL_TEMPLATE_DERIVATION_STALE,
        message: format!(
            "panel template lens identity derivation is stale: contract version {}; \
             stored measurements are {}; deltas: {}; {detail}",
            verdict.describe_transition(),
            re_attestation_phrase(verdict),
            verdict.describe_steps(),
        ),
        remediation: remediation_for(verdict),
    }
}

fn re_attestation_phrase(verdict: &DerivationVerdict) -> &'static str {
    match verdict.impact() {
        Some(DerivationImpact::IdentityOnly) => {
            "unchanged by every delta and may be honestly re-stamped"
        }
        Some(DerivationImpact::MeasurementChanging) => {
            "a measurement of different bytes and MUST be re-measured"
        }
        None => "not classifiable by this binary",
    }
}

fn remediation_for(verdict: &DerivationVerdict) -> &'static str {
    match verdict {
        DerivationVerdict::Ahead { .. } => {
            "this record was written by a newer binary; upgrade before opening it"
        }
        _ => match verdict.impact() {
            Some(DerivationImpact::IdentityOnly) => {
                "run `calyx panel registry-repair --vault <vault> --slot <slot>` to re-stamp the \
                 identity; the stored vectors are the same measurement and are preserved"
            }
            Some(DerivationImpact::MeasurementChanging) => {
                "re-measure the corpus at this binary: the stored vectors measure different bytes \
                 and must not be re-stamped; re-ingest into a fresh vault"
            }
            None => "no recorded derivation delta explains this record; treat as tamper-suspect",
        },
    }
}
