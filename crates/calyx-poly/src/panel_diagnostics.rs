//! Panel higher-order information diagnostics (issue #207).
//!
//! The Calyx handbook (§7.4, §13) and PRD `04_ASSOCIATION_ENGINE_NO_EMBEDDERS.md` §1.3 treat the
//! higher-order information *structure* of the panel as first-class Assay output. Per-slot MI (#50)
//! and the scalar sufficiency test (#79) say how much each slot knows about the outcome; they do
//! **not** say whether the panel is diverse or merely redundant, nor which slot combinations only
//! predict together. This module composes the real `calyx-assay` subsystem to produce, from grounded
//! observations:
//!
//! - **Total correlation** `TC(Φ) = Σ H(slotₖ) − H(Φ)` and the derived effective rank `n_eff`
//!   (`calyx_assay::total_correlation`), plus an independent effective-rank cross-check via
//!   `stable_rank` over the slot Pearson matrix.
//! - **Interaction information** for every slot triple, classified redundant / synergistic / unclear
//!   from whether its bootstrap CI straddles zero (`calyx_assay::interaction_information`); the
//!   synergistic triples ("only predict together") are surfaced ranked most-synergistic first.
//!
//! Every result carries estimator identity, CI, `n_samples`, provenance, and a [`TrustTag`] derived
//! from the anchors that grounded the observations (issue #209): a single proxy anchor, or a sample
//! count below the MI floor, makes the whole diagnostic Provisional — never a confident emission.
//! Fail closed on non-finite input, ragged samples, and estimator errors. No hand-set weights, no
//! passed-in numbers standing in for a computed diagnostic (doctrine #1 / handbook §12).

use std::collections::BTreeMap;

use calyx_assay::{
    IIResult, IISign, NeffReport, TCResult, TotalCorrelationConfig, TrustTag,
    interaction_information_with_config, stable_rank, total_correlation_with_config,
};
use calyx_core::{Anchor, Clock};
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};
use crate::grounding::rollup_trust;

/// Schema tag persisted with every diagnostics artifact.
pub const PANEL_DIAGNOSTICS_SCHEMA_VERSION: &str = "poly.panel_diagnostics.v1";
/// Artifact-kind tag persisted with every diagnostics row.
pub const PANEL_DIAGNOSTICS_ARTIFACT_KIND: &str = "poly_panel_diagnostics";
/// Estimator identity for the composed KSG-based diagnostics.
pub const ESTIMATOR_KSG: &str = "ksg";

/// The panel had zero slots.
pub const ERR_EMPTY_PANEL: &str = "CALYX_POLY_DIAG_EMPTY_PANEL";
/// A slot column had a different sample count than the panel, or an anchor count mismatch.
pub const ERR_RAGGED_PANEL: &str = "CALYX_POLY_DIAG_RAGGED_SAMPLES";
/// A slot column or key was non-finite / empty.
pub const ERR_NON_FINITE: &str = "CALYX_POLY_DIAG_NON_FINITE_SLOT";
/// A requested slot key was missing from an observation's scalar map.
pub const ERR_MISSING_SLOT: &str = "CALYX_POLY_DIAG_MISSING_SLOT";

/// Tunable diagnostics configuration. The bootstrap-resample count is a precision knob (higher = a
/// tighter CI, more compute); it is not a fallback. Production uses the engine defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PanelDiagnosticsConfig {
    /// KSG neighbor count and bootstrap resamples for TC and interaction information.
    pub tc: TotalCorrelationConfig,
}

/// Aligned per-slot scalar series assembled from grounded observations. Column `k` is slot `k`'s
/// value across every observation (length `n_samples`); `anchors[i]` is the anchor that grounded
/// observation `i`. All columns share the same length and every value is finite (validated at
/// construction — fail closed otherwise).
#[derive(Clone, Debug)]
pub struct PanelMatrix {
    slot_keys: Vec<String>,
    columns: Vec<Vec<f32>>,
    anchors: Vec<Anchor>,
}

impl PanelMatrix {
    /// Validates and builds a panel matrix from explicit columns. Fails closed on an empty panel,
    /// ragged columns, an anchor-count mismatch, or a non-finite value.
    pub fn new(
        slot_keys: Vec<String>,
        columns: Vec<Vec<f32>>,
        anchors: Vec<Anchor>,
    ) -> Result<Self> {
        if slot_keys.is_empty() || columns.is_empty() {
            return Err(PolyError::diagnostics(
                ERR_EMPTY_PANEL,
                "panel diagnostics require at least one slot",
            ));
        }
        if slot_keys.len() != columns.len() {
            return Err(PolyError::diagnostics(
                ERR_RAGGED_PANEL,
                format!(
                    "{} slot keys but {} columns",
                    slot_keys.len(),
                    columns.len()
                ),
            ));
        }
        let n = columns[0].len();
        if anchors.len() != n {
            return Err(PolyError::diagnostics(
                ERR_RAGGED_PANEL,
                format!("{} anchors but {n} samples per slot", anchors.len()),
            ));
        }
        for (idx, col) in columns.iter().enumerate() {
            if col.len() != n {
                return Err(PolyError::diagnostics(
                    ERR_RAGGED_PANEL,
                    format!(
                        "slot '{}' has {} samples, expected {n}",
                        slot_keys[idx],
                        col.len()
                    ),
                ));
            }
            if col.iter().any(|v| !v.is_finite()) {
                return Err(PolyError::diagnostics(
                    ERR_NON_FINITE,
                    format!("slot '{}' contains a non-finite value", slot_keys[idx]),
                ));
            }
        }
        Ok(Self {
            slot_keys,
            columns,
            anchors,
        })
    }

    /// Builds a panel matrix from `(scalars, grounding anchor)` observations, selecting `slot_keys`
    /// from each observation's verbatim scalar map. Every selected key must be present and finite in
    /// every observation — a gap fails closed (a fabricated zero would corrupt the estimate, #182).
    pub fn from_scalar_observations(
        slot_keys: &[String],
        observations: &[(BTreeMap<String, f64>, Anchor)],
    ) -> Result<Self> {
        if slot_keys.is_empty() {
            return Err(PolyError::diagnostics(
                ERR_EMPTY_PANEL,
                "panel diagnostics require at least one slot key",
            ));
        }
        let mut columns: Vec<Vec<f32>> =
            vec![Vec::with_capacity(observations.len()); slot_keys.len()];
        let mut anchors = Vec::with_capacity(observations.len());
        for (scalars, anchor) in observations {
            for (idx, key) in slot_keys.iter().enumerate() {
                let value = scalars.get(key).ok_or_else(|| {
                    PolyError::diagnostics(
                        ERR_MISSING_SLOT,
                        format!("observation is missing required slot '{key}'"),
                    )
                })?;
                if !value.is_finite() {
                    return Err(PolyError::diagnostics(
                        ERR_NON_FINITE,
                        format!("slot '{key}' has a non-finite value {value}"),
                    ));
                }
                columns[idx].push(*value as f32);
            }
            anchors.push(anchor.clone());
        }
        Self::new(slot_keys.to_vec(), columns, anchors)
    }

    /// Number of observations (samples per slot).
    pub fn n_samples(&self) -> usize {
        self.columns.first().map_or(0, Vec::len)
    }

    /// Ordered slot keys.
    pub fn slot_keys(&self) -> &[String] {
        &self.slot_keys
    }

    /// Per-slot value columns (column `k` is slot `k`'s series across observations).
    pub fn columns(&self) -> &[Vec<f32>] {
        &self.columns
    }

    /// The grounding anchor for each observation (length `n_samples`).
    pub fn anchors(&self) -> &[Anchor] {
        &self.anchors
    }
}

/// One slot-triple interaction-information diagnostic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TripleDiagnostic {
    /// The three slot keys, in panel order.
    pub slots: [String; 3],
    /// The interaction-information estimate (bits, sign, CI, trust) from `calyx-assay`.
    pub ii: IIResult,
}

/// The persisted panel higher-order diagnostics record — the FSV source of truth on disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelDiagnostics {
    /// Schema tag.
    pub schema_version: String,
    /// Artifact-kind tag.
    pub artifact_kind: String,
    /// The domain the panel belongs to.
    pub domain: String,
    /// The panel version the observations were measured with.
    pub panel_version: u32,
    /// Estimator identity for every diagnostic in this record.
    pub estimator: String,
    /// Ordered slot keys.
    pub slot_keys: Vec<String>,
    /// Observations (samples per slot).
    pub n_samples: usize,
    /// Total correlation + engine-derived effective rank `n_eff`.
    pub total_correlation: TCResult,
    /// Independent effective-rank cross-check via `stable_rank` over the Pearson matrix.
    pub effective_rank: NeffReport,
    /// Number of slot triples evaluated (`C(N, 3)`).
    pub triples_evaluated: usize,
    /// Synergistic triples ("only predict together"), most-synergistic (most-negative II) first.
    pub synergistic_triples: Vec<TripleDiagnostic>,
    /// Count of triples classified redundant.
    pub redundant_triple_count: usize,
    /// Count of triples whose CI straddled zero (unclear / below floor).
    pub unclear_triple_count: usize,
    /// Record-level trust: Trusted only if every grounding anchor is a resolved outcome **and** the
    /// diagnostic cleared the sample floor; a proxy anchor or a below-floor estimate → Provisional.
    pub trust: TrustTag,
    /// True if any diagnostic fell below its MI sample floor (honesty gate).
    pub provisional: bool,
    /// blake3 of the canonical diagnostic payload, for reproducible provenance.
    pub provenance_hash: String,
    /// Wall-clock (from the injected [`Clock`]) at computation.
    pub computed_at: u64,
}

/// Computes the panel higher-order information diagnostics over `matrix`, grounded and trust-tagged.
pub fn compute_panel_diagnostics(
    domain: &str,
    panel_version: u32,
    matrix: &PanelMatrix,
    clock: &dyn Clock,
    config: &PanelDiagnosticsConfig,
) -> Result<PanelDiagnostics> {
    let slot_keys = matrix.slot_keys.clone();
    let n_samples = matrix.n_samples();

    let tc = total_correlation_with_config(&matrix.columns, clock, &config.tc)?;
    let effective_rank = stable_rank(&pearson_matrix(&matrix.columns));

    let mut synergistic = Vec::new();
    let mut redundant = 0usize;
    let mut unclear = 0usize;
    let mut triples_evaluated = 0usize;
    let mut any_provisional = tc.provisional;
    let n = slot_keys.len();
    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                triples_evaluated += 1;
                let ii = interaction_information_with_config(
                    &matrix.columns[i],
                    &matrix.columns[j],
                    &matrix.columns[k],
                    clock,
                    &config.tc,
                )?;
                if ii.provisional {
                    any_provisional = true;
                }
                match ii.sign {
                    IISign::Synergistic => synergistic.push(TripleDiagnostic {
                        slots: [
                            slot_keys[i].clone(),
                            slot_keys[j].clone(),
                            slot_keys[k].clone(),
                        ],
                        ii,
                    }),
                    IISign::Redundant => redundant += 1,
                    IISign::Unclear => unclear += 1,
                }
            }
        }
    }
    // Most-synergistic first: interaction information is most negative for the strongest synergy.
    synergistic.sort_by(|a, b| a.ii.ii.total_cmp(&b.ii.ii));

    let anchor_trust = rollup_trust(&matrix.anchors)?;
    let trust = if any_provisional || anchor_trust != TrustTag::Trusted {
        TrustTag::Provisional
    } else {
        TrustTag::Trusted
    };

    let provenance_hash = provenance_hash(
        domain,
        panel_version,
        &slot_keys,
        n_samples,
        &tc,
        &effective_rank,
        &synergistic,
    );

    Ok(PanelDiagnostics {
        schema_version: PANEL_DIAGNOSTICS_SCHEMA_VERSION.to_string(),
        artifact_kind: PANEL_DIAGNOSTICS_ARTIFACT_KIND.to_string(),
        domain: domain.to_string(),
        panel_version,
        estimator: ESTIMATOR_KSG.to_string(),
        slot_keys,
        n_samples,
        total_correlation: tc,
        effective_rank,
        triples_evaluated,
        synergistic_triples: synergistic,
        redundant_triple_count: redundant,
        unclear_triple_count: unclear,
        trust,
        provisional: any_provisional,
        provenance_hash,
        computed_at: clock.now(),
    })
}

/// Persists a diagnostics record as JSON under `dir` and returns its path (the FSV source of truth).
/// The file name is stable per `(domain, panel_version)` so the latest diagnostic is read back
/// deterministically.
pub fn write_panel_diagnostics(
    dir: &std::path::Path,
    record: &PanelDiagnostics,
) -> Result<std::path::PathBuf> {
    let file_name = format!(
        "panel_diagnostics_{}_v{}.json",
        sanitize(&record.domain),
        record.panel_version
    );
    crate::diagnostics_store::write_json(dir, &file_name, record)
}

/// Reads a persisted diagnostics record back from disk, failing closed if missing or off-schema.
pub fn read_panel_diagnostics(path: &std::path::Path) -> Result<PanelDiagnostics> {
    crate::diagnostics_store::read_json(path)
}

fn sanitize(domain: &str) -> String {
    domain
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Pearson correlation matrix over the slot columns. A degenerate (zero-variance) column yields a
/// zero row/column except its unit diagonal, so `stable_rank` still reports a finite effective rank.
fn pearson_matrix(columns: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let n = columns.len();
    let stats: Vec<(f64, f64)> = columns.iter().map(|c| mean_std(c)).collect();
    let mut matrix = vec![vec![0.0f32; n]; n];
    for i in 0..n {
        matrix[i][i] = 1.0;
        for j in (i + 1)..n {
            let corr = pearson(&columns[i], &columns[j], stats[i], stats[j]);
            matrix[i][j] = corr;
            matrix[j][i] = corr;
        }
    }
    matrix
}

fn mean_std(col: &[f32]) -> (f64, f64) {
    let n = col.len() as f64;
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let mean = col.iter().map(|v| *v as f64).sum::<f64>() / n;
    let var = col.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / n;
    (mean, var.sqrt())
}

fn pearson(a: &[f32], b: &[f32], (mean_a, std_a): (f64, f64), (mean_b, std_b): (f64, f64)) -> f32 {
    if std_a <= f64::EPSILON || std_b <= f64::EPSILON {
        return 0.0;
    }
    let n = a.len() as f64;
    let cov = a
        .iter()
        .zip(b)
        .map(|(x, y)| (*x as f64 - mean_a) * (*y as f64 - mean_b))
        .sum::<f64>()
        / n;
    (cov / (std_a * std_b)).clamp(-1.0, 1.0) as f32
}

fn provenance_hash(
    domain: &str,
    panel_version: u32,
    slot_keys: &[String],
    n_samples: usize,
    tc: &TCResult,
    effective_rank: &NeffReport,
    synergistic: &[TripleDiagnostic],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&panel_version.to_le_bytes());
    hasher.update(&(n_samples as u64).to_le_bytes());
    for key in slot_keys {
        hasher.update(key.as_bytes());
        hasher.update(&[0]);
    }
    hasher.update(&tc.tc.to_le_bytes());
    hasher.update(&tc.n_eff.to_le_bytes());
    hasher.update(&effective_rank.n_eff.to_le_bytes());
    for triple in synergistic {
        hasher.update(&triple.ii.ii.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{AnchorKind, AnchorValue};

    fn resolved(i: usize) -> Anchor {
        Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(i.is_multiple_of(2)),
            source: "uma:test".to_string(),
            observed_at: i as u64,
            confidence: 1.0,
        }
    }

    #[test]
    fn matrix_fails_closed_on_ragged_and_non_finite() {
        let anchors = vec![resolved(0), resolved(1)];
        let ragged = PanelMatrix::new(
            vec!["a".into(), "b".into()],
            vec![vec![1.0, 2.0], vec![1.0]],
            anchors.clone(),
        );
        assert_eq!(ragged.unwrap_err().code(), ERR_RAGGED_PANEL);

        let nonfinite = PanelMatrix::new(vec!["a".into()], vec![vec![1.0, f32::NAN]], anchors);
        assert_eq!(nonfinite.unwrap_err().code(), ERR_NON_FINITE);
    }

    #[test]
    fn missing_slot_fails_closed() {
        let mut scalars = BTreeMap::new();
        scalars.insert("a".to_string(), 1.0);
        let obs = vec![(scalars, resolved(0))];
        let err =
            PanelMatrix::from_scalar_observations(&["a".into(), "b".into()], &obs).unwrap_err();
        assert_eq!(err.code(), ERR_MISSING_SLOT);
    }

    #[test]
    fn pearson_identical_columns_is_one() {
        let a = vec![1.0f32, 2.0, 3.0, 4.0];
        let b = a.clone();
        let sa = mean_std(&a);
        let sb = mean_std(&b);
        assert!((pearson(&a, &b, sa, sb) - 1.0).abs() < 1e-5);
    }
}
