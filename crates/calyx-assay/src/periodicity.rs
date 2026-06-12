//! Periodicity detection on irregularly sampled series (PRD `26 §4`, PH52).
//!
//! Implements the generalised (floating-mean) Lomb-Scargle periodogram of
//! Zechmeister & Kürster 2009 (A&A 496, 577, eqs. 4-15) — VanderPlas 2018
//! (ApJS 236, 16) recommends the floating-mean form unconditionally — plus a
//! slotted autocorrelation for irregular samples as an independent cross-check.
//! Peak significance is a seeded permutation false-alarm probability (never an
//! analytic approximation), so every reported period carries an honest FAP.
//! All randomness is ChaCha8 seeded; results are bit-deterministic.

use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use calyx_core::{Anchor, CalyxError, Result};

use crate::estimate::{TrustTag, trust_for_anchor};

/// Minimum samples for any periodicity estimate (fail-closed below).
pub const MIN_PERIODICITY_SAMPLES: usize = 8;
/// Default frequency-grid oversampling factor `n_o` (VanderPlas §7.1: 5-10).
pub const DEFAULT_PERIODOGRAM_OVERSAMPLE: f64 = 10.0;
/// Default permutation count for the false-alarm probability (min p = 1/(P+1)).
pub const DEFAULT_FAP_PERMUTATIONS: usize = 100;
/// Default deterministic seed for FAP permutations.
pub const DEFAULT_PERIODICITY_SEED: u64 = 0;
/// Default number of reported peaks (PRD 26 §4: multiple overlapping periods).
pub const DEFAULT_MAX_PEAKS: usize = 3;
/// FAP at or below which a peak counts as significant.
pub const SIGNIFICANT_PEAK_FAP: f64 = 0.01;
/// Hard ceiling on frequency-grid size; exceeding it is an error, never a
/// silent truncation.
pub const MAX_FREQUENCY_GRID: usize = 1 << 20;
/// Hard ceiling on autocorrelation input length (pairwise O(n^2)).
pub const MAX_ACF_SAMPLES: usize = 8_192;

/// Frequency-grid and significance configuration for [`lomb_scargle_with_config`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodogramConfig {
    /// Oversampling factor `n_o`; grid spacing is `1 / (n_o * span)`.
    pub oversample: f64,
    /// Lowest frequency searched; defaults to `1 / span`.
    pub min_frequency: Option<f64>,
    /// Highest frequency searched; defaults to the pseudo-Nyquist
    /// `0.5 / median(dt)`.
    pub max_frequency: Option<f64>,
    /// Permutations for the FAP estimate (must be >= 1).
    pub fap_permutations: usize,
    /// Seed for the FAP permutation RNG.
    pub seed: u64,
    /// Maximum number of reported peaks.
    pub max_peaks: usize,
}

impl Default for PeriodogramConfig {
    fn default() -> Self {
        Self {
            oversample: DEFAULT_PERIODOGRAM_OVERSAMPLE,
            min_frequency: None,
            max_frequency: None,
            fap_permutations: DEFAULT_FAP_PERMUTATIONS,
            seed: DEFAULT_PERIODICITY_SEED,
            max_peaks: DEFAULT_MAX_PEAKS,
        }
    }
}

/// One periodogram local maximum with its permutation false-alarm probability.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodogramPeak {
    pub frequency: f64,
    pub period: f64,
    /// Normalised GLS power in `[0, 1]`.
    pub power: f64,
    /// Add-one permutation p-value of the max-power statistic.
    pub false_alarm_probability: f64,
}

/// Full periodogram readback: grid, powers, ranked peaks, trust.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodicityReport {
    pub frequencies: Vec<f64>,
    pub powers: Vec<f64>,
    /// Local maxima ranked by power (descending), at most `max_peaks`.
    pub peaks: Vec<PeriodogramPeak>,
    pub n_samples: usize,
    pub time_span: f64,
    pub trust: TrustTag,
}

impl PeriodicityReport {
    /// Highest-power peak, if any local maximum exists.
    pub fn dominant(&self) -> Option<&PeriodogramPeak> {
        self.peaks.first()
    }

    /// Peaks whose FAP is at or below `max_fap`.
    pub fn significant_peaks(&self, max_fap: f64) -> Vec<&PeriodogramPeak> {
        self.peaks
            .iter()
            .filter(|peak| peak.false_alarm_probability <= max_fap)
            .collect()
    }
}

/// Slotted autocorrelation readback for irregular samples.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutocorrelationReport {
    /// Populated slot lag centres (slots with zero pairs are omitted, never
    /// zero-filled).
    pub lags: Vec<f64>,
    /// Normalised autocorrelation per populated slot.
    pub coefficients: Vec<f64>,
    /// Pair count per populated slot.
    pub pair_counts: Vec<usize>,
    pub slot_width: f64,
    /// Lag of the highest positive local maximum, if one exists.
    pub dominant_period: Option<f64>,
    pub n_samples: usize,
    pub trust: TrustTag,
}

/// GLS periodogram with default configuration; trust is `Provisional`.
pub fn lomb_scargle(times: &[f64], values: &[f64]) -> Result<PeriodicityReport> {
    lomb_scargle_with_config(times, values, &PeriodogramConfig::default())
}

/// GLS periodogram with grounded-anchor trust discipline.
pub fn lomb_scargle_with_anchor(
    times: &[f64],
    values: &[f64],
    anchor: &Anchor,
) -> Result<PeriodicityReport> {
    let mut report = lomb_scargle_with_config(times, values, &PeriodogramConfig::default())?;
    report.trust = trust_for_anchor(Some(anchor));
    Ok(report)
}

/// GLS periodogram over an explicit configuration.
pub fn lomb_scargle_with_config(
    times: &[f64],
    values: &[f64],
    config: &PeriodogramConfig,
) -> Result<PeriodicityReport> {
    let stats = validate_series(times, values)?;
    let frequencies = frequency_grid(times, stats.span, config)?;
    let centered: Vec<f64> = values.iter().map(|value| value - stats.mean).collect();
    let powers = gls_powers(times, &centered, stats.variance, &frequencies);
    let mut peaks = ranked_peaks(&frequencies, &powers, config.max_peaks);
    assign_permutation_fap(
        times,
        &centered,
        stats.variance,
        &frequencies,
        &mut peaks,
        config,
    )?;
    Ok(PeriodicityReport {
        frequencies,
        powers,
        peaks,
        n_samples: times.len(),
        time_span: stats.span,
        trust: TrustTag::Provisional,
    })
}

/// Bins event timestamps into a uniform count series `(bin_centres, counts)`
/// so point-process recurrence streams can feed [`lomb_scargle`]. Zero-count
/// bins are real observations of "no events" and are included.
pub fn bin_event_counts(event_times: &[f64], bin_width: f64) -> Result<(Vec<f64>, Vec<f64>)> {
    if event_times.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "bin_event_counts requires at least one event time",
        ));
    }
    if !bin_width.is_finite() || bin_width <= 0.0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "bin_event_counts bin_width must be finite and positive, got {bin_width}"
        )));
    }
    for (index, pair) in event_times.windows(2).enumerate() {
        if !pair[0].is_finite() || pair[0] > pair[1] {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "event time at index {index} is non-finite or decreasing"
            )));
        }
    }
    let last = *event_times.last().expect("non-empty checked above");
    if !last.is_finite() {
        return Err(CalyxError::assay_insufficient_samples(
            "final event time is non-finite",
        ));
    }
    let first = event_times[0];
    let bin_count = ((last - first) / bin_width).floor() as usize + 1;
    if bin_count > MAX_FREQUENCY_GRID {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "bin_event_counts would produce {bin_count} bins (max {MAX_FREQUENCY_GRID}); \
             widen bin_width"
        )));
    }
    let mut counts = vec![0.0_f64; bin_count];
    for &event in event_times {
        let bin = (((event - first) / bin_width).floor() as usize).min(bin_count - 1);
        counts[bin] += 1.0;
    }
    let centres = (0..bin_count)
        .map(|bin| first + (bin as f64 + 0.5) * bin_width)
        .collect();
    Ok((centres, counts))
}

/// Slotted autocorrelation (Edelson & Krolik 1988 lineage) with default slot
/// width = median inter-sample spacing and max lag = half the span.
pub fn autocorrelation(times: &[f64], values: &[f64]) -> Result<AutocorrelationReport> {
    let stats = validate_series(times, values)?;
    if times.len() > MAX_ACF_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "autocorrelation input has {} samples (max {MAX_ACF_SAMPLES}); bin first",
            times.len()
        )));
    }
    let slot_width = median_spacing(times);
    let max_lag = stats.span / 2.0;
    let slot_count = (max_lag / slot_width).floor() as usize;
    if slot_count == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "autocorrelation span too short for a single lag slot",
        ));
    }
    let centered: Vec<f64> = values.iter().map(|value| value - stats.mean).collect();
    let mut sums = vec![0.0_f64; slot_count + 1];
    let mut counts = vec![0_usize; slot_count + 1];
    for i in 0..times.len() {
        for j in (i + 1)..times.len() {
            let lag = times[j] - times[i];
            if lag > max_lag {
                break;
            }
            let slot = (lag / slot_width).round() as usize;
            if slot >= 1 && slot <= slot_count {
                sums[slot] += centered[i] * centered[j];
                counts[slot] += 1;
            }
        }
    }
    let mut lags = Vec::new();
    let mut coefficients = Vec::new();
    let mut pair_counts = Vec::new();
    for slot in 1..=slot_count {
        if counts[slot] > 0 {
            lags.push(slot as f64 * slot_width);
            coefficients.push((sums[slot] / counts[slot] as f64) / stats.variance);
            pair_counts.push(counts[slot]);
        }
    }
    let dominant_period = positive_local_max(&lags, &coefficients);
    Ok(AutocorrelationReport {
        lags,
        coefficients,
        pair_counts,
        slot_width,
        dominant_period,
        n_samples: times.len(),
        trust: TrustTag::Provisional,
    })
}

struct SeriesStats {
    span: f64,
    mean: f64,
    /// Population variance (uniform weights), the GLS `YY` term.
    variance: f64,
}

fn validate_series(times: &[f64], values: &[f64]) -> Result<SeriesStats> {
    if times.len() != values.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "times has {} entries but values has {}",
            times.len(),
            values.len()
        )));
    }
    if times.len() < MIN_PERIODICITY_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "periodicity requires >= {MIN_PERIODICITY_SAMPLES} samples, got {}",
            times.len()
        )));
    }
    for (index, (&time, &value)) in times.iter().zip(values).enumerate() {
        if !time.is_finite() || !value.is_finite() {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "sample {index} contains NaN or infinity"
            )));
        }
    }
    for (index, pair) in times.windows(2).enumerate() {
        if pair[0] >= pair[1] {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "times must be strictly increasing; violation at index {index}"
            )));
        }
    }
    let span = times[times.len() - 1] - times[0];
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    if variance <= 0.0 {
        return Err(CalyxError::assay_low_signal(
            "values have zero variance; no periodic signal is measurable",
        ));
    }
    Ok(SeriesStats {
        span,
        mean,
        variance,
    })
}

/// Regular grid in frequency (never period): `f_min..=f_max` spaced
/// `1 / (oversample * span)` (VanderPlas 2018 §7.1).
fn frequency_grid(times: &[f64], span: f64, config: &PeriodogramConfig) -> Result<Vec<f64>> {
    if !config.oversample.is_finite() || config.oversample < 1.0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "periodogram oversample must be >= 1, got {}",
            config.oversample
        )));
    }
    let min_frequency = config.min_frequency.unwrap_or(1.0 / span);
    let max_frequency = config
        .max_frequency
        .unwrap_or_else(|| 0.5 / median_spacing(times));
    if !(min_frequency.is_finite() && max_frequency.is_finite())
        || min_frequency <= 0.0
        || max_frequency <= min_frequency
    {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "invalid frequency range [{min_frequency}, {max_frequency}]"
        )));
    }
    let step = 1.0 / (config.oversample * span);
    let grid_len = ((max_frequency - min_frequency) / step).floor() as usize + 1;
    if grid_len > MAX_FREQUENCY_GRID {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "frequency grid would have {grid_len} points (max {MAX_FREQUENCY_GRID}); \
             lower oversample or narrow the frequency range"
        )));
    }
    Ok((0..grid_len)
        .map(|index| min_frequency + index as f64 * step)
        .collect())
}

fn median_spacing(times: &[f64]) -> f64 {
    let mut gaps: Vec<f64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
    gaps.sort_by(f64::total_cmp);
    gaps[gaps.len() / 2]
}

/// Normalised GLS power per Zechmeister & Kürster 2009 eqs. 4-15 with uniform
/// weights: `p = (SS*YC^2 + CC*YS^2 - 2*CS*YC*YS) / (YY * (CC*SS - CS^2))`.
fn gls_powers(times: &[f64], centered: &[f64], variance: f64, frequencies: &[f64]) -> Vec<f64> {
    let weight = 1.0 / times.len() as f64;
    frequencies
        .iter()
        .map(|&frequency| {
            let omega = 2.0 * std::f64::consts::PI * frequency;
            let (mut c, mut s, mut cc_hat, mut cs_hat) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
            let (mut yc, mut ys) = (0.0_f64, 0.0_f64);
            for (&time, &value) in times.iter().zip(centered) {
                let (sin, cos) = (omega * time).sin_cos();
                c += cos;
                s += sin;
                cc_hat += cos * cos;
                cs_hat += cos * sin;
                yc += value * cos;
                ys += value * sin;
            }
            let (c, s) = (c * weight, s * weight);
            let cc = cc_hat * weight - c * c;
            let ss = (1.0 - cc_hat * weight) - s * s;
            let cs = cs_hat * weight - c * s;
            let (yc, ys) = (yc * weight, ys * weight);
            let determinant = cc * ss - cs * cs;
            if determinant.abs() < f64::EPSILON {
                return 0.0;
            }
            let power =
                (ss * yc * yc + cc * ys * ys - 2.0 * cs * yc * ys) / (variance * determinant);
            power.clamp(0.0, 1.0)
        })
        .collect()
}

/// Local maxima ranked by power descending, truncated to `max_peaks`.
fn ranked_peaks(frequencies: &[f64], powers: &[f64], max_peaks: usize) -> Vec<PeriodogramPeak> {
    let mut maxima: Vec<(f64, f64)> = Vec::new();
    for index in 1..powers.len().saturating_sub(1) {
        if powers[index] > powers[index - 1] && powers[index] >= powers[index + 1] {
            maxima.push((frequencies[index], powers[index]));
        }
    }
    maxima.sort_by(|a, b| f64::total_cmp(&b.1, &a.1));
    maxima.truncate(max_peaks);
    maxima
        .into_iter()
        .map(|(frequency, power)| PeriodogramPeak {
            frequency,
            period: 1.0 / frequency,
            power,
            false_alarm_probability: 1.0,
        })
        .collect()
}

/// Seeded permutation FAP: shuffle the values over the fixed times, take the
/// max power per permutation, and report the add-one p-value per peak.
fn assign_permutation_fap(
    times: &[f64],
    centered: &[f64],
    variance: f64,
    frequencies: &[f64],
    peaks: &mut [PeriodogramPeak],
    config: &PeriodogramConfig,
) -> Result<()> {
    if config.fap_permutations == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "fap_permutations must be >= 1; the FAP is mandatory, not optional",
        ));
    }
    if peaks.is_empty() {
        return Ok(());
    }
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut shuffled = centered.to_vec();
    let mut max_powers = Vec::with_capacity(config.fap_permutations);
    for _ in 0..config.fap_permutations {
        shuffled.shuffle(&mut rng);
        let max_power = gls_powers(times, &shuffled, variance, frequencies)
            .into_iter()
            .fold(0.0_f64, f64::max);
        max_powers.push(max_power);
    }
    for peak in peaks.iter_mut() {
        let exceed = max_powers
            .iter()
            .filter(|&&max_power| max_power >= peak.power)
            .count();
        peak.false_alarm_probability = (exceed + 1) as f64 / (config.fap_permutations + 1) as f64;
    }
    Ok(())
}

/// Dominant ACF period: the smallest-lag positive local maximum whose
/// coefficient is within [`ACF_PEAK_TOLERANCE`] of the strongest one.
///
/// For a periodic series every multiple of the true period is a local
/// maximum with the same expected coefficient, while long-lag slots have
/// fewer pairs and therefore noisier coefficients — a plain global max can
/// land on an arbitrary harmonic. Selecting the earliest near-maximal peak
/// recovers the fundamental (standard ACF period-detection practice, e.g.
/// McQuillan et al. 2013).
fn positive_local_max(lags: &[f64], coefficients: &[f64]) -> Option<f64> {
    const ACF_PEAK_TOLERANCE: f64 = 0.8;
    let mut peaks: Vec<(f64, f64)> = Vec::new();
    for index in 1..coefficients.len().saturating_sub(1) {
        let value = coefficients[index];
        if value > 0.0 && value > coefficients[index - 1] && value >= coefficients[index + 1] {
            peaks.push((lags[index], value));
        }
    }
    let strongest = peaks
        .iter()
        .map(|&(_, value)| value)
        .fold(f64::NEG_INFINITY, f64::max);
    peaks
        .into_iter()
        .find(|&(_, value)| value >= ACF_PEAK_TOLERANCE * strongest)
        .map(|(lag, _)| lag)
}
